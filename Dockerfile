# syntax=docker/dockerfile:1

FROM rust:1-bookworm AS builder
WORKDIR /app

# Копируем файлы конфигурации и исходный код для сборки
COPY Cargo.toml Cargo.lock ./
COPY src ./src

RUN cargo build --release

FROM debian:bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --create-home --home-dir /home/wol-server --shell /usr/sbin/nologin wol-server

WORKDIR /home/wol-server

# Копируем скомпилированный бинарник из этапа сборки
COPY --from=builder /app/target/release/wol_server /usr/local/bin/wol_server

USER wol-server

# Сервер работает в network_mode: host, но декларативно указываем порт
EXPOSE 8084

ENTRYPOINT ["wol_server"]
