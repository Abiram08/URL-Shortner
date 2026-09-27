use chrono::{DateTime, NaiveDateTime, Utc};
use redis::aio::MultiplexedConnection;
use redis::AsyncCommands;
use sqlx::{PgPool, Row};

// Axum handlers run on a multi-threaded runtime, so any error held across an
// await point must be Send + Sync. This alias keeps the function signatures tidy.
type ShortyError = Box<dyn std::error::Error + Send + Sync>;

const BASE62_ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";

/// Convert a numeric database id into a short, URL-safe code.
/// 0 -> "a", 1 -> "b", 61 -> "9", 62 -> "ba", and so on.
pub fn encode_to_base62_with_salt(number: i64, salt: u64) -> String {
    encode_to_base62((number as u64 ^ salt) as i64)
}

pub fn encode_to_base62(mut number: i64) -> String {
    if number == 0 {
        return "a".to_string();
    }

    let mut digits = Vec::new();
    while number > 0 {
        let remainder = (number % 62) as usize;
        digits.push(BASE62_ALPHABET[remainder] as char);
        number /= 62;
    }

    digits.reverse();
    digits.into_iter().collect()
}

/// Reject obviously invalid URLs early.
/// Same rule the frontend uses: a real HTTP(S) URL with a proper host.
/// localhost is allowed even though it has no dot.
pub fn is_valid_url(url: &str) -> bool {
    match url::Url::parse(url) {
        Ok(parsed) => {
            let scheme = parsed.scheme();
            match parsed.host_str() {
                Some(host) => {
                    (scheme == "http" || scheme == "https")
                        && (host == "localhost" || host.contains('.'))
                }
                None => false,
            }
        }
        Err(_) => false,
    }
}

/// Save a URL, but if the exact long_url already exists return the old short code.
/// This keeps Shorty idempotent: retrying a failed request will not create duplicates.
pub async fn save_url(
    pool: &PgPool,
    redis: &mut MultiplexedConnection,
    _base_url: &str,
    long_url: &str,
    ttl: u64,
) -> Result<String, ShortyError> {
    // Guard against huge payloads (DB bloat / Redis memory).
    if long_url.len() > 2048 {
        return Err("Invalid URL: too long (max 2048 characters)".into());
    }
    if !is_valid_url(long_url) {
        return Err("Invalid URL: must start with http:// or https:// and contain a host".into());
    }

    // 1. Fast idempotency check: same long_url should always yield the same short code.
    let existing = sqlx::query("SELECT short_code FROM urls WHERE long_url = $1")
        .bind(long_url)
        .fetch_optional(pool)
        .await?;

    if let Some(row) = existing {
        let code: String = row.try_get("short_code")?;
        return Ok(code);
    }

    // 2. Reserve the next auto-increment id and encode it.
    let id_row = sqlx::query("SELECT nextval('urls_id_seq') AS id")
        .fetch_one(pool)
        .await?;
    let id: i64 = id_row.try_get("id")?;
    // Salt is passed via _base_url hack: caller should use encode_to_base62_with_salt.
    // For now use plain encode; main.rs will switch to salted variant.
    let short_code = encode_to_base62(id);

    // 3. Store the mapping. Race: two concurrent inserts for the same long_url
    // both miss the check above. The UNIQUE(idx_urls_long_url) will reject the
    // second. Catch that and return the winner's code instead of leaking a
    // Postgres duplicate-key error to the user.
    let insert = sqlx::query("INSERT INTO urls (id, short_code, long_url) VALUES ($1, $2, $3)")
        .bind(id)
        .bind(&short_code)
        .bind(long_url)
        .execute(pool)
        .await;

    if let Err(e) = insert {
        if let Some(db_err) = e.as_database_error() {
            if db_err.code().as_deref() == Some("23505") {
                // Unique violation — fetch the code that won the race.
                if let Some(row) =
                    sqlx::query("SELECT short_code FROM urls WHERE long_url = $1")
                        .bind(long_url)
                        .fetch_optional(pool)
                        .await?
                {
                    let code: String = row.try_get("short_code")?;
                    // Warm cache for the race winner as well.
                    let _ = redis.set_ex::<_, _, ()>(&code, long_url, ttl).await;
                    return Ok(code);
                }
            }
        }
        return Err(e.into());
    }

    // 4. Warm the cache so the first redirect is fast.
    if let Err(e) = redis.set_ex::<_, _, ()>(&short_code, long_url, ttl).await {
        tracing::error!("Could not save to Redis cache: {}", e);
    }

    Ok(short_code)
}

