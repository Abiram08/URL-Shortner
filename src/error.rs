use axum::{http::StatusCode, response::IntoResponse, Json};
use serde_json::json;

#[derive(thiserror::Error, Debug)]
pub enum AppError {
    #[error("not found")]
    NotFound,
    #[error("rate limited")]
    RateLimited,
    #[error("invalid request: {0}")]
    BadRequest(String),
    #[error("database error")]
    Database(#[from] sqlx::Error),
    #[error("cache error")]
    Cache(#[from] redis::RedisError),
}

impl IntoResponse for AppError {
    fn into_response(self) -> axum::response::Response {
        let (status, msg) = match &self {
            AppError::NotFound => (StatusCode::NOT_FOUND, self.to_string()),
            AppError::RateLimited => (StatusCode::TOO_MANY_REQUESTS, self.to_string()),
            AppError::BadRequest(m) => (StatusCode::BAD_REQUEST, m.clone()),
            AppError::Database(e) => {
                tracing::error!("database error: {}", e);
                (StatusCode::INTERNAL_SERVER_ERROR, "Internal server error".to_string())
            }
            AppError::Cache(e) => {
                tracing::error!("cache error: {}", e);
                (StatusCode::INTERNAL_SERVER_ERROR, "Internal server error".to_string())
            }
        };
        (status, Json(json!({ "error": msg }))).into_response()
    }
}
