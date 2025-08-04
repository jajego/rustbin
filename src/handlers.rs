use axum::{
    body::Body,
    extract::{ConnectInfo, Path, Query, State},
    http::{Request, StatusCode},
    response::{IntoResponse},
    Json,
};
use chrono::Utc;
use http_body_util::BodyExt;
use sqlx::{query, Row};
use std::{collections::HashMap, net::SocketAddr};
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::{
    models::{BinResponse, LoggedRequest, PingQuery, PingResponse},
    state::AppState,
};
use crate::utils::uuid::validate_uuid;

// Abuse prevention constants
const MAX_REQUESTS_PER_BIN: i64 = 100;
const MAX_HEADERS_SIZE: usize = 1024 * 1024; // 1MB
const MAX_BODY_SIZE: usize = 1024 * 1024; // 1MB

pub async fn create_bin(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
) -> Result<impl IntoResponse, impl IntoResponse> {
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();

    info!(%id, %addr, "Creating new bin");

    let result = query("INSERT INTO bins (id, last_updated) VALUES (?, ?)")
        .bind(&id)
        .bind(&now)
        .execute(&state.db)
        .await;

    match result {
        Ok(_) => Ok(Json(BinResponse { bin_id: id.to_string() })),
        Err(err) => {
            error!(%id, %addr, %err, "Failed to create bin");
            Err((StatusCode::INTERNAL_SERVER_ERROR, "Failed to insert bin").into_response())
        }
    }
}

async fn update_last_updated(state: &AppState, id: &str) -> Result<(), sqlx::Error> {
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
) -> Result<impl IntoResponse, impl IntoResponse> {
    validate_uuid(&id).map_err(|e| (StatusCode::BAD_REQUEST, e).into_response())?;

    let (parts, body) = req.into_parts();
    let method = parts.method;
    let headers = parts.headers;

    let body_bytes = body.collect().await.unwrap().to_bytes();
    let body_str = String::from_utf8_lossy(&body_bytes).to_string();
    
    // Validate body size
    if body_bytes.len() > MAX_BODY_SIZE {
        warn!(%id, %addr, body_size = body_bytes.len(), "Request body too large, rejecting");
        return Err((StatusCode::PAYLOAD_TOO_LARGE, "Request body exceeds 1MB limit").into_response());
    }

    let request_id = Uuid::new_v4();

    let headers_json = serde_json::to_string(
        &headers
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
            .collect::<HashMap<_, _>>(),
    ).unwrap_or_else(|_| "{}".to_string());

    // Validate headers size
    if headers_json.len() > MAX_HEADERS_SIZE {
        warn!(%id, %addr, headers_size = headers_json.len(), "Request headers too large, rejecting");
        return Err((StatusCode::PAYLOAD_TOO_LARGE, "Request headers exceed 1MB limit").into_response());
    }

    // Check if the bin exists before logging the request
    let bin_exists = query("SELECT COUNT(*) FROM bins WHERE id = ?")
        .bind(&id)
        .fetch_one(&state.db)
        .await;

    match bin_exists {
        Ok(row) => {
            let count: i64 = row.get(0);
            if count == 0 {
                warn!(%id, %addr, "Attempted to log request to non-existent bin");
                return Err((StatusCode::NOT_FOUND, "Bin not found").into_response());
            }
        }
        Err(err) => {
            error!(%id, %addr, %err, "Failed to check bin existence");
            return Err((StatusCode::INTERNAL_SERVER_ERROR, "Failed to check bin existence").into_response());
        }
    }

    let result = query(
        "INSERT INTO requests (bin_id, request_id, method, headers, body, timestamp) VALUES (?, ?, ?, ?, ?, ?)"
    )
    .bind(&id)
    .bind(&request_id)
    .bind(method.to_string())
    .bind(headers_json.clone())
    .bind(body_str.clone())
    .bind(Utc::now().to_rfc3339())
    .execute(&state.db)
    .await;

    match result {
        Ok(_) => {
            info!(%id, %addr, %method, headers = %headers_json, body = %body_str, "Request logged");

            // Enforce request limit per bin (keep only the latest 100 requests)
            let count_result = query("SELECT COUNT(*) FROM requests WHERE bin_id = ?")
                .bind(&id)
                .fetch_one(&state.db)
                .await;

            if let Ok(row) = count_result {
                let count: i64 = row.get(0);
                if count > MAX_REQUESTS_PER_BIN {
                    let excess = count - MAX_REQUESTS_PER_BIN;
                    let delete_result = query(
                        "DELETE FROM requests WHERE bin_id = ? AND id IN (
                            SELECT id FROM requests WHERE bin_id = ? ORDER BY id ASC LIMIT ?
                        )"
                    )
                    .bind(&id)
                    .bind(&id)
                    .bind(excess)
                    .execute(&state.db)
                    .await;

                    match delete_result {
                        Ok(deleted) => {
                            info!(%id, rows_deleted = deleted.rows_affected(), "Cleaned up old requests to maintain limit");
                        },
                        Err(err) => {
                            error!(%id, %err, "Failed to clean up old requests");
                        }
                    }
                }
            }
            update_last_updated(&state, &id).await.ok();

            if let Some(sender) = state.bin_channels.get(&id) {
                let payload = serde_json::json!({
                    "method": method.to_string(),
                    "headers": headers_json,
                    "body": body_str,
                    "timestamp": Utc::now().to_rfc3339(),
                    "request_id": request_id,
                });
                let _ = sender.send(payload.to_string());
            }

            Ok("Request logged".to_string())
        },
        Err(err) => {
            error!(%id, %addr, %err, "DB error");
            Err((StatusCode::NOT_FOUND, "Bin not found or error logging request").into_response())
        }
    }
}

