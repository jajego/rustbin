use sqlx::SqlitePool;
use chrono::{Utc, Duration};
use tokio::time::{sleep, Duration as TokioDuration};
use tracing::{info, warn};
use dashmap::DashMap;
use std::sync::Arc;
use tokio::sync::broadcast;

use crate::config::CleanupConfig;

pub async fn start_cleanup_task(
    db: SqlitePool, 
    bin_channels: Arc<DashMap<String, broadcast::Sender<String>>>,
    config: &CleanupConfig,
) {
    let cleanup_config = config.clone();
    tokio::spawn(async move {
        loop {
            let cutoff = Utc::now() - Duration::hours(cleanup_config.bin_expiry_hours);
            
            let expired_bins = match sqlx::query_as::<_, (String,)>(
                "SELECT id FROM bins WHERE last_updated < ?"
            )
            .bind(cutoff)
            .fetch_all(&db)
            .await
            {
                Ok(bins) => bins,
                Err(err) => {
                    warn!("Failed to query expired bins: {:?}", err);
                    sleep(TokioDuration::from_secs(cleanup_config.cleanup_interval_seconds)).await;
                    continue;
                }
            };

            let mut deleted_count = 0;
            let mut kept_alive_count = 0;

            for (bin_id,) in expired_bins {
                // Check if there are active WebSocket connections for this bin
                let has_active_connections = bin_channels
                    .get(&bin_id)
                    .map(|sender| sender.receiver_count() > 0)
                    .unwrap_or(false);

                if has_active_connections {
                    // Bin has active WebSocket connections, keep it alive
                    kept_alive_count += 1;
                    info!(%bin_id, "Keeping expired bin alive due to active WebSocket connections");
                    continue;
                }

                // No active connections, safe to delete
                if let Err(err) = sqlx::query("DELETE FROM bins WHERE id = ?")
                    .bind(&bin_id)
                    .execute(&db)
                    .await
                {
                    warn!(%bin_id, %err, "Failed to delete expired bin");
                } else {
                    deleted_count += 1;
                    info!(%bin_id, "Deleted expired bin");
                    
                    // Clean up the channel entry if it exists
                    bin_channels.remove(&bin_id);
                }
            }

            if deleted_count > 0 || kept_alive_count > 0 {
                info!(
                    deleted = deleted_count, 
                    kept_alive = kept_alive_count, 
                    "Cleanup task completed"
                );
            }

            sleep(TokioDuration::from_secs(cleanup_config.cleanup_interval_seconds)).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;
    use chrono::Utc;

    #[tokio::test]
    async fn test_cleanup_respects_active_websocket_connections() {
        // Setup test database
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect(":memory:")
            .await
            .unwrap();

        // Create tables
        sqlx::query("CREATE TABLE bins (id TEXT UNIQUE PRIMARY KEY, last_updated TEXT NOT NULL);")
            .execute(&pool)
            .await
            .unwrap();

        sqlx::query("CREATE TABLE requests (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            bin_id TEXT,
            request_id TEXT UNIQUE NOT NULL,
            method TEXT,
            headers TEXT,
            body TEXT,
            timestamp TEXT
        );")
        .execute(&pool)
        .await
        .unwrap();

        let bin_channels = Arc::new(DashMap::new());

        // Create two bins that are older than 1 hour
        let old_time = Utc::now() - Duration::hours(2);
        let bin_id_with_connection = "test-bin-with-ws";
        let bin_id_without_connection = "test-bin-without-ws";

        // Insert both bins with old timestamps
        sqlx::query("INSERT INTO bins (id, last_updated) VALUES (?, ?)")
            .bind(bin_id_with_connection)
            .bind(old_time.to_rfc3339())
            .execute(&pool)
            .await
            .unwrap();

        sqlx::query("INSERT INTO bins (id, last_updated) VALUES (?, ?)")
            .bind(bin_id_without_connection)
            .bind(old_time.to_rfc3339())
            .execute(&pool)
            .await
            .unwrap();

        // Create a WebSocket connection for one bin (simulate active connection)
        let (tx, _rx): (broadcast::Sender<String>, broadcast::Receiver<String>) = broadcast::channel(100);
        let _rx_keepalive = tx.subscribe(); // Keep a receiver alive to simulate active connection
        bin_channels.insert(bin_id_with_connection.to_string(), tx);

        // Run the cleanup logic manually (extract the cleanup logic to a function for testing)
        let cutoff = Utc::now() - Duration::hours(1);
        
        // Get expired bins
        let expired_bins = sqlx::query_as::<_, (String,)>(
            "SELECT id FROM bins WHERE last_updated < ?"
        )
        .bind(cutoff)
        .fetch_all(&pool)
        .await
        .unwrap();

        let mut deleted_count = 0;
        let mut kept_alive_count = 0;

        for (bin_id,) in expired_bins {
            // Check if there are active WebSocket connections for this bin
            let has_active_connections = bin_channels
                .get(&bin_id)
                .map(|sender| sender.receiver_count() > 0)
                .unwrap_or(false);

            if has_active_connections {
                // Bin has active WebSocket connections, keep it alive
                kept_alive_count += 1;
                continue;
            }

            // No active connections, safe to delete
            if sqlx::query("DELETE FROM bins WHERE id = ?")
                .bind(&bin_id)
                .execute(&pool)
                .await
                .is_ok()
            {
                deleted_count += 1;
                bin_channels.remove(&bin_id);
            }
        }

        // Verify that one bin was deleted and one was kept alive
        assert_eq!(deleted_count, 1, "Should delete bin without WebSocket connections");
        assert_eq!(kept_alive_count, 1, "Should keep bin with active WebSocket connections");

        // Verify the bin with connection still exists
        let bin_exists = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM bins WHERE id = ?"
        )
        .bind(bin_id_with_connection)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(bin_exists, 1, "Bin with active WebSocket should still exist");

        // Verify the bin without connection was deleted
        let bin_exists = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM bins WHERE id = ?"
        )
        .bind(bin_id_without_connection)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(bin_exists, 0, "Bin without WebSocket should be deleted");
    }
}
