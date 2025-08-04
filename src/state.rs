use dashmap::DashMap;
use std::sync::Arc;
use tokio::sync::broadcast;
use sqlx::{SqlitePool, sqlite::SqlitePoolOptions};

use crate::config::{DatabaseConfig, LimitsConfig};

#[derive(Clone)]
pub struct AppState {
    pub db: SqlitePool,
    pub bin_channels: Arc<DashMap<String, broadcast::Sender<String>>>,
    pub limits: LimitsConfig,
}

impl AppState {
    pub async fn new(database_config: &DatabaseConfig, limits_config: &LimitsConfig) -> Result<Self, sqlx::Error> {
        let pool = SqlitePoolOptions::new()
            .max_connections(database_config.max_connections)
            .connect(&database_config.url)
            .await?;

        Ok(AppState { 
            db: pool, 
            bin_channels: Arc::new(DashMap::new()),
            limits: limits_config.clone(),
        })
    }
}