pub async fn inspect_bin(
    State(state): State<AppState>,
    Path(id): Path<String>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
) -> Result<impl IntoResponse, impl IntoResponse> {
    validate_uuid(&id).map_err(|e| (StatusCode::BAD_REQUEST, e).into_response())?;

    // First, check if the bin exists
    let bin_exists = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM bins WHERE id = ?")
        .bind(&id)
        .fetch_one(&state.db)
        .await;

    match bin_exists {
        Ok(count) if count == 0 => {
            info!(%id, %addr, "Attempted to inspect non-existent bin");
            return Err((StatusCode::NOT_FOUND, "Bin not found").into_response());
        },
        Err(err) => {
            error!(%id, %addr, %err, "Failed to check bin existence");
            return Err((StatusCode::INTERNAL_SERVER_ERROR, "Failed to check bin existence").into_response());
        },
        _ => {} // Bin exists, continue
    }

    // Now fetch the requests for this bin
    let rows = sqlx::query_as::<_, LoggedRequest>(
        r#"
        SELECT 
            method, 
            headers, 
            body, 
            timestamp,
            request_id
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
            info!(%id, %addr, request_count = data.len(), "Successfully fetched bin requests");
            Ok(Json(data))
        },
        Err(err) => {
            error!(%id, %addr, %err, "Failed to fetch logged requests");
            Err((StatusCode::INTERNAL_SERVER_ERROR, "Failed to fetch logged requests").into_response())
        }
    }
}

