use serde::{Serialize, Deserialize};

#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct LoggedRequest {
   pub method: String,
   pub headers: Option<String>, // ← changed to Option<String>
   pub body: String,
   pub timestamp: String,
}

#[derive(Serialize)]
pub struct PingResponse {
    pub ok: bool,
    pub message: String,
}

#[derive(Deserialize)]
pub struct PingQuery {
    pub message: Option<String>,
}
