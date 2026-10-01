//! 認證機制：密碼雜湊與 session。
//!
//! 這裡只放機制，不放政策。「誰可以核准報價」「誰看得到成本」屬於業務規則，
//! 住在 `acms_domain::authz`。
//!
//! Session 存於 Postgres 而非記憶體：重啟服務不會把所有人登出，
//! 且 session 可即時撤銷——這是選 cookie session 而非 JWT 的主要理由。

use argon2::password_hash::rand_core::{OsRng, RngCore};
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use sqlx::PgExecutor;
use time::{Duration, OffsetDateTime};

use acms_domain::UserId;

/// Session 有效期。G1 的使用情境為平日工作時間，8 小時涵蓋一個工作天。
pub const SESSION_TTL: Duration = Duration::hours(8);

/// Cookie 名稱。
pub const COOKIE_NAME: &str = "acms_session";

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("帳號或密碼錯誤")]
    InvalidCredentials,

    #[error("此帳號已停用")]
    Inactive,

    #[error("尚未登入或登入已逾期")]
    NoSession,

    #[error("密碼處理失敗")]
    Hashing,

    #[error("資料庫錯誤")]
    Database(#[from] sqlx::Error),
}

/// 以 argon2id 雜湊密碼。
///
/// 不用 SHA 系列：它們為快速雜湊而設計，正是密碼雜湊最不該有的性質。
pub fn hash_password(password: &str) -> Result<String, AuthError> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|_| AuthError::Hashing)
}

/// 驗證密碼。
///
/// 比對由 argon2 內部以常數時間進行，此處不另外處理時序。
pub fn verify_password(password: &str, hash: &str) -> bool {
    PasswordHash::new(hash)
        .map(|parsed| {
            Argon2::default()
                .verify_password(password.as_bytes(), &parsed)
                .is_ok()
        })
        .unwrap_or(false)
}

/// `hash_password` 的 async 包裝，在 blocking 執行緒池上執行。
///
/// argon2 是刻意設計成慢且吃記憶體的（預設 m=19 MiB、t=2），
/// 直接在 tokio worker 上跑會把整個 runtime 的一條 worker 阻塞數十毫秒。
/// `/auth/login` 是唯一不需認證的端點，若雜湊在 worker 上執行，
/// 併發打它就能讓所有 worker 排隊、健康檢查逾時、容器被重啟。
pub async fn hash_password_blocking(password: String) -> Result<String, AuthError> {
    tokio::task::spawn_blocking(move || hash_password(&password))
        .await
        .map_err(|_| AuthError::Hashing)?
}

/// `verify_password` 的 async 包裝。理由同 [`hash_password_blocking`]。
///
/// 執行緒 panic 時回 false——驗證失敗是這裡唯一安全的預設。
async fn verify_password_blocking(password: String, hash: String) -> bool {
    tokio::task::spawn_blocking(move || verify_password(&password, &hash))
        .await
        .unwrap_or(false)
}

/// 產生 session token：32 位元組的密碼學亂數。
///
/// 不用流水號或時間戳——那些可被猜測。
fn generate_token() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    hex(&bytes)
}

fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut acc, b| {
            use std::fmt::Write;
            let _ = write!(acc, "{b:02x}");
            acc
        })
}

/// `sessions.id` 存的是 token 的 SHA-256 摘要，不是 token 本身。
///
/// 原本直接存 token，那一欄就等於 bearer 憑證：任何能 `SELECT * FROM sessions`
/// 的人（唯讀帳號、每日 `pg_dump` 留在 volume 上的 14 份備份、拿到備份檔的人）
/// 都能把值貼進 cookie 冒充任何人，而稽核會記成受害者做的。存摘要之後
/// 讀到資料庫不等於拿到憑證。
///
/// 這裡用單次 SHA-256 而非 argon2：token 已是 256 bit 的 `OsRng` 亂數，
/// 沒有字典可暴力破解，需要的只是單向與抗碰撞；每個請求都要查 session，
/// 慢雜湊在這條路上是純成本。
fn token_digest(token: &str) -> String {
    use sha2::{Digest, Sha256};
    hex(&Sha256::digest(token.as_bytes()))
}

