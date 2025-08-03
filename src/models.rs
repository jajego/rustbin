use serde::{Serialize, Deserialize};
use uuid::Uuid;
#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct LoggedRequest {
   pub method: String,
   pub headers: String,
   pub body: Option<String>,
   pub timestamp: String,
   pub request_id: Uuid,
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
