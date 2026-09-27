mod analytics;
mod cache;
mod config;
mod db;
mod error;
mod idempotency;
mod metrics;
mod rate_limiter;
mod shortener;

use analytics::AnalyticsQueue;
use axum::{
    extract::{ConnectInfo, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    middleware,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use cache::create_redis_client;
use config::Config;
use db::{create_pool, initialize_database};
use rate_limiter::RateLimiter;
use redis::aio::MultiplexedConnection;
use serde::{Deserialize, Serialize};
use serde_json::json;
use shortener::{get_long_url, get_url_stats, save_url};
use sqlx::PgPool;
use std::net::SocketAddr;
use std::sync::{Arc, OnceLock};
use metrics_exporter_prometheus::PrometheusHandle;
use tower_http::services::{ServeDir, ServeFile};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

/// Global handle to the Prometheus recorder. Kept outside of AppState so the
/// state struct stays small and the metrics endpoint does not need to clone it.
static PROMETHEUS: OnceLock<PrometheusHandle> = OnceLock::new();

/// Shared application state passed to every handler.
#[derive(Clone)]
struct AppState {
    pool: PgPool,
    redis: MultiplexedConnection,
    config: Config,
    rate_limiter: Arc<RateLimiter>,
    analytics: Arc<AnalyticsQueue>,
}

#[derive(Deserialize)]
struct ShortenRequest {
    // The frontend sends camelCase JSON, so map it to Rust's snake_case field.
    #[serde(rename = "longUrl")]
    long_url: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ShortenResponse {
    short_url: String,
    short_code: String,
}

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
    redis: &'static str,
    database: &'static str,
    timestamp: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StatsResponse {
    short_code: String,
    long_url: String,
    click_count: i32,
    created_at: chrono::DateTime<chrono::Utc>,
}

fn client_ip(headers: &HeaderMap, addr: &SocketAddr, trusted_proxy_count: usize) -> String {
    // Only trust X-Forwarded-For if the deploy explicitly says a proxy is in front.
    // Otherwise an attacker can send `X-Forwarded-For: 1.2.3.4` and bypass rate limits.
    if trusted_proxy_count == 0 {
        return addr.ip().to_string();
    }
    // When behind a trusted proxy, take the left-most (original client) entry.
    if let Some(v) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
        if let Some(first) = v.split(',').next().map(|s| s.trim()) {
            if !first.is_empty() {
                return first.to_string();
            }
        }
    }
    addr.ip().to_string()
}

async fn health_handler(State(state): State<AppState>) -> impl IntoResponse {
    let db_ok = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        sqlx::query("SELECT 1").fetch_one(&state.pool),
    )
    .await
    .map(|r| r.is_ok())
    .unwrap_or(false);

    let mut redis = state.redis.clone();
    let redis_ok = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        redis::cmd("PING").query_async::<_, String>(&mut redis),
    )
    .await
    .map(|r: Result<String, _>| r.map(|p| p == "PONG").unwrap_or(false))
    .unwrap_or(false);

    let status = if db_ok && redis_ok { "ok" } else { "degraded" };

    (
        StatusCode::OK,
        Json(HealthResponse {
            status,
            redis: if redis_ok { "connected" } else { "disconnected" },
            database: if db_ok { "connected" } else { "disconnected" },
            timestamp: chrono::Utc::now().to_rfc3339(),
        }),
    )
}

async fn shorten_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(payload): Json<ShortenRequest>,
) -> impl IntoResponse {
    let mut redis = state.redis.clone();

    // Validate Idempotency-Key: 1..128 chars, ascii alphanumeric + -_ only.
    // Prevent Redis key blow-up from huge attacker headers.
    let idempotency_key = headers
        .get("idempotency-key")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().to_string())
        .filter(|s| {
            !s.is_empty()
                && s.len() <= 128
                && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
        });

    if let Some(key) = &idempotency_key {
        if let Some(json) = idempotency::lookup(&mut redis, key).await {
            if let Ok(cached) = serde_json::from_str::<ShortenResponse>(&json) {
                return (StatusCode::CREATED, Json(cached)).into_response();
            }
        }
    }

    let ip = client_ip(&headers, &addr, state.config.trusted_proxy_count);

    if !state.rate_limiter.allow(&mut redis, &ip).await {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({ "error": "Rate limit exceeded. Slow down!" })),
        )
            .into_response();
    }

    let mut redis = state.redis.clone();
    match save_url(
        &state.pool,
        &mut redis,
        &state.config.base_url,
        &payload.long_url,
        state.config.cache_ttl_seconds,
    )
    .await
    {
        Ok(short_code) => {
            let short_url = format!("{}/{}", state.config.base_url, short_code);
            let response = ShortenResponse {
                short_url: short_url.clone(),
                short_code: short_code.clone(),
            };

            // Cache the result under the idempotency key so retries see the
            // same short URL instead of creating duplicates.
            if let Some(key) = idempotency_key {
                if let Ok(json) = serde_json::to_string(&response) {
                    idempotency::store(
                        &mut redis,
                        &key,
                        &json,
                        state.config.idempotency_key_ttl_seconds,
                    )
                    .await;
                }
            }

            (StatusCode::CREATED, Json(response)).into_response()
        }
        Err(e) => {
            let msg = e.to_string();
            // Validation errors are 400, everything else is 500 without leaking internals.
            if msg.starts_with("Invalid URL") {
                (
                    StatusCode::BAD_REQUEST,
                    Json(json!({ "error": msg })),
                )
                    .into_response()
            } else {
                tracing::error!("Shorten failed: {}", msg);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({ "error": "Internal server error" })),
                )
                    .into_response()
            }
        }
    }
}

