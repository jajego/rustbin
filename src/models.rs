use serde::{Serialize, Deserialize};

#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct LoggedRequest {
   pub request_id: String,
   pub method: String,
   pub headers: String,
   pub body: Option<String>,
   pub timestamp: String,
}

#[derive(Serialize)]
pub struct BinResponse {
    pub bin_id: String,
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
