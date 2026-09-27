use redis::aio::MultiplexedConnection;
use redis::Script;
use std::sync::Arc;

/// Redis-backed token bucket replacement for the old in-memory Map.
/// An in-memory rate limiter only works correctly on a single instance —
/// moved to Redis so the limit holds even with multiple app instances behind a load balancer.
pub struct RateLimiter {
    script: Script,
    max_requests: u32,
    window_seconds: u64,
}

impl RateLimiter {
    pub fn new(max_requests: u32, window_ms: u64) -> Arc<Self> {
        // Lua makes the INCR + EXPIRE atomic. Without that, two concurrent requests
        // from the same IP could both read the old count and both sneak through.
        let script = Script::new(
            r#"
            local key = KEYS[1]
            local window = tonumber(ARGV[1])
            local current = redis.call('INCR', key)
            if current == 1 then
                redis.call('EXPIRE', key, window)
            end
            return current
            "#,
        );

        Arc::new(Self {
            script,
            max_requests,
            // Redis EXPIRE takes seconds, so round up the configured window.
            window_seconds: std::cmp::max(1, window_ms.div_ceil(1000)),
        })
    }

    /// Returns true if the request is allowed, false if it should be rejected.
    pub async fn allow(&self, redis: &mut MultiplexedConnection, ip: &str) -> bool {
        let key = format!("ratelimit:{}", ip);

        match self
            .script
            .key(&key)
            .arg(self.window_seconds)
            .invoke_async::<_, u64>(redis)
            .await
        {
            Ok(count) => count <= self.max_requests as u64,
            Err(e) => {
                // Fail open: if Redis is unreachable we would rather serve the request
                // than reject every visitor because the limiter broke.
                tracing::error!("Rate limiter could not reach Redis, allowing request: {}", e);
                true
            }
        }
    }
}
