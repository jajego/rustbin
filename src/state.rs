use sqlx::{SqlitePool, sqlite::SqlitePoolOptions};

#[derive(Clone)]
pub struct AppState {
    pub db: SqlitePool,
}

impl AppState {
    pub async fn new() -> Result<Self, sqlx::Error> {
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect(&std::env::var("DATABASE_URL").expect("DATABASE_URL must be set"))
            .await?;

        Ok(AppState { db: pool })
    }
}
