# syntax=docker/dockerfile:1
FROM rust:1.94-slim-bookworm AS builder

WORKDIR /build

# 先只帶 manifest 建一次假的空專案，讓依賴編譯結果留在獨立的層。
# 之後只改業務程式碼時，這一層可直接命中快取——Rust 依賴編譯很慢，值得這道手續。
COPY Cargo.toml Cargo.lock ./
COPY crates/domain/Cargo.toml crates/domain/
COPY crates/app/Cargo.toml crates/app/
RUN mkdir -p crates/domain/src crates/app/src \
 && touch crates/domain/src/lib.rs \
 && echo 'fn main() {}' > crates/app/src/main.rs \
 && cargo build --release -p acms-app \
 && rm -rf crates

# 真正的原始碼。migrations 由 sqlx::migrate! 在編譯期嵌入 binary，
# 因此它們必須在這一步就位。
COPY crates crates
RUN touch crates/domain/src/lib.rs crates/app/src/main.rs \
 && cargo build --release -p acms-app


FROM debian:bookworm-slim

# rustls 不需要系統 OpenSSL，只需要根憑證
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --system --create-home --uid 10001 acms

COPY --from=builder /build/target/release/acms-app /usr/local/bin/acms-app

USER acms
ENV PORT=8080
EXPOSE 8080

# 健康檢查由 binary 自己處理（acms-app healthcheck），映像不需要 curl
CMD ["acms-app"]
