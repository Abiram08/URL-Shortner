use sqlx::{PgPool, Row};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::time::{interval, Duration};

/// Analytics via mpsc channel — redirect path only does `try_send` (no I/O).
/// Background task batches every 5s into a single UPDATE per code.
/// Trade-off: up to 5s of clicks can be lost on a hard crash.
pub struct AnalyticsQueue {
    sender: mpsc::Sender<String>,
    pool: PgPool,
}

impl AnalyticsQueue {
    pub fn new(pool: PgPool, _batch_size: i64) -> Arc<Self> {
        let (tx, rx) = mpsc::channel::<String>(10_000);
        let pool_clone = pool.clone();
        tokio::spawn(flush_loop(pool_clone, rx));
        Arc::new(Self { sender: tx, pool })
    }

    pub fn start_flusher(self: Arc<Self>, _interval_seconds: u64) {
        // Already spawned in new(); kept for compat with main.rs.
    }

    pub fn record_click(&self, short_code: &str) {
        if let Err(e) = self.sender.try_send(short_code.to_string()) {
            tracing::warn!("analytics channel full/dropped: {}", e);
        }
    }
}

async fn flush_loop(pool: PgPool, mut rx: mpsc::Receiver<String>) {
    let mut ticker = interval(Duration::from_secs(5));
    let mut buf: Vec<String> = Vec::new();
    loop {
        tokio::select! {
            _ = ticker.tick() => {
                if !buf.is_empty() {
                    flush_batch(&pool, std::mem::take(&mut buf)).await;
                }
            }
            msg = rx.recv() => match msg {
                Some(code) => buf.push(code),
                None => {
                    if !buf.is_empty() { flush_batch(&pool, buf).await; }
                    break;
                }
            }
        }
    }
}

async fn flush_batch(pool: &PgPool, batch: Vec<String>) {
    if batch.is_empty() { return; }
    let mut counts: HashMap<String, i32> = HashMap::new();
    for c in batch { *counts.entry(c).or_insert(0) += 1; }
    for (code, count) in counts {
        if let Err(e) = sqlx::query("UPDATE urls SET click_count = click_count + $1 WHERE short_code = $2")
            .bind(count).bind(&code).execute(pool).await
        { tracing::error!("analytics flush failed for {}: {}", code, e); }
    }
}