/// 驗證帳密並建立 session，回傳 token。
pub async fn login(
    ex: impl PgExecutor<'_> + Copy,
    username: &str,
    password: &str,
) -> Result<String, AuthError> {
    let credentials = crate::db::user::credentials_for(ex, username).await?;

    // 帳號不存在時仍執行一次雜湊驗證，讓回應時間不因帳號是否存在而不同。
    let Some(credentials) = credentials else {
        let _ = verify_password_blocking(password.to_owned(), DUMMY_HASH.to_owned()).await;
        return Err(AuthError::InvalidCredentials);
    };

    if !verify_password_blocking(password.to_owned(), credentials.password_hash).await {
        return Err(AuthError::InvalidCredentials);
    }

    if !credentials.is_active {
        return Err(AuthError::Inactive);
    }

    let token = generate_token();
    sqlx::query!(
        "INSERT INTO sessions (id, user_id, expires_at) VALUES ($1, $2, $3)",
        token_digest(&token),
        credentials.user_id.0,
        OffsetDateTime::now_utc() + SESSION_TTL,
    )
    .execute(ex)
    .await?;

    Ok(token)
}

/// 一組固定的 argon2 雜湊，供帳號不存在時做等時比對之用。
/// 值本身不重要，只需格式合法。
const DUMMY_HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHRzYWx0c2E$\
                          O1VtAr4mCTMFZFkYMTBYmbKKn5vGBQxG3Gjgy2nPKEc";

/// 由 token 取出使用者，順帶清掉過期的 session。
pub async fn resolve(ex: impl PgExecutor<'_> + Copy, token: &str) -> Result<UserId, AuthError> {
    let key = token_digest(token);
    let row = sqlx::query!(
        "SELECT user_id, expires_at FROM sessions WHERE id = $1",
        key,
    )
    .fetch_optional(ex)
    .await?;

    let Some(row) = row else {
        return Err(AuthError::NoSession);
    };

    if row.expires_at <= OffsetDateTime::now_utc() {
        // 過期即刪除，不留給後續請求再判斷一次
        let _ = sqlx::query!("DELETE FROM sessions WHERE id = $1", key)
            .execute(ex)
            .await;
        return Err(AuthError::NoSession);
    }

    Ok(UserId(row.user_id))
}

pub async fn logout(ex: impl PgExecutor<'_>, token: &str) -> Result<(), AuthError> {
    sqlx::query!("DELETE FROM sessions WHERE id = $1", token_digest(token))
        .execute(ex)
        .await?;
    Ok(())
}

/// 清除已過期的 session。由排程或啟動時呼叫。
pub async fn purge_expired(ex: impl PgExecutor<'_>) -> Result<u64, AuthError> {
    Ok(
        sqlx::query!("DELETE FROM sessions WHERE expires_at <= now()")
            .execute(ex)
            .await?
            .rows_affected(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_round_trips_and_rejects_wrong_input() {
        let hash = hash_password("正確密碼2026").unwrap();
        assert!(verify_password("正確密碼2026", &hash));
        assert!(!verify_password("錯誤密碼", &hash));
        assert!(!verify_password("", &hash));
    }

    /// 相同密碼每次雜湊的結果必須不同，否則等於沒加鹽。
    #[test]
    fn identical_passwords_hash_differently() {
        let a = hash_password("same").unwrap();
        let b = hash_password("same").unwrap();
        assert_ne!(a, b);
        assert!(verify_password("same", &a));
        assert!(verify_password("same", &b));
    }

    #[test]
    fn malformed_hash_is_rejected_rather_than_panicking() {
        assert!(!verify_password("x", "not-a-hash"));
        assert!(!verify_password("x", ""));
    }

    /// 用於等時比對的假雜湊必須格式合法，否則那次比對會提早失敗而失去效果。
    #[test]
    fn dummy_hash_is_well_formed() {
        assert!(PasswordHash::new(DUMMY_HASH).is_ok());
        assert!(!verify_password("anything", DUMMY_HASH));
    }

    #[test]
    fn tokens_are_long_and_unique() {
        let a = generate_token();
        let b = generate_token();
        assert_eq!(a.len(), 64);
        assert_ne!(a, b);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
