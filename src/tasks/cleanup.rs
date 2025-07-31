use sqlx::SqlitePool;
use chrono::{Utc, Duration};
use tokio::time::{sleep, Duration as TokioDuration};
use tracing::warn;

pub async fn start_cleanup_task(db: SqlitePool) {
    tokio::spawn(async move {
        loop {
            let cutoff = Utc::now() - Duration::hours(1);
            if let Err(err) = sqlx::query("DELETE FROM bins WHERE last_updated < ?")
                .bind(cutoff)
                .execute(&db)
                .await
            {
                warn!("Failed to delete expired bins: {:?}", err);
            }
            sleep(TokioDuration::from_secs(60)).await;
        }
    });
}
