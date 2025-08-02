use axum::{
    body::Body,
    extract::{Path, State, Query, ConnectInfo},
    http::{Request, StatusCode},
    response::IntoResponse,
    Json,
};
use chrono::Utc;
use http_body_util::BodyExt;
use sqlx::query;
use std::{collections::HashMap, net::SocketAddr};
use tracing::{info, error};
use uuid::Uuid;

use crate::{
    models::{LoggedRequest, BinResponse, PingQuery, PingResponse},
    state::AppState,
};

pub async fn create_bin(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
) -> Result<impl IntoResponse, impl IntoResponse> {
    println!("Creating new bin at {}", addr);
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();

    info!(%id, %addr, "Creating new bin");

    let result = query("INSERT INTO bins (id, last_updated) VALUES (?, ?)")
        .bind(&id)
        .bind(&now)
        .execute(&state.db)
        .await;

    match result {
        Ok(_) => {
            info!(%id, %addr, "Successfully created bin");
            Ok(Json(BinResponse { bin_id: id }))
        }
        Err(err) => {
            error!(%id, %addr, %err, "Failed to create bin");
            Err((StatusCode::INTERNAL_SERVER_ERROR, "Failed to insert bin"))
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
) -> Result<String, String> {
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
        "INSERT INTO requests (bin_id, request_id, method, headers, body, timestamp) VALUES (?, ?, ?, ?, ?, ?)"
    )
    .bind(&id)
    .bind(Uuid::new_v4())
    .bind(method.to_string())
    .bind(headers_json.clone())
    .bind(body_str.clone())
    .bind(Utc::now().to_rfc3339())
    .execute(&state.db)
    .await;

    match result {
        Ok(_) => {
            info!(%id, %addr, %method, headers = %headers_json, body = %body_str, "Request logged");
            update_last_updated(&state, &id).await.ok();
            Ok("Request logged".to_string())
        },
        Err(err) => {
            error!(%id, %addr, %err, "DB error");
            Err("Bin not found or error logging request".to_string())
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
) -> Result<String, String> {
    let result = sqlx::query_scalar!("SELECT last_updated FROM bins WHERE id = ?", id)
        .fetch_optional(&state.db)
        .await
        .map_err(|err| {
            error!(%id, %err, "Failed to fetch bin expiration");
            "Bin not found".to_string()
        })?;

        let Some(bin_record) = result else {
            return Err("Bin not found".to_string());
        };

        let Some(last_updated) = bin_record else {
            return Err("Bin didnt have a last_updated field".to_string());
        };

        Ok(last_updated)
    }

pub async fn ping(Query(query): Query<PingQuery>) -> Json<PingResponse> {
    let message = query.message.unwrap_or_else(|| "pong".to_string());

    Json(PingResponse {
        ok: true,
        message,
    })
}