/// Look up a short code using cache-aside: Redis first, then PostgreSQL.
/// If PostgreSQL has it, backfill Redis for the next request.
pub async fn get_long_url(
    pool: &PgPool,
    redis: &mut MultiplexedConnection,
    short_code: &str,
    ttl: u64,
) -> Result<Option<String>, ShortyError> {
    if !is_valid_short_code(short_code) {
        return Ok(None);
    }
    match redis.get::<_, Option<String>>(short_code).await {
        Ok(Some(url)) => return Ok(Some(url)),
        Ok(None) => {}
        Err(e) => {
            tracing::error!("Something went wrong while checking Redis: {}", e);
        }
    }

    let row = sqlx::query("SELECT long_url FROM urls WHERE short_code = $1")
        .bind(short_code)
        .fetch_optional(pool)
        .await?;

    if let Some(row) = row {
        let long_url: String = row.try_get("long_url")?;

        if let Err(e) = redis.set_ex::<_, _, ()>(short_code, &long_url, ttl).await {
            tracing::error!("Could not save to Redis cache: {}", e);
        }

        Ok(Some(long_url))
    } else {
        Ok(None)
    }
}

pub struct UrlStats {
    pub short_code: String,
    pub long_url: String,
    pub click_count: i32,
    pub created_at: DateTime<Utc>,
}

pub fn is_valid_short_code(code: &str) -> bool {
    // Cheap guard before hitting Redis/Postgres. Prevents huge keys & injection.
    !code.is_empty()
        && code.len() <= 10
        && code.chars().all(|c| c.is_ascii_alphanumeric())
}

/// Fetch public statistics for a short code.
pub async fn get_url_stats(
    pool: &PgPool,
    short_code: &str,
) -> Result<Option<UrlStats>, ShortyError> {
    if !is_valid_short_code(short_code) {
        return Ok(None);
    }
    let row = sqlx::query(
        "SELECT short_code, long_url, click_count, created_at FROM urls WHERE short_code = $1"
    )
    .bind(short_code)
    .fetch_optional(pool)
    .await?;

    if let Some(row) = row {
        let naive: NaiveDateTime = row.try_get("created_at")?;
        Ok(Some(UrlStats {
            short_code: row.try_get("short_code")?,
            long_url: row.try_get("long_url")?,
            click_count: row.try_get("click_count")?,
            created_at: naive.and_utc(),
        }))
    } else {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base62_encodes_zero() {
        assert_eq!(encode_to_base62(0), "a");
    }

    #[test]
    fn base62_encodes_one() {
        assert_eq!(encode_to_base62(1), "b");
    }

    #[test]
    fn base62_encodes_large_number() {
        assert_eq!(encode_to_base62(999_999), "emjb");
    }

    #[test]
    fn base62_encodes_round_trip_examples() {
        assert_eq!(encode_to_base62(61), "9");
        assert_eq!(encode_to_base62(62), "ba");
        assert_eq!(encode_to_base62(63), "bb");
    }

    #[test]
    fn valid_urls_accepted() {
        assert!(is_valid_url("https://example.com"));
        assert!(is_valid_url("http://localhost:3000"));
    }

    #[test]
    fn invalid_urls_rejected() {
        assert!(!is_valid_url("ftp://example.com"));
        assert!(!is_valid_url("not-a-url"));
        assert!(!is_valid_url("https://nodot"));
    }
}
