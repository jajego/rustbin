use serde::{Serialize, Deserialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LoggedRequest {
   pub method: String,
   pub headers: Vec<(String, String)>,
   pub body: String,
   pub timestamp: String,
}

// Debug structs
#[derive(Serialize)]
pub struct PingResponse {
   pub ok: bool,
   pub message: String,
}

#[derive(Deserialize)]
pub struct PingQuery {
    pub message: Option<String>,
}