pub async fn delete_bin(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, impl IntoResponse> {
    let uuid = validate_uuid(&id).map_err(|e| (StatusCode::BAD_REQUEST, e).into_response())?;

    let result = query("DELETE FROM bins WHERE id = ?").bind(uuid.to_string())
    .execute(&state.db)
    .await;

    match result {
        Ok(res) => {
            if res.rows_affected() == 0 {
                return Err((StatusCode::NOT_FOUND, "Bin not found").into_response());
            }
            info!(%id, %addr, "Bin deleted");
            update_last_updated(&state, &id).await.ok();
            Ok("Bin deleted".to_string())
        },
        Err(err) => {
            error!(%id, %addr,  %err, "DB error");
            Err((StatusCode::NOT_FOUND, "Bin not found or error deleting Bin").into_response())     
        }
    }
}

pub async fn delete_request(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, impl IntoResponse> {
    let uuid = validate_uuid(&id).map_err(|e| (StatusCode::BAD_REQUEST, e).into_response())?;

    let result = query("DELETE FROM requests WHERE request_id = ?").bind(uuid)
    .execute(&state.db)
    .await;

    match result {
        Ok(res) => {
            if res.rows_affected() == 0 {
                return Err((StatusCode::NOT_FOUND, "Request not found").into_response());
            }
            info!(%id, %addr, "Request deleted");
            update_last_updated(&state, &id).await.ok();
            Ok("Request deleted".to_string())
        },
        Err(err) => {
            error!(%id, %addr,  %err, "DB error");
            Err((StatusCode::NOT_FOUND, "Request not found or error deleting request").into_response())     
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        extract::{State, ConnectInfo, Path, Query},
        http::{Request, Method},
        body::Body,
    };
    use std::net::SocketAddr;
    use sqlx::sqlite::SqlitePoolOptions;
    use uuid::Uuid;
    use serde_json::{from_slice};
    use http_body_util::BodyExt;

    pub async fn setup_test_db() -> AppState {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect(":memory:")
            .await
            .unwrap();

        sqlx::query("CREATE TABLE bins (id TEXT UNIQUE PRIMARY KEY, last_updated TEXT NOT NULL);")
            .execute(&pool)
            .await
            .unwrap();

        sqlx::query("CREATE TABLE requests (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            bin_id TEXT,
            request_id TEXT UNIQUE NOT NULL,
            method TEXT,
            headers TEXT,
            body TEXT,
            timestamp TEXT
        );")
        .execute(&pool)
        .await
        .unwrap();

        AppState {
            db: pool,
            bin_channels: std::sync::Arc::new(dashmap::DashMap::new()),
        }
    }

    fn test_addr() -> SocketAddr {
        SocketAddr::from(([0, 0, 0, 0], 8080))
    }

    async fn response_json<T: for<'de> serde::Deserialize<'de>>(resp: impl IntoResponse) -> T {
        let response = resp.into_response();
        let (_parts, body) = response.into_parts();
        let bytes = body.collect().await.unwrap().to_bytes();
        from_slice(&bytes).unwrap()
    }

    async fn response_string(resp: impl IntoResponse) -> String {
        let response = resp.into_response();
        let (_parts, body) = response.into_parts();
        let bytes = body.collect().await.unwrap().to_bytes();
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    #[tokio::test]
    async fn test_create_bin() {
        let state = setup_test_db().await;
        let addr = test_addr();
        let result = create_bin(State(state), ConnectInfo(addr)).await;
        assert!(result.is_ok());
        let resp = result.ok().unwrap();
        let bin_response: BinResponse = response_json(resp).await;
        assert!(Uuid::parse_str(&bin_response.bin_id).is_ok());
    }

    #[tokio::test]
    async fn test_log_request_and_inspect_bin() {
        let state = setup_test_db().await;
        let addr = test_addr();
        // Create a bin first
        let bin_id = {
            let result = create_bin(State(state.clone()), ConnectInfo(addr)).await;
            assert!(result.is_ok());
            let resp = result.ok().unwrap();
            let bin_response: BinResponse = response_json(resp).await;
            bin_response.bin_id
        };
        // Log a request
        let req = Request::builder()
            .method(Method::POST)
            .uri("/")
            .header("x-test", "true")
            .body(Body::from("test body"))
            .unwrap();
        let log_result = log_request(
            State(state.clone()),
            Path(bin_id.clone()),
            ConnectInfo(addr),
            req,
        )
        .await;
        assert!(log_result.is_ok());
        // Inspect bin
        let result = inspect_bin(
            State(state.clone()),
            Path(bin_id.clone()),
            ConnectInfo(addr),
        )
        .await;
        assert!(result.is_ok());
        let resp = result.ok().unwrap();
        let requests: Vec<LoggedRequest> = response_json(resp).await;
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].method, "POST");
        assert_eq!(requests[0].body.as_deref(), Some("test body"));
    }

    #[tokio::test]
    async fn test_delete_bin() {
        let state = setup_test_db().await;
        let addr = test_addr();
        let bin_id = {
            let result = create_bin(State(state.clone()), ConnectInfo(addr)).await;
            assert!(result.is_ok());
            let resp = result.ok().unwrap();
            let bin_response: BinResponse = response_json(resp).await;
            bin_response.bin_id
        };
        // Delete the bin
        let result = delete_bin(
            State(state.clone()),
            ConnectInfo(addr),
            Path(bin_id.clone()),
        )
        .await;
        assert!(result.is_ok());
        let resp = result.ok().unwrap();
        let msg = response_string(resp).await;
        assert_eq!(msg, "Bin deleted");
        // Try deleting again, should be not found
        let result = delete_bin(
            State(state.clone()),
            ConnectInfo(addr),
            Path(bin_id.clone()),
        )
        .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_delete_request() {
        let state = setup_test_db().await;
        let addr = test_addr();
        let bin_id = {
            let result = create_bin(State(state.clone()), ConnectInfo(addr)).await;
            assert!(result.is_ok());
            let resp = result.ok().unwrap();
            let bin_response: BinResponse = response_json(resp).await;
            bin_response.bin_id
        };
        // Log a request
        let req = Request::builder()
            .method(Method::POST)
            .uri("/")
            .header("x-test", "true")
            .body(Body::from("test body"))
            .unwrap();
        let log_result = log_request(
            State(state.clone()),
            Path(bin_id.clone()),
            ConnectInfo(addr),
            req,
        )
        .await;
        assert!(log_result.is_ok());
        // Get the request_id
        let result = inspect_bin(
            State(state.clone()),
            Path(bin_id.clone()),
            ConnectInfo(addr),
        )
        .await;
        assert!(result.is_ok());
        let resp = result.ok().unwrap();
        let requests: Vec<LoggedRequest> = response_json(resp).await;
        let request_id = requests[0].request_id.to_string();
        // Delete the request
        let result = delete_request(
            State(state.clone()),
            ConnectInfo(addr),
            Path(request_id.clone()),
        )
        .await;
        assert!(result.is_ok());
        let resp = result.ok().unwrap();
        let msg = response_string(resp).await;
        assert_eq!(msg, "Request deleted");
        // Try deleting again, should be not found
        let result = delete_request(
            State(state.clone()),
            ConnectInfo(addr),
            Path(request_id.clone()),
        )
        .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_ping() {
        let query = PingQuery { message: Some("hello".to_string()) };
        let resp = ping(Query(query)).await;
        let axum::Json(resp) = resp;
        assert!(resp.ok);
        assert_eq!(resp.message, "hello");
        let query = PingQuery { message: None };
        let resp = ping(Query(query)).await;
        let axum::Json(resp) = resp;
        assert!(resp.ok);
        assert_eq!(resp.message, "pong");
    }

    #[tokio::test]
    async fn test_log_request_body_size_limit() {
        let state = setup_test_db().await;
        let addr = test_addr();
        
        // Create a bin first
        let bin_id = {
            let result = create_bin(State(state.clone()), ConnectInfo(addr)).await;
            assert!(result.is_ok());
            let resp = result.ok().unwrap();
            let bin_response: BinResponse = response_json(resp).await;
            bin_response.bin_id
        };

        // Create a body larger than 1MB
        let large_body = "x".repeat(MAX_BODY_SIZE + 1);
        let req = Request::builder()
            .method(Method::POST)
            .uri("/")
            .body(Body::from(large_body))
            .unwrap();

        let log_result = log_request(
            State(state.clone()),
            Path(bin_id.clone()),
            ConnectInfo(addr),
            req,
        )
        .await;
        
        assert!(log_result.is_err());
        let response = log_result.err().unwrap().into_response();
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[tokio::test]
    async fn test_log_request_headers_size_limit() {
        let state = setup_test_db().await;
        let addr = test_addr();
        
        // Create a bin first
        let bin_id = {
            let result = create_bin(State(state.clone()), ConnectInfo(addr)).await;
            assert!(result.is_ok());
            let resp = result.ok().unwrap();
            let bin_response: BinResponse = response_json(resp).await;
            bin_response.bin_id
        };

        // Create headers that will exceed 1MB when serialized
        let large_header_value = "x".repeat(MAX_HEADERS_SIZE / 2);
        let req = Request::builder()
            .method(Method::POST)
            .uri("/")
            .header("large-header-1", &large_header_value)
            .header("large-header-2", &large_header_value)
            .header("large-header-3", &large_header_value)
            .body(Body::from("small body"))
            .unwrap();

        let log_result = log_request(
            State(state.clone()),
            Path(bin_id.clone()),
            ConnectInfo(addr),
            req,
        )
        .await;
        
        assert!(log_result.is_err());
        let response = log_result.err().unwrap().into_response();
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[tokio::test]
    async fn test_request_limit_enforcement() {
        let state = setup_test_db().await;
        let addr = test_addr();
        
        // Create a bin first
        let bin_id = {
            let result = create_bin(State(state.clone()), ConnectInfo(addr)).await;
            assert!(result.is_ok());
            let resp = result.ok().unwrap();
            let bin_response: BinResponse = response_json(resp).await;
            bin_response.bin_id
        };

        // Log 101 requests (one more than the limit)
        for i in 0..=100 {
            let req = Request::builder()
                .method(Method::POST)
                .uri("/")
                .header("request-number", i.to_string())
                .body(Body::from(format!("request body {}", i)))
                .unwrap();

            let log_result = log_request(
                State(state.clone()),
                Path(bin_id.clone()),
                ConnectInfo(addr),
                req,
            )
            .await;
            assert!(log_result.is_ok());
        }

        // Check that only 100 requests remain
        let result = inspect_bin(
            State(state.clone()),
            Path(bin_id.clone()),
            ConnectInfo(addr),
        )
        .await;
        assert!(result.is_ok());
        let resp = result.ok().unwrap();
        let requests: Vec<LoggedRequest> = response_json(resp).await;
        
        // Should have exactly 100 requests
        assert_eq!(requests.len(), 100);
        
        // The oldest request (request 0) should be gone, newest should be request 100
        let bodies: Vec<String> = requests.iter()
            .map(|r| r.body.clone().unwrap_or_default())
            .collect();
        
        assert!(!bodies.contains(&"request body 0".to_string()));
        assert!(bodies.contains(&"request body 100".to_string()));
    }

    #[tokio::test]
    async fn test_inspect_nonexistent_bin() {
        let state = setup_test_db().await;
        let addr = test_addr();
        
        let fake_bin_id = Uuid::new_v4().to_string();
        
        let result = inspect_bin(
            State(state.clone()),
            Path(fake_bin_id),
            ConnectInfo(addr),
        )
        .await;
        
        assert!(result.is_err());
        let response = result.err().unwrap().into_response();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_inspect_invalid_uuid() {
        let state = setup_test_db().await;
        let addr = test_addr();
        
        let result = inspect_bin(
            State(state.clone()),
            Path("not-a-uuid".to_string()),
            ConnectInfo(addr),
        )
        .await;
        
        assert!(result.is_err());
        let response = result.err().unwrap().into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_log_request_nonexistent_bin() {
        let state = setup_test_db().await;
        let addr = test_addr();
        
        let fake_bin_id = Uuid::new_v4().to_string();
        let req = Request::builder()
            .method(Method::POST)
            .uri("/")
            .body(Body::from("test"))
            .unwrap();

        let log_result = log_request(
            State(state.clone()),
            Path(fake_bin_id),
            ConnectInfo(addr),
            req,
        )
        .await;
        
        assert!(log_result.is_err());
        let response = log_result.err().unwrap().into_response();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_boundary_sizes() {
        let state = setup_test_db().await;
        let addr = test_addr();
        
        // Create a bin first
        let bin_id = {
            let result = create_bin(State(state.clone()), ConnectInfo(addr)).await;
            assert!(result.is_ok());
            let resp = result.ok().unwrap();
            let bin_response: BinResponse = response_json(resp).await;
            bin_response.bin_id
        };

        // Test body at exactly the limit (should succeed)
        let exactly_limit_body = "x".repeat(MAX_BODY_SIZE);
        let req = Request::builder()
            .method(Method::POST)
            .uri("/")
            .body(Body::from(exactly_limit_body))
            .unwrap();

        let log_result = log_request(
            State(state.clone()),
            Path(bin_id.clone()),
            ConnectInfo(addr),
            req,
        )
        .await;
        assert!(log_result.is_ok());

        // Test body one byte over the limit (should fail)
        let over_limit_body = "x".repeat(MAX_BODY_SIZE + 1);
        let req = Request::builder()
            .method(Method::POST)
            .uri("/")
            .body(Body::from(over_limit_body))
            .unwrap();

        let log_result = log_request(
            State(state.clone()),
            Path(bin_id.clone()),
            ConnectInfo(addr),
            req,
        )
        .await;
        assert!(log_result.is_err());
    }

    #[tokio::test]
    async fn test_request_ordering() {
        let state = setup_test_db().await;
        let addr = test_addr();
        
        // Create a bin first
        let bin_id = {
            let result = create_bin(State(state.clone()), ConnectInfo(addr)).await;
            assert!(result.is_ok());
            let resp = result.ok().unwrap();
            let bin_response: BinResponse = response_json(resp).await;
            bin_response.bin_id
        };

        // Log multiple requests with distinct markers
        for i in 0..5 {
            let req = Request::builder()
                .method(Method::POST)
                .uri("/")
                .body(Body::from(format!("request_{}", i)))
                .unwrap();

            let log_result = log_request(
                State(state.clone()),
                Path(bin_id.clone()),
                ConnectInfo(addr),
                req,
            )
            .await;
            assert!(log_result.is_ok());
            
            // Small delay to ensure different timestamps
            tokio::time::sleep(tokio::time::Duration::from_millis(1)).await;
        }

        // Check that requests are returned in the correct order (by id, which should be chronological)
        let result = inspect_bin(
            State(state.clone()),
            Path(bin_id.clone()),
            ConnectInfo(addr),
        )
        .await;
        assert!(result.is_ok());
        let resp = result.ok().unwrap();
        let requests: Vec<LoggedRequest> = response_json(resp).await;
        
        assert_eq!(requests.len(), 5);
        
        // Requests should be ordered by ID (chronological order)
        for i in 0..5 {
            assert_eq!(requests[i].body.as_deref().unwrap(), format!("request_{}", i));
        }
    }
}