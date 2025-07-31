use axum::{
    body::Body,
    extract::{Path, State, Query, ConnectInfo},
    http::{Request},
    Json,
};
use chrono::Utc;
use http_body_util::BodyExt;
use sqlx::query;
use sqlx::Row;
use std::{collections::HashMap, net::SocketAddr};
use tracing::{info, error};
use uuid::Uuid;

use crate::{
    models::{LoggedRequest, PingQuery, PingResponse},
    state::AppState,
};

pub async fn create_bin(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
) -> Json<HashMap<&'static str, String>> {
    let id = Uuid::new_v4().to_string();
    info!(%id, %addr, "Creating new bin");

    let now = Utc::now().to_rfc3339();

    let result = query("INSERT INTO bins (id, last_updated) VALUES (?, ?)")
        .bind(&id)
        .bind(&now)
        .execute(&state.db)
        .await;

    match result {
        Ok(_) => {
            info!(%id, %addr, "Successfully created bin");
            Json(HashMap::from([("bin_id", id)]))
        }
        Err(err) => {
            error!(%id, %addr, %err, "Failed to create bin");
            panic!("Failed to insert bin");
        }
    }
}

async fn update_last_updated(
    state: &AppState,
    id: &str,
) -> Result<(), sqlx::Error> {
    let now = Utc::now().to_rfc3339();
    query("UPDATE bins SET last_updated = ? WHERE id = ?")
        .bind(&now)
        .bind(id)
        .execute(&state.db)
        .await?;
    Ok(())
}

pub async fn log_request(
    State(state): State<AppState>,
    Path(id): Path<String>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    req: Request<Body>,
) -> String {
    let (parts, body) = req.into_parts();

    let method = parts.method;
    let headers = parts.headers;

    let body_bytes = body.collect().await.unwrap().to_bytes();
    let body_str = String::from_utf8_lossy(&body_bytes).to_string();

    let headers_json = serde_json::to_string(
        &headers
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
            .collect::<HashMap<_, _>>(),
    ).unwrap();

    let result = query(
        "INSERT INTO requests (bin_id, method, headers, body, timestamp) VALUES (?, ?, ?, ?, ?)"
    )
    .bind(&id)
    .bind(method.to_string())
    .bind(headers_json.clone()) // Clone for easy use in info! below
    .bind(body_str.clone())
    .bind(Utc::now().to_rfc3339())
    .execute(&state.db)
    .await;

    match result {
        Ok(_) => {
            info!(%id, %addr, %method, headers = %headers_json, body = %body_str, "Request logged");
            update_last_updated(&state, &id).await.ok();
            "Request logged".to_string()
        },
        Err(err) => {
            error!(%id, %addr, %err, "DB error");
            "Bin not found or error logging request".to_string()
        }
    }
}

pub async fn inspect_bin(
    State(state): State<AppState>,
    Path(id): Path<String>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
) -> Json<Vec<LoggedRequest>> {
    info!(%id, %addr, "Inspecting bin");

    let rows = sqlx::query_as::<_, LoggedRequest>(
        r#"
        SELECT 
            method, 
            headers, 
            body, 
            timestamp
        FROM requests
        WHERE bin_id = ?
        ORDER BY id
        "#
    )
    .bind(&id)
    .fetch_all(&state.db)
    .await;

    match rows {
        Ok(data) => {
            info!(%id, %addr, count = data.len(), "Successfully fetched logged requests");
            Json(data)
        }
        Err(err) => {
            error!(%id, %addr, %err, "Failed to fetch logged requests");
            Json(vec![])
        }
    }
}

pub async fn get_bin_expiration(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> String {
    let row = sqlx::query("SELECT last_updated FROM bins WHERE id = ?")
        .bind(&id)
        .fetch_one(&state.db)
        .await;

    match row {
        Ok(row) => {
            let last_updated: String = row.get("last_updated");
            format!("Bin {} was last updated at {}", id, last_updated)
        }
        Err(err) => {
            error!(%id, %err, "Failed to fetch bin expiration");
            "Bin not found".to_string()
        }
    }
}

pub async fn ping(Query(query): Query<PingQuery>) -> Json<PingResponse> {
    let message = query.message.unwrap_or_else(|| "pong".to_string());

    Json(PingResponse {
        ok: true,
        message,
    })
}
