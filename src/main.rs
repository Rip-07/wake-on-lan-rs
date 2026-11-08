use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use sea_orm::{ConnectionTrait, Database, DatabaseConnection, EntityTrait};
use serde::{Deserialize, Serialize};
use std::net::{Ipv4Addr, SocketAddr};
use std::str::FromStr;
use tokio::net::UdpSocket;

mod computer {
    use super::*;
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
    #[sea_orm(table_name = "computers")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub id: i32,
        pub name: String,
        pub mac_address: String,
        pub broadcast_address: String,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

#[derive(Debug, PartialEq)]
pub struct MacAddress(pub [u8; 6]);

#[derive(Debug, PartialEq, Eq)]
pub struct ParseMacError;

impl FromStr for MacAddress {
    type Err = ParseMacError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = if s.contains(':') {
            s.split(':').collect()
        } else if s.contains('-') {
            s.split('-').collect()
        } else {
            return Err(ParseMacError);
        };

        if parts.len() != 6 {
            return Err(ParseMacError);
        }

        let mut bytes = [0u8; 6];
        for (i, part) in parts.iter().enumerate() {
            if part.len() != 2 || !part.is_ascii() {
                return Err(ParseMacError);
            }
            bytes[i] = u8::from_str_radix(part, 16).map_err(|_| ParseMacError)?;
        }

        Ok(MacAddress(bytes))
    }
}

pub fn build_magic_packet(mac: &MacAddress) -> [u8; 102] {
    let mut packet = [0u8; 102];
    packet[0..6].copy_from_slice(&[0xFF; 6]);
    for i in 1..17 {
        packet[i * 6..i * 6 + 6].copy_from_slice(&mac.0);
    }
    packet
}

#[derive(Clone)]
struct AppState {
    db: DatabaseConnection,
    api_key: String,
}

#[derive(Serialize)]
struct WakeSuccessResponse {
    message: &'static str,
    computer_name: String,
}

#[derive(Serialize)]
struct ErrorResponse {
    error: String,
}

type ApiError = (StatusCode, Json<ErrorResponse>);

fn api_error(status: StatusCode, error: impl Into<String>) -> ApiError {
    (
        status,
        Json(ErrorResponse {
            error: error.into(),
        }),
    )
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let database_url = std::env::var("DATABASE_URL")
        .expect("КРИТИЧЕСКАЯ ОШИБКА: Переменная окружения DATABASE_URL не задана!");

    let api_key = std::env::var("API_KEY")
        .expect("КРИТИЧЕСКАЯ ОШИБКА: Переменная окружения API_KEY не задана!");

    let port = std::env::var("PORT").unwrap_or_else(|_| "8084".to_string());

    let db = Database::connect(&database_url)
        .await
        .expect("Не удалось подключиться к базе данных");

    let db_backend = db.get_database_backend();
    let schema = sea_orm::Schema::new(db_backend);
    let mut create_table_statement = schema.create_table_from_entity(computer::Entity);

    db.execute(db_backend.build(create_table_statement.if_not_exists()))
        .await
        .expect("Не удалось создать таблицу в базе данных");

    let state = AppState { db, api_key };

    // Публичный /health доступен без X-Api-key для проверки работоспособности CI/CD
    let app = Router::new()
        .route("/health", get(health_check))
        .merge(
            Router::new()
                .route("/computers", get(get_computers))
                .route("/computers/:id/wake", post(wake_computer))
                .layer(axum::middleware::from_fn_with_state(
                    state.clone(),
                    auth_middleware,
                )),
        )
        .with_state(state);

    let addr: SocketAddr = format!("0.0.0.0:{}", port).parse().unwrap();
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .unwrap_or_else(|error| panic!("Не удалось запустить WOL сервер на {addr}: {error}"));
    println!("WOL Сервер запущен на {}", addr);
    axum::serve(listener, app).await.unwrap();
}

async fn health_check() -> StatusCode {
    StatusCode::OK
}

