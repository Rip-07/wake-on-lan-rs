# Wake-on-LAN (WOL) Server

Fast async Rust microservice (Axum + SeaORM + PostgreSQL) that sends Wake-on-LAN magic packets to machines in your LAN via a small API-key protected HTTP API.

Высокопроизводительный асинхронный микросервис на Rust (Axum + SeaORM) для отправки Magic Packet на устройства внутри локальной сети.

## Особенности архитектуры
- **Язык**: Rust (Edition 2024, компиляция с LTO-оптимизациями)
- **База данных**: PostgreSQL
- **Сетевой режим**: `network_mode: host` (необходим для работы UDP-broadcasting на Unraid)

## Конфигурация (Environment Variables)
Для работы контейнера требуются следующие переменные:
- `API_KEY`: Секретный токен авторизации (передается в заголовке `X-Api-key`).
- `DATABASE_URL`: Строка подключения к PostgreSQL (`postgres://user:pass@host:5432/db`).
- `PORT`: Внутренний порт приложения.

## Наполнение Базы Данных (Seed)
Поскольку автоматический сидинг тестовых данных отключен, выполните следующий SQL-запрос в вашей PostgreSQL для регистрации первого компьютера:

```sql
INSERT INTO computers (name, mac_address, broadcast_address)
VALUES ('Рабочий ПК', 'AA:BB:CC:DD:EE:FF', '192.168.1.255');
```

## Спецификация API

### 1. Проверка контейнера (публичный)
```bash
GET /health
```
Возвращает статус `200 OK`. Используется системами мониторинга или CI/CD для проверки успешности деплоя.

### 2. Получение списка компьютеров
```bash
GET /computers
Header: X-Api-key: <ваш_ключ>
```

### 3. Пробуждение компьютера по ID
```bash
POST /computers/:id/wake
Header: X-Api-key: <ваш_ключ>
```

## Быстрый старт
```bash
git clone https://github.com/Rip-07/wake-on-lan-rs.git
cd wake-on-lan-rs
export API_KEY=$(openssl rand -hex 32)
export DATABASE_URL=postgres://wol:password@localhost:5432/wol_db
docker compose up -d --build
```

## Безопасность
- Используйте длинный случайный `API_KEY` (например, `openssl rand -hex 32`).
- Не публикуйте сервис в интернет напрямую: держите его в LAN/VPN или за reverse proxy с HTTPS.

## Тесты

```bash
cargo check --locked
cargo test --locked
```

## Лицензия

MIT
