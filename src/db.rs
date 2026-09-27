use sqlx::postgres::PgPoolOptions;
use sqlx::{migrate::MigrateError, PgPool};

/// Create a PostgreSQL connection pool.
/// The pool is the source of truth for all URL data.
/// Retries for ~30s so the app survives Postgres starting slightly later.
pub async fn create_pool(database_url: &str, max_connections: u32, acquire_timeout_seconds: u64) -> PgPool {
    let mut attempts = 0;
    loop {
        match PgPoolOptions::new()
            .max_connections(max_connections)
            .acquire_timeout(std::time::Duration::from_secs(acquire_timeout_seconds))
            .connect(database_url)
            .await
        {
            Ok(pool) => return pool,
            Err(e) => {
                attempts += 1;
                if attempts >= 10 {
                    panic!("Failed to create PostgreSQL pool after {} attempts: {}", attempts, e);
                }
                tracing::warn!(
                    "Postgres not ready (attempt {}/10): {}, retrying in 3s",
                    attempts,
                    e
                );
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            }
        }
    }
}

/// Run embedded migrations before the app accepts traffic.
/// Migrations live in ./migrations and are compiled into the binary,
/// so the same image can upgrade its own schema on startup.
pub async fn initialize_database(pool: &PgPool) -> Result<(), MigrateError> {
    sqlx::migrate!("./migrations").run(pool).await
}
