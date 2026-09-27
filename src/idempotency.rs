use redis::aio::MultiplexedConnection;
use redis::AsyncCommands;

/// Redis-backed cache for Idempotency-Key results.
///
/// If a client retries a failed request with the same key, we can return the
/// cached short URL instead of creating a duplicate row.

fn full_key(key: &str) -> String {
    format!("idempotency:{}", key)
}

/// Look up a previously stored response by idempotency key.
pub async fn lookup(redis: &mut MultiplexedConnection, key: &str) -> Option<String> {
    redis
        .get::<_, Option<String>>(&full_key(key))
        .await
        .ok()
        .flatten()
}

/// Store a response JSON so retries can be served without re-processing.
pub async fn store(redis: &mut MultiplexedConnection, key: &str, json: &str, ttl_seconds: u64) {
    if let Err(e) = redis
        .set_ex::<_, _, ()>(&full_key(key), json, ttl_seconds)
        .await
    {
        tracing::error!("Failed to cache idempotency key: {}", e);
    }
}
