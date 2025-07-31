use axum::{
    extract::{Path, State, Query},
    http::{HeaderMap, Method, Request},
    body::Body,
    Json,
};
use uuid::Uuid;
use chrono::Utc;
use std::collections::HashMap;

use http_body_util::BodyExt;

use crate::{state::AppState, models::LoggedRequest, models::PingResponse, models::PingQuery};

pub async fn create_bin(State(state): State<AppState>) -> Json<HashMap<&'static str, String>> {
    let id = Uuid::new_v4().to_string();
    state.bins.lock().unwrap().insert(id.clone(), Vec::new());
    Json(HashMap::from([("bin_id", id)]))
}

pub async fn log_request(
    State(state): State<AppState>,
    Path(id): Path<String>,
    method: Method,
    headers: HeaderMap,
    req: Request<Body>,
) -> String {
    let body = req.into_body();
    let body_bytes = body.collect().await.unwrap().to_bytes();
    let body_str = String::from_utf8_lossy(&body_bytes).to_string();

    let mut bins = state.bins.lock().unwrap();
    if let Some(logs) = bins.get_mut(&id) {
        let log = LoggedRequest {
            method: method.to_string(),
            headers: headers
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
                .collect(),
            body: body_str,
            timestamp: Utc::now().to_rfc3339(),
        };
        logs.push(log);
        "Request logged".to_string()
    } else {
        "Bin not found".to_string()
    }
}

pub async fn inspect_bin(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Json<Vec<LoggedRequest>> {
    let bins = state.bins.lock().unwrap();
    if let Some(logs) = bins.get(&id) {
        Json(logs.clone())
    } else {
        Json(vec![])
    }
}

pub async fn ping(Query(query): Query<PingQuery>) -> Json<PingResponse> {
    let message = query
        .message
        .unwrap_or_else(|| "pong".to_string());

    Json(PingResponse {
        ok: true,
        message,
    })
}
