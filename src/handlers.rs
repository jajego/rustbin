use axum::{
    body::Body,
    extract::{ConnectInfo, Path, Query, State},
    http::{header, HeaderValue, Request, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use chrono::Utc;
use http_body_util::BodyExt;
use sqlx::query;
use std::{collections::HashMap, net::SocketAddr};
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::{
    models::{BinResponse, LoggedRequest, PingQuery, PingResponse},
    state::AppState,
};
use crate::utils::uuid::validate_uuid;

#[cfg(test)]
use std::sync::Arc;
#[cfg(test)]
use dashmap::DashMap;

// Note: These constants are now configured via rustbin.toml
// They remain here for backwards compatibility with tests
#[cfg(test)]
pub const MAX_HEADERS_SIZE: usize = 1024 * 1024; // 1MB
#[cfg(test)]
pub const MAX_BODY_SIZE: usize = 1024 * 1024; // 1MB

// Common error response helpers
fn internal_error(message: String) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, message)
}

fn not_found_error(message: String) -> (StatusCode, String) {
    (StatusCode::NOT_FOUND, message)
}

fn bad_request_error(message: String) -> (StatusCode, String) {
    (StatusCode::BAD_REQUEST, message)
}

fn payload_too_large_error(message: String) -> (StatusCode, String) {
    (StatusCode::PAYLOAD_TOO_LARGE, message)
}

// Validation helpers
fn validate_bin_id(id: &str) -> Result<Uuid, (StatusCode, String)> {
    validate_uuid(id).map_err(|e| bad_request_error(e))
}

async fn check_bin_exists(state: &AppState, id: &str) -> Result<(), (StatusCode, String)> {
    let count = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM bins WHERE id = ?")
        .bind(id)
        .fetch_one(&state.db)
        .await
        .map_err(|err| {
            error!(%id, %err, "Failed to check bin existence");
            internal_error("Failed to check bin existence".to_string())
        })?;

    if count == 0 {
        warn!(%id, "Attempted to access non-existent bin");
        return Err(not_found_error("Bin not found".to_string()));
    }
    Ok(())
}

// Request processing helpers
#[derive(Debug)]
struct ProcessedRequest {
    method: String,
    headers_json: String,
    body: String,
    request_id: Uuid,
}

async fn process_request_data(
    req: Request<Body>,
    id: &str,
    addr: &SocketAddr,
    limits: &crate::config::LimitsConfig,
) -> Result<ProcessedRequest, (StatusCode, String)> {
    let (parts, body) = req.into_parts();
    let method = parts.method;
    let headers = parts.headers;

    let body_bytes = body.collect().await.unwrap().to_bytes();
    let body_str = String::from_utf8_lossy(&body_bytes).to_string();
    
    // Validate body size
    if body_bytes.len() > limits.max_body_size {
        warn!(%id, %addr, body_size = body_bytes.len(), max_allowed = limits.max_body_size, "Request body too large, rejecting");
        return Err(payload_too_large_error("Request body exceeds size limit".to_string()));
    }

    let headers_json = serde_json::to_string(
        &headers
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
            .collect::<HashMap<_, _>>(),
    ).unwrap_or_else(|_| "{}".to_string());

    // Validate headers size
    if headers_json.len() > limits.max_headers_size {
        warn!(%id, %addr, headers_size = headers_json.len(), max_allowed = limits.max_headers_size, "Request headers too large, rejecting");
        return Err(payload_too_large_error("Request headers exceed size limit".to_string()));
    }

    Ok(ProcessedRequest {
        method: method.to_string(),
        headers_json,
        body: body_str,
        request_id: Uuid::new_v4(),
    })
}

async fn enforce_request_limit(state: &AppState, bin_id: &str) -> Result<(), sqlx::Error> {
    let count = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM requests WHERE bin_id = ?")
        .bind(bin_id)
        .fetch_one(&state.db)
        .await?;

    if count > state.limits.max_requests_per_bin {
        let excess = count - state.limits.max_requests_per_bin;
        let deleted = query(
            "DELETE FROM requests WHERE bin_id = ? AND id IN (
                SELECT id FROM requests WHERE bin_id = ? ORDER BY id ASC LIMIT ?
            )"
        )
        .bind(bin_id)
        .bind(bin_id)
        .bind(excess)
        .execute(&state.db)
        .await?;

        info!(%bin_id, rows_deleted = deleted.rows_affected(), "Cleaned up old requests to maintain limit");
    }
    Ok(())
}