async fn redirect_handler(
    State(state): State<AppState>,
    axum::extract::Path(short_code): axum::extract::Path<String>,
) -> impl IntoResponse {
    // Redirects are intentionally not rate-limited as strictly as POST /shorten;
    // they must serve 1000s of req/s and are guarded by short_code validation
    // (alphanumeric, <=10) in shortener::get_long_url.
    let mut redis = state.redis.clone();
    match get_long_url(
        &state.pool,
        &mut redis,
        &short_code,
        state.config.cache_ttl_seconds,
    )
    .await
    {
        Ok(Some(long_url)) => {
            // Analytics is fire-and-forget: never let a click-record failure
            // break the redirect for the end user.
            state.analytics.record_click(&short_code).await;
            let location = match HeaderValue::from_str(&long_url) {
                Ok(v) => v,
                Err(_) => {
                    tracing::error!("Invalid Location header value for {}: {}", short_code, long_url);
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(json!({ "error": "Invalid redirect URL" })),
                    )
                        .into_response();
                }
            };
            (StatusCode::FOUND, [(header::LOCATION, location)]).into_response()
        }
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "Short code not found" })),
        )
            .into_response(),
        Err(e) => {
            tracing::error!("Redirect failed: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "Internal server error" })),
            )
                .into_response()
        }
    }
}

async fn metrics_handler() -> String {
    PROMETHEUS
        .get()
        .expect("Prometheus recorder not initialized")
        .render()
}

async fn stats_handler(
    State(state): State<AppState>,
    axum::extract::Path(short_code): axum::extract::Path<String>,
) -> impl IntoResponse {
    match get_url_stats(&state.pool, &short_code).await {
        Ok(Some(stats)) => (
            StatusCode::OK,
            Json(StatsResponse {
                short_code: stats.short_code,
                long_url: stats.long_url,
                click_count: stats.click_count,
                created_at: stats.created_at,
            }),
        )
            .into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "Short code not found" })),
        )
            .into_response(),
        Err(e) => {
            tracing::error!("Stats lookup failed: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "Internal server error" })),
            )
                .into_response()
        }
    }
}

#[tokio::main]
async fn main() {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "shorty=info,tower_http=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let config = Config::from_env();
    let pool = create_pool(
        &config.database_url,
        config.database_max_connections,
        config.database_acquire_timeout_seconds,
    )
    .await;
    let redis = create_redis_client(&config.redis_url).await;

    if let Err(e) = initialize_database(&pool).await {
        tracing::error!("Database initialization failed: {}", e);
        std::process::exit(1);
    }

    let rate_limiter = RateLimiter::new(
        config.rate_limit_max_requests,
        config.rate_limit_window_ms,
    );
    let analytics = AnalyticsQueue::new(pool.clone(), config.analytics_batch_size);
    analytics
        .clone()
        .start_flusher(config.analytics_flush_interval_seconds);

    let prometheus = metrics::install_recorder();
    PROMETHEUS
        .set(prometheus)
        .expect("Prometheus recorder already initialized");

    let state = AppState {
        pool: pool.clone(),
        redis,
        config: config.clone(),
        rate_limiter,
        analytics,
    };

    let app = Router::new()
        .route("/health", get(health_handler))
        .route("/shorten", post(shorten_handler))
        // Serve static assets before the `/:code` catch-all swallows them.
        .route_service("/", ServeFile::new("public/index.html"))
        .route_service("/app.js", ServeFile::new("public/app.js"))
        .route_service("/shorty.css", ServeFile::new("public/shorty.css"))
        .nest_service("/assets", ServeDir::new("public/assets"))
        .route("/:code", get(redirect_handler))
        .route("/stats/:code", get(stats_handler))
        .route("/metrics", get(metrics_handler))
        .fallback_service(ServeDir::new("public").append_index_html_on_directories(true))
        // Tower order: outermost = first in ServiceBuilder.
        // Trace outermost so every request is logged, BodyLimit innermost so it rejects before parsing.
        .layer(middleware::from_fn(metrics::middleware))
        .layer(tower_http::trace::TraceLayer::new_for_http())
        .layer(tower_http::limit::RequestBodyLimitLayer::new(8 * 1024))
        .with_state(state);

    let addr = format!("0.0.0.0:{}", config.port);
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();

    tracing::info!("Shorty listening on http://{}", addr);

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await
    .unwrap();

    pool.close().await;
    tracing::info!("Shutdown complete.");
}

async fn shutdown_signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("failed to install Ctrl+C handler");
    tracing::info!("Received shutdown signal, finishing active requests...");
}
