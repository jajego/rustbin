use axum::{
    body::Body,
    extract::{ConnectInfo, Path, Query, State},
    http::{Request, StatusCode},
    response::{IntoResponse},
    Json,
};
use chrono::Utc;
use http_body_util::BodyExt;
use sqlx::query;
use std::{collections::HashMap, net::SocketAddr};
use tracing::{error, info};
use uuid::Uuid;

use crate::{
    models::{BinResponse, LoggedRequest, PingQuery, PingResponse},
    state::AppState,
};
use crate::utils::uuid::validate_uuid;

pub async fn create_bin(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
) -> Result<impl IntoResponse, impl IntoResponse> {
    let id = Uuid::new_v4();
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

    let headers_json = serde_json::to_string(
        &headers
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
            .collect::<HashMap<_, _>>(),
    ).unwrap_or_else(|_| "{}".to_string());

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

    print!("Hiii! {:?}", rows);

    match rows {
        Ok(data) => Ok(Json(data)),
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

    let result = query("DELETE FROM bins WHERE id = ?").bind(uuid)
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

    print!("Result is {:?}", result);

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

    async fn setup_test_db() -> AppState {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect(":memory:")
            .await
            .unwrap();

        sqlx::query("CREATE TABLE bins (id TEXT PRIMARY KEY, last_updated TEXT NOT NULL);")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("CREATE TABLE requests (id INTEGER PRIMARY KEY AUTOINCREMENT, bin_id TEXT, request_id TEXT, method TEXT, headers TEXT, body TEXT, timestamp TEXT);")
            .execute(&pool)
            .await
            .unwrap();

        AppState { db: pool }
    }

    fn test_addr() -> SocketAddr {
        SocketAddr::from(([127, 0, 0, 1], 8080))
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
}