async fn send_websocket_notification(state: &AppState, bin_id: &str, request_data: &ProcessedRequest) {
    if let Some(sender) = state.bin_channels.get(bin_id) {
        let payload = serde_json::json!({
            "method": request_data.method,
            "headers": request_data.headers_json,
            "body": request_data.body,
            "timestamp": Utc::now().to_rfc3339(),
            "request_id": request_data.request_id,
        });
        let _ = sender.send(payload.to_string());
    }
}

// Helper function to add CORS headers to any response
fn add_cors_headers(mut response: Response) -> Response {
    let headers = response.headers_mut();
    
    // Allow all origins
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
    
    // Allow all methods
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static("GET, POST, PUT, DELETE, PATCH, HEAD, OPTIONS"),
    );
    
    // Allow all headers
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_HEADERS,
        HeaderValue::from_static("*"),
    );
    
    // Cache preflight for 1 day
    headers.insert(
        header::ACCESS_CONTROL_MAX_AGE,
        HeaderValue::from_static("86400"),
    );
    
    response
}

// Handler for OPTIONS requests (CORS preflight)
pub async fn options_handler() -> Response {
    add_cors_headers(Response::new(Body::empty()))
}

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
        Ok(_) => {
            let response = Json(BinResponse { bin_id: id.to_string() }).into_response();
            Ok(add_cors_headers(response))
        },
        Err(err) => {
            error!(%id, %addr, %err, "Failed to create bin");
            let response = (StatusCode::INTERNAL_SERVER_ERROR, "Failed to insert bin").into_response();
            Err(add_cors_headers(response))
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
    
async fn store_request_in_db(
    state: &AppState,
    bin_id: &str,
    request_data: &ProcessedRequest,
) -> Result<(), sqlx::Error> {
    query(
        "INSERT INTO requests (bin_id, request_id, method, headers, body, timestamp) VALUES (?, ?, ?, ?, ?, ?)"
    )
    .bind(bin_id)
    .bind(&request_data.request_id)
    .bind(&request_data.method)
    .bind(&request_data.headers_json)
    .bind(&request_data.body)
    .bind(Utc::now().to_rfc3339())
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
    // Validate input
    validate_bin_id(&id).map_err(|e| add_cors_headers(e.into_response()))?;
    
    // Check if bin exists
    check_bin_exists(&state, &id).await.map_err(|e| add_cors_headers(e.into_response()))?;
    
    // Process request data (headers, body, validation)
    let request_data = process_request_data(req, &id, &addr, &state.limits).await.map_err(|e| add_cors_headers(e.into_response()))?;
    
    // Store request in database
    match store_request_in_db(&state, &id, &request_data).await {
        Ok(_) => {
            info!(%id, %addr, method = %request_data.method, 
                  headers = %request_data.headers_json, body = %request_data.body, 
                  "Request logged");
            
            // Clean up old requests if needed
            if let Err(err) = enforce_request_limit(&state, &id).await {
                error!(%id, %err, "Failed to clean up old requests");
            }
            
            // Update bin timestamp
            update_last_updated(&state, &id).await.ok();
            
            // Send websocket notification
            send_websocket_notification(&state, &id, &request_data).await;
            
            // Return response with CORS headers
            let response = "Request logged".to_string().into_response();
            Ok(add_cors_headers(response))
        },
        Err(err) => {
            error!(%id, %addr, %err, "DB error");
            let response = (StatusCode::NOT_FOUND, "Bin not found or error logging request").into_response();
            Err(add_cors_headers(response))
        }
    }
}

pub async fn inspect_bin(
    State(state): State<AppState>,
    Path(id): Path<String>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
) -> Result<impl IntoResponse, impl IntoResponse> {
    // Validate input and check bin existence
    validate_bin_id(&id).map_err(|e| add_cors_headers(e.into_response()))?;
    check_bin_exists(&state, &id).await.map_err(|e| add_cors_headers(e.into_response()))?;

    // Fetch the requests for this bin
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
            let response = Json(data).into_response();
            Ok(add_cors_headers(response))
        },
        Err(err) => {
            error!(%id, %addr, %err, "Failed to fetch logged requests");
            let response = (StatusCode::INTERNAL_SERVER_ERROR, "Failed to fetch logged requests").into_response();
            Err(add_cors_headers(response))
        }
    }
}

