# syntax=docker/dockerfile:1

# ---------- 前端 ----------
# 型別由後端的 OpenAPI 規格產生，因此 schema.d.ts 已在版控中。
# 這一層只負責編譯，不重新產生型別——那需要跑後端 binary。
FROM node:24-slim AS web

WORKDIR /web
COPY web/package.json web/package-lock.json ./
RUN npm ci
COPY web/ ./
RUN npm run build


# ---------- 後端 ----------
FROM rust:1.94-slim-bookworm AS builder

WORKDIR /build

# sqlx 的 query! 巨集在編譯期驗證 SQL。建置容器時沒有資料庫可連，
# 因此改讀版控中的 .sqlx 離線快取。快取與 schema 不同步時建置會失敗，
# 這正是要的效果——它逼著 migration 與查詢一起更新。
ENV SQLX_OFFLINE=true

# 單執行緒編譯。typst-library 在 opt-level 3 下的峰值記憶體很高，
# 平行編譯會在記憶體有限的建置機（例如預設 2GiB 的 podman machine）
# 被 OOM killer 殺掉，錯誤訊息只有 signal: 9，不容易看出原因。
# 代價是建置變慢，但依賴層有快取，只有第一次與改動相依時才付這個成本。
ENV CARGO_BUILD_JOBS=1

# 先只帶 manifest 建一次假的空專案，讓依賴編譯結果留在獨立的層。
# 之後只改業務程式碼時，這一層可直接命中快取——Rust 依賴編譯很慢，值得這道手續。
COPY Cargo.toml Cargo.lock ./
COPY .sqlx .sqlx
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
# rustls 不需要系統 OpenSSL，只需要根憑證。
# fonts-noto-cjk 供報價單 PDF 排版中文——沒有它 typst 只會印出豆腐方塊，
# 所以 PDF 產生器在找不到中文字型時會明確失敗而非靜默產出廢紙。
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates fonts-noto-cjk \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --system --create-home --uid 10001 acms

COPY --from=builder /build/target/release/acms-app /usr/local/bin/acms-app
COPY --from=web /web/dist /srv/web

USER acms
ENV STATIC_DIR=/srv/web
ENV FONT_DIR=/usr/share/fonts
ENV PORT=8080
EXPOSE 8080

# 健康檢查由 binary 自己處理（acms-app healthcheck），映像不需要 curl
CMD ["acms-app"]
