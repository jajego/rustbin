use axum::{http::StatusCode, extract::connect_info::MockConnectInfo};
use axum_test::TestServer;
use rustbin::{
    models::{BinResponse, LoggedRequest},
    routes,
    state::AppState,
};
use serde_json::Value;
use sqlx::sqlite::SqlitePoolOptions;
use std::net::SocketAddr;
use uuid::Uuid;

async fn setup_test_app() -> TestServer {
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect(":memory:")
        .await
        .unwrap();

    // Create tables
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

    let state = AppState {
        db: pool,
        bin_channels: std::sync::Arc::new(dashmap::DashMap::new()),
    };

    let app = routes::create_router(state)
        .layer(MockConnectInfo(SocketAddr::from(([127, 0, 0, 1], 8080))));
    TestServer::new(app).unwrap()
}

#[tokio::test]
async fn test_full_workflow() {
    let server = setup_test_app().await;

    // Step 1: Create a bin
    let response = server.post("/create").await;
    response.assert_status_ok();
    let bin_response: BinResponse = response.json();
    let bin_id = bin_response.bin_id;
    assert!(Uuid::parse_str(&bin_id).is_ok());

    // Step 2: Log a request to the bin
    let response = server
        .post(&format!("/bin/{}", bin_id))
        .add_header("x-test-header", "test-value")
        .add_header("content-type", "application/json")
        .text(r#"{"test": "data"}"#)
        .await;
    response.assert_status_ok();

    // Step 3: Inspect the bin
    let response = server.get(&format!("/bin/{}/inspect", bin_id)).await;
    response.assert_status_ok();
    let requests: Vec<LoggedRequest> = response.json();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, "POST");
    assert_eq!(requests[0].body.as_deref(), Some(r#"{"test": "data"}"#));

    // Step 4: Delete the request
    let request_id = requests[0].request_id.to_string();
    let response = server.delete(&format!("/request/{}", request_id)).await;
    response.assert_status_ok();

    // Step 5: Verify request is gone
    let response = server.get(&format!("/bin/{}/inspect", bin_id)).await;
    response.assert_status_ok();
    let requests: Vec<LoggedRequest> = response.json();
    assert_eq!(requests.len(), 0);

    // Step 6: Delete the bin
    let response = server.delete(&format!("/delete/{}", bin_id)).await;
    response.assert_status_ok();
}

#[tokio::test]
async fn test_abuse_prevention_integration() {
    let server = setup_test_app().await;

    // Create a bin
    let response = server.post("/create").await;
    response.assert_status_ok();
    let bin_response: BinResponse = response.json();
    let bin_id = bin_response.bin_id;

    // Test body size limit
    let large_body = "x".repeat(1024 * 1024 + 1); // 1MB + 1 byte
    let response = server
        .post(&format!("/bin/{}", bin_id))
        .text(large_body)
        .await;
    response.assert_status(StatusCode::PAYLOAD_TOO_LARGE);

    // Test request count limit by logging many requests
    for i in 0..150 {
        let response = server
            .post(&format!("/bin/{}", bin_id))
            .text(format!("request_{}", i))
            .await;
        response.assert_status_ok();
    }

    // Verify only 100 requests are kept
    let response = server.get(&format!("/bin/{}/inspect", bin_id)).await;
    response.assert_status_ok();
    let requests: Vec<LoggedRequest> = response.json();
    assert_eq!(requests.len(), 100);

    // Verify FIFO behavior - oldest requests should be gone
    let bodies: Vec<String> = requests.iter()
        .map(|r| r.body.clone().unwrap_or_default())
        .collect();
    
    // First requests should be gone
    assert!(!bodies.contains(&"request_0".to_string()));
    assert!(!bodies.contains(&"request_49".to_string()));
    
    // Latest requests should still be there
    assert!(bodies.contains(&"request_149".to_string()));
    assert!(bodies.contains(&"request_100".to_string()));
}

#[tokio::test]
async fn test_bin_not_found_scenarios() {
    let server = setup_test_app().await;

    let fake_bin_id = Uuid::new_v4().to_string();

    // Test inspect non-existent bin
    let response = server.get(&format!("/bin/{}/inspect", fake_bin_id)).await;
    response.assert_status(StatusCode::NOT_FOUND);

    // Test log request to non-existent bin
    let response = server
        .post(&format!("/bin/{}", fake_bin_id))
        .text("test")
        .await;
    response.assert_status(StatusCode::NOT_FOUND);

    // Test delete non-existent bin
    let response = server.delete(&format!("/delete/{}", fake_bin_id)).await;
    response.assert_status(StatusCode::NOT_FOUND);

    // Test invalid UUID
    let response = server.get("/bin/not-a-uuid/inspect").await;
    response.assert_status(StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_concurrent_requests() {
    let server = setup_test_app().await;

    // Create a bin
    let response = server.post("/create").await;
    response.assert_status_ok();
    let bin_response: BinResponse = response.json();
    let bin_id = bin_response.bin_id;

    // Send multiple requests sequentially (since TestServer doesn't support cloning)
    for i in 0..20 {
        let response = server
            .post(&format!("/bin/{}", bin_id))
            .text(format!("concurrent_request_{}", i))
            .await;
        response.assert_status_ok();
    }

    // Verify all requests were logged
    let response = server.get(&format!("/bin/{}/inspect", bin_id)).await;
    response.assert_status_ok();
    let requests: Vec<LoggedRequest> = response.json();
    assert_eq!(requests.len(), 20);
}

#[tokio::test]
async fn test_different_http_methods() {
    let server = setup_test_app().await;

    // Create a bin
    let response = server.post("/create").await;
    response.assert_status_ok();
    let bin_response: BinResponse = response.json();
    let bin_id = bin_response.bin_id;

    // Test different HTTP methods
    let methods = vec!["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"];
    
    for method in &methods {
        let response = match *method {
            "GET" => server.get(&format!("/bin/{}", bin_id)).await,
            "POST" => server.post(&format!("/bin/{}", bin_id)).text("test").await,
            "PUT" => server.put(&format!("/bin/{}", bin_id)).text("test").await,
            "PATCH" => server.patch(&format!("/bin/{}", bin_id)).text("test").await,
            "DELETE" => server.delete(&format!("/bin/{}", bin_id)).await,
            _ => {
                // For HEAD and OPTIONS, use a generic request
                server.method(
                    method.parse().unwrap(),
                    &format!("/bin/{}", bin_id)
                ).await
            }
        };
        response.assert_status_ok();
    }

    // Verify all methods were logged
    let response = server.get(&format!("/bin/{}/inspect", bin_id)).await;
    response.assert_status_ok();
    let requests: Vec<LoggedRequest> = response.json();
    assert_eq!(requests.len(), methods.len());

    // Verify methods are correctly recorded
    let recorded_methods: Vec<String> = requests.iter()
        .map(|r| r.method.clone())
        .collect();
    
    for method in &methods {
        assert!(recorded_methods.contains(&method.to_string()));
    }
}

#[tokio::test]
async fn test_unicode_and_special_characters() {
    let server = setup_test_app().await;

    // Create a bin
    let response = server.post("/create").await;
    response.assert_status_ok();
    let bin_response: BinResponse = response.json();
    let bin_id = bin_response.bin_id;

    // Test with various special characters and encodings
    let test_cases = vec![
        "Hello, 世界!",  // Unicode
        "🚀 Emoji test 🎉",  // Emojis
        "Special chars: <>&\"'",  // HTML special chars
        "JSON: {\"key\": \"value\"}",  // JSON content
        "XML: <root><item>value</item></root>",  // XML content
        "Binary-ish: \x00\x01\x02\x7F",  // Binary-like content
    ];

    for (i, test_content) in test_cases.iter().enumerate() {
        let response = server
            .post(&format!("/bin/{}", bin_id))
            .add_header("x-test-case", i.to_string())
            .text(*test_content)
            .await;
        response.assert_status_ok();
    }

    // Verify all requests were logged correctly
    let response = server.get(&format!("/bin/{}/inspect", bin_id)).await;
    response.assert_status_ok();
    let requests: Vec<LoggedRequest> = response.json();
    assert_eq!(requests.len(), test_cases.len());

    // Verify content integrity
    for (i, request) in requests.iter().enumerate() {
        assert_eq!(request.body.as_deref().unwrap(), test_cases[i]);
    }
}

#[tokio::test]
async fn test_ping_endpoint() {
    let server = setup_test_app().await;

    // Test ping with message
    let response = server.get("/ping?message=hello").await;
    response.assert_status_ok();
    let ping_response: Value = response.json();
    assert_eq!(ping_response["ok"], true);
    assert_eq!(ping_response["message"], "hello");

    // Test ping without message
    let response = server.get("/ping").await;
    response.assert_status_ok();
    let ping_response: Value = response.json();
    assert_eq!(ping_response["ok"], true);
    assert_eq!(ping_response["message"], "pong");
}

#[tokio::test]
async fn test_headers_processing() {
    let server = setup_test_app().await;

    // Create a bin
    let response = server.post("/create").await;
    response.assert_status_ok();
    let bin_response: BinResponse = response.json();
    let bin_id = bin_response.bin_id;

    // Send request with various headers
    let response = server
        .post(&format!("/bin/{}", bin_id))
        .add_header("content-type", "application/json")
        .add_header("user-agent", "test-agent/1.0")
        .add_header("x-custom", "custom-value")
        .add_header("authorization", "Bearer token123")
        .text(r#"{"data": "test"}"#)
        .await;
    response.assert_status_ok();

    // Verify headers were captured
    let response = server.get(&format!("/bin/{}/inspect", bin_id)).await;
    response.assert_status_ok();
    let requests: Vec<LoggedRequest> = response.json();
    assert_eq!(requests.len(), 1);

    let headers_json = &requests[0].headers;
    let headers: serde_json::Value = serde_json::from_str(headers_json).unwrap();
    
    // Check that important headers are present (keys may vary in case)
    let headers_map = headers.as_object().unwrap();
    let has_content_type = headers_map.keys().any(|k| k.to_lowercase() == "content-type");
    let has_user_agent = headers_map.keys().any(|k| k.to_lowercase() == "user-agent");
    let has_custom = headers_map.keys().any(|k| k.to_lowercase() == "x-custom");
    
    assert!(has_content_type);
    assert!(has_user_agent);
    assert!(has_custom);
}