async fn auth_middleware(
    State(state): State<AppState>,
    headers: HeaderMap,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Result<axum::response::Response, StatusCode> {
    let key_valid = headers
        .get("X-Api-key")
        .and_then(|k| k.to_str().ok())
        .is_some_and(|k| constant_time_eq(k.as_bytes(), state.api_key.as_bytes()));

    if key_valid {
        Ok(next.run(request).await)
    } else {
        Err(StatusCode::UNAUTHORIZED)
    }
}

/// Сравнение без раннего выхода, чтобы время ответа не выдавало совпадающий префикс ключа.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

// Исправление: Безопасная обработка ошибки БД через map_err вместо unwrap()
async fn get_computers(
    State(state): State<AppState>,
) -> Result<Json<Vec<computer::Model>>, StatusCode> {
    let computers = computer::Entity::find()
        .all(&state.db)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(computers))
}

async fn wake_computer(
    Path(id): Path<i32>,
    State(state): State<AppState>,
) -> Result<(StatusCode, Json<WakeSuccessResponse>), ApiError> {
    let comp = computer::Entity::find_by_id(id)
        .one(&state.db)
        .await
        .map_err(|_| {
            api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to query the computer database",
            )
        })?
        .ok_or_else(|| {
            api_error(
                StatusCode::NOT_FOUND,
                format!("Computer with id {id} not found"),
            )
        })?;

    let mac = MacAddress::from_str(&comp.mac_address).map_err(|_| {
        api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("Computer '{}' has an invalid MAC address", comp.name),
        )
    })?;
    let broadcast_ip = Ipv4Addr::from_str(&comp.broadcast_address).map_err(|_| {
        api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("Computer '{}' has an invalid broadcast address", comp.name),
        )
    })?;
    let target_addr = SocketAddr::new(std::net::IpAddr::V4(broadcast_ip), 9);

    let packet = build_magic_packet(&mac);

    let socket = UdpSocket::bind("0.0.0.0:0").await.map_err(|_| {
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to create a UDP socket",
        )
    })?;
    socket.set_broadcast(true).map_err(|_| {
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to enable UDP broadcast",
        )
    })?;
    socket.send_to(&packet, target_addr).await.map_err(|_| {
        api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            format!("Failed to send the WOL packet to '{}'", comp.name),
        )
    })?;

    Ok((
        StatusCode::ACCEPTED,
        Json(WakeSuccessResponse {
            message: "WOL packet sent successfully",
            computer_name: comp.name,
        }),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_mac_parsing() {
        assert_eq!(
            MacAddress::from_str("AA:BB:CC:DD:EE:FF"),
            Ok(MacAddress([0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF]))
        );
        assert_eq!(
            MacAddress::from_str("12-34-56-78-90-AB"),
            Ok(MacAddress([0x12, 0x34, 0x56, 0x78, 0x90, 0xAB]))
        );
    }

    #[test]
    fn test_invalid_mac_parsing() {
        assert!(MacAddress::from_str("AA:BB:CC:DD:EE").is_err());
        assert!(MacAddress::from_str("AA:BB:CC:DD:EE:FF:GG").is_err());
        assert!(MacAddress::from_str("Zz:BB:CC:DD:EE:FF").is_err());
        assert!(MacAddress::from_str("AA:B:CC:DD:EE:FF").is_err());
        assert!(MacAddress::from_str("Привет:мир:1:2:3:4").is_err());
    }

    #[test]
    fn test_constant_time_eq() {
        assert!(constant_time_eq(b"secret", b"secret"));
        assert!(!constant_time_eq(b"secret", b"secreT"));
        assert!(!constant_time_eq(b"secret", b"secret2"));
    }

    #[test]
    fn test_magic_packet_builder() {
        let raw_mac = [0x01, 0x02, 0x03, 0x04, 0x05, 0x06];
        let mac = MacAddress(raw_mac);
        let packet = build_magic_packet(&mac);

        // Проверяем 6 байт синхронизации (0xFF)
        assert_eq!(&packet[0..6], &[0xFF; 6]);

        let mac_payload = &packet[6..102];
        let (chunks, remainder) = mac_payload.as_chunks::<6>();

        for chunk in chunks {
            assert_eq!(chunk, &raw_mac);
        }
        assert!(remainder.is_empty());
    }
}
