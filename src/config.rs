use std::env;

#[derive(Clone)]
pub struct Config {
    pub port: u16,
    pub base_url: String,
    pub database_url: String,
    pub redis_url: String,
    pub cache_ttl_seconds: u64,
    pub rate_limit_max_requests: u32,
    pub rate_limit_window_ms: u64,
    pub analytics_batch_size: i64,
    pub analytics_flush_interval_seconds: u64,
    pub idempotency_key_ttl_seconds: u64,
    pub database_max_connections: u32,
    pub database_acquire_timeout_seconds: u64,
    pub trusted_proxy_count: usize,
    pub short_code_salt: u64,
    pub redis_pool_max_size: usize,
}

impl Config {
    /// Load settings from environment variables.
    /// .env is optional in production, but it makes local development easier.
    pub fn from_env() -> Self {
        let _ = dotenvy::dotenv();

        let port = env::var("PORT")
            .unwrap_or_else(|_| "3000".to_string())
            .parse()
            .expect("PORT must be a number");

        let mut base_url = env::var("BASE_URL")
            .unwrap_or_else(|_| "http://localhost:3000".to_string());
        // Normalize: no trailing slash, so format!("{}/{}", base_url, code) is clean.
        base_url = base_url.trim_end_matches('/').to_string();
        if url::Url::parse(&base_url).is_err() {
            panic!("BASE_URL must be a valid URL, got: {}", base_url);
        }

        let database_url = env::var("DATABASE_URL")
            .expect("DATABASE_URL must be set");

        let redis_url = env::var("REDIS_URL")
            .expect("REDIS_URL must be set");

        let cache_ttl_seconds = env::var("CACHE_TTL_SECONDS")
            .unwrap_or_else(|_| "86400".to_string())
            .parse()
            .expect("CACHE_TTL_SECONDS must be a number");

        let rate_limit_max_requests = env::var("RATE_LIMIT_MAX_REQUESTS")
            .unwrap_or_else(|_| "10".to_string())
            .parse()
            .expect("RATE_LIMIT_MAX_REQUESTS must be a number");

        let rate_limit_window_ms = env::var("RATE_LIMIT_WINDOW_MS")
            .unwrap_or_else(|_| "60000".to_string())
            .parse()
            .expect("RATE_LIMIT_WINDOW_MS must be a number");

        let analytics_batch_size = env::var("ANALYTICS_BATCH_SIZE")
            .unwrap_or_else(|_| "1000".to_string())
            .parse()
            .expect("ANALYTICS_BATCH_SIZE must be a number");

        let analytics_flush_interval_seconds = env::var("ANALYTICS_FLUSH_INTERVAL_SECONDS")
            .unwrap_or_else(|_| "5".to_string())
            .parse()
            .expect("ANALYTICS_FLUSH_INTERVAL_SECONDS must be a number");

        let idempotency_key_ttl_seconds = env::var("IDEMPOTENCY_KEY_TTL_SECONDS")
            .unwrap_or_else(|_| "3600".to_string())
            .parse()
            .expect("IDEMPOTENCY_KEY_TTL_SECONDS must be a number");

        let database_max_connections = env::var("DATABASE_MAX_CONNECTIONS")
            .unwrap_or_else(|_| "20".to_string())
            .parse()
            .expect("DATABASE_MAX_CONNECTIONS must be a number");

        let database_acquire_timeout_seconds = env::var("DATABASE_ACQUIRE_TIMEOUT_SECONDS")
            .unwrap_or_else(|_| "5".to_string())
            .parse()
            .expect("DATABASE_ACQUIRE_TIMEOUT_SECONDS must be a number");

        let trusted_proxy_count = env::var("TRUSTED_PROXY_COUNT")
            .unwrap_or_else(|_| "0".to_string())
            .parse()
            .expect("TRUSTED_PROXY_COUNT must be a number");

        let short_code_salt = env::var("SHORT_CODE_SALT")
            .unwrap_or_else(|_| "0".to_string())
            .parse::<u64>()
            .or_else(|_| {
                // Allow hex like 0x5f3759df
                let s = env::var("SHORT_CODE_SALT").unwrap_or_else(|_| "0".to_string());
                let s = s.trim();
                if s.starts_with("0x") || s.starts_with("0X") {
                    u64::from_str_radix(&s[2..], 16)
                } else {
                    s.parse::<u64>()
                }
            })
            .expect("SHORT_CODE_SALT must be a number (decimal or 0x hex)");

        let redis_pool_max_size = env::var("REDIS_POOL_MAX_SIZE")
            .unwrap_or_else(|_| "16".to_string())
            .parse()
            .expect("REDIS_POOL_MAX_SIZE must be a number");

        Self {
            port,
            base_url,
            database_url,
            redis_url,
            cache_ttl_seconds,
            rate_limit_max_requests,
            rate_limit_window_ms,
            analytics_batch_size,
            analytics_flush_interval_seconds,
            idempotency_key_ttl_seconds,
            database_max_connections,
            database_acquire_timeout_seconds,
            trusted_proxy_count,
            short_code_salt,
            redis_pool_max_size,
        }
    }
}
