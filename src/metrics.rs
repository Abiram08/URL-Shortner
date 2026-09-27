use axum::{extract::Request, middleware::Next, response::Response};
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use std::time::Instant;

/// Install the global Prometheus metrics recorder and return a handle
/// that can render the current snapshot on `/metrics`.
pub fn install_recorder() -> PrometheusHandle {
    PrometheusBuilder::new()
        .install_recorder()
        .expect("failed to install Prometheus recorder")
}

/// Tower middleware that records request count and latency.
///
/// This runs after routing but before the handler, so we see the real
/// HTTP method and path (including 404s).
pub async fn middleware(request: Request, next: Next) -> Response {
    let start = Instant::now();
    let method = request.method().to_string();
    // Use the matched route template (e.g. "/:code") instead of the raw URL.
    // This keeps the Prometheus label cardinality low even when millions of
    // different short codes are requested.
    let path = request
        .extensions()
        .get::<axum::extract::MatchedPath>()
        .map(|mp| mp.as_str().to_string())
        .unwrap_or_else(|| {
            // Fallback for 404s / unmatched routes: don't use raw path (high cardinality).
            // Use a bounded label so an attacker requesting /random1, /random2, ... 
            // doesn't create unbounded Prometheus series.
            let p = request.uri().path();
            if p.starts_with("/assets/") {
                "/assets/*".to_string()
            } else if p == "/" || p.starts_with("/health") || p.starts_with("/metrics") {
                p.to_string()
            } else {
                "unknown".to_string()
            }
        });

    let response = next.run(request).await;

    let duration = start.elapsed().as_secs_f64();
    let status = response.status().as_u16().to_string();

    metrics::counter!(
        "http_requests_total",
        "method" => method.clone(),
        "path" => path.clone(),
        "status" => status.clone(),
    )
    .increment(1);

    metrics::histogram!(
        "http_request_duration_seconds",
        "method" => method,
        "path" => path,
    )
    .record(duration);

    response
}
