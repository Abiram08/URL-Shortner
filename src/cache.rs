use redis::aio::MultiplexedConnection;
use redis::Client;

pub async fn create_redis_client(redis_url: &str) -> MultiplexedConnection {
    let client = Client::open(redis_url).expect("Failed to parse Redis URL");
    let mut attempts = 0;
    loop {
        match client.get_multiplexed_async_connection().await {
            Ok(conn) => return conn,
            Err(e) => {
                attempts += 1;
                if attempts >= 10 {
                    panic!("Failed to connect to Redis after {} attempts: {}", attempts, e);
                }
                tracing::warn!("Redis not ready (attempt {}/10): {}, retrying in 3s", attempts, e);
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            }
        }
    }
}
