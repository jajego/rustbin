use serde::{Serialize, Deserialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LoggedRequest {
   pub method: String,
   pub headers: Vec<(String, String)>,
   pub body: String,
   pub timestamp: String,
}