pub async fn delete_bin(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, impl IntoResponse> {
    let uuid = validate_bin_id(&id).map_err(|e| add_cors_headers(e.into_response()))?;

    let result = query("DELETE FROM bins WHERE id = ?")
        .bind(uuid.to_string())
        .execute(&state.db)
        .await;

    match result {
        Ok(res) => {
            if res.rows_affected() == 0 {
                let response = (StatusCode::NOT_FOUND, "Bin not found").into_response();
                return Err(add_cors_headers(response));
            }
            info!(%id, %addr, "Bin deleted");
            update_last_updated(&state, &id).await.ok();
            let response = "Bin deleted".to_string().into_response();
            Ok(add_cors_headers(response))
        },
        Err(err) => {
            error!(%id, %addr, %err, "DB error");
            let response = (StatusCode::NOT_FOUND, "Bin not found or error deleting Bin").into_response();
            Err(add_cors_headers(response))     
        }
    }
}

pub async fn delete_request(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, impl IntoResponse> {
    let uuid = validate_bin_id(&id).map_err(|e| add_cors_headers(e.into_response()))?;

    let result = query("DELETE FROM requests WHERE request_id = ?")
        .bind(uuid)
        .execute(&state.db)
        .await;

    match result {
        Ok(res) => {
            if res.rows_affected() == 0 {
                let response = (StatusCode::NOT_FOUND, "Request not found").into_response();
                return Err(add_cors_headers(response));
            }
            info!(%id, %addr, "Request deleted");
            update_last_updated(&state, &id).await.ok();
            let response = "Request deleted".to_string().into_response();
            Ok(add_cors_headers(response))
        },
        Err(err) => {
            error!(%id, %addr, %err, "DB error");
            let response = (StatusCode::NOT_FOUND, "Request not found or error deleting request").into_response();
            Err(add_cors_headers(response))     
        }
    }
}

pub async fn ping(Query(query): Query<PingQuery>) -> impl IntoResponse {
    let message = query.message.unwrap_or_else(|| "pong".to_string());

    let response = Json(PingResponse {
        ok: true,
        message,
    }).into_response();
    
    add_cors_headers(response)
}

pub async fn clear_bin_requests(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, impl IntoResponse> {
    let _uuid = validate_bin_id(&id).map_err(|e| add_cors_headers(e.into_response()))?;
    
    // Check if bin exists
    check_bin_exists(&state, &id).await.map_err(|e| add_cors_headers(e.into_response()))?;

    let result = query("DELETE FROM requests WHERE bin_id = ?")
        .bind(&id)
        .execute(&state.db)
        .await;

    match result {
        Ok(res) => {
            let deleted_count = res.rows_affected();
            info!(%id, %addr, deleted_count, "Cleared all requests from bin");
            update_last_updated(&state, &id).await.ok();
            
            let response = format!("Cleared {} requests from bin", deleted_count).into_response();
            Ok(add_cors_headers(response))
        },
        Err(err) => {
            error!(%id, %addr, %err, "DB error while clearing bin requests");
            let response = (StatusCode::INTERNAL_SERVER_ERROR, "Failed to clear bin requests").into_response();
            Err(add_cors_headers(response))     
        }
    }
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
            bin_channels: Arc::new(DashMap::new()),
            limits: crate::config::LimitsConfig::default(),
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
        let ping_response: PingResponse = response_json(resp).await;
        assert!(ping_response.ok);
        assert_eq!(ping_response.message, "hello");
        
        let query = PingQuery { message: None };
        let resp = ping(Query(query)).await;
        let ping_response: PingResponse = response_json(resp).await;
        assert!(ping_response.ok);
        assert_eq!(ping_response.message, "pong");
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
    async fn test_options_request_logging() {
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

        // Log an OPTIONS request
        let req = Request::builder()
            .method(Method::OPTIONS)
            .uri("/")
            .header("Access-Control-Request-Method", "POST")
            .header("Access-Control-Request-Headers", "content-type")
            .body(Body::from(""))
            .unwrap();
        
        let log_result = log_request(
            State(state.clone()),
            Path(bin_id.clone()),
            ConnectInfo(addr),
            req,
        )
        .await;
        assert!(log_result.is_ok());

        // Inspect bin to verify OPTIONS request was logged
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
        assert_eq!(requests[0].method, "OPTIONS");
        
        // Verify CORS headers are captured
        let headers: serde_json::Value = serde_json::from_str(&requests[0].headers).unwrap();
        assert!(headers.get("access-control-request-method").is_some());
        assert!(headers.get("access-control-request-headers").is_some());
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

    #[tokio::test]
    async fn test_clear_bin_requests() {
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

        // Log multiple requests
        for i in 0..3 {
            let req = Request::builder()
                .method(Method::POST)
                .uri("/")
                .header("x-request", i.to_string())
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

        // Verify we have 3 requests
        let result = inspect_bin(
            State(state.clone()),
            Path(bin_id.clone()),
            ConnectInfo(addr),
        )
        .await;
        assert!(result.is_ok());
        let resp = result.ok().unwrap();
        let requests: Vec<LoggedRequest> = response_json(resp).await;
        assert_eq!(requests.len(), 3);

        // Clear all requests
        let result = clear_bin_requests(
            State(state.clone()),
            ConnectInfo(addr),
            Path(bin_id.clone()),
        )
        .await;
        assert!(result.is_ok());
        let resp = result.ok().unwrap();
        let msg = response_string(resp).await;
        assert!(msg.contains("Cleared 3 requests from bin"));

        // Verify all requests are gone
        let result = inspect_bin(
            State(state.clone()),
            Path(bin_id.clone()),
            ConnectInfo(addr),
        )
        .await;
        assert!(result.is_ok());
        let resp = result.ok().unwrap();
        let requests: Vec<LoggedRequest> = response_json(resp).await;
        assert_eq!(requests.len(), 0);
    }

    #[tokio::test]
    async fn test_clear_nonexistent_bin() {
        let state = setup_test_db().await;
        let addr = test_addr();
        
        let fake_bin_id = uuid::Uuid::new_v4().to_string();
        
        let result = clear_bin_requests(
            State(state.clone()),
            ConnectInfo(addr),
            Path(fake_bin_id),
        )
        .await;
        
        assert!(result.is_err());
        let response = result.err().unwrap().into_response();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_delete_individual_request_integration() {
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

        // Log multiple requests
        for i in 0..3 {
            let req = Request::builder()
                .method(Method::POST)
                .uri("/")
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

        // Get all requests
        let result = inspect_bin(
            State(state.clone()),
            Path(bin_id.clone()),
            ConnectInfo(addr),
        )
        .await;
        assert!(result.is_ok());
        let resp = result.ok().unwrap();
        let requests: Vec<LoggedRequest> = response_json(resp).await;
        assert_eq!(requests.len(), 3);

        // Delete the middle request
        let request_id_to_delete = requests[1].request_id.to_string();
        let result = delete_request(
            State(state.clone()),
            ConnectInfo(addr),
            Path(request_id_to_delete),
        )
        .await;
        assert!(result.is_ok());

        // Verify we now have 2 requests
        let result = inspect_bin(
            State(state.clone()),
            Path(bin_id.clone()),
            ConnectInfo(addr),
        )
        .await;
        assert!(result.is_ok());
        let resp = result.ok().unwrap();
        let requests: Vec<LoggedRequest> = response_json(resp).await;
        assert_eq!(requests.len(), 2);
    }
}