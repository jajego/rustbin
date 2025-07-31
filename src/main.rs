use axum::{
    routing::{get, post},
    Router,
};
use std::net::SocketAddr;

mod handlers;
mod state;
mod models;

#[tokio::main]
async fn main() {
    let app_state = state::AppState::new();

    let app = Router::new()
        .route("/create", post(handlers::create_bin))
        .route("/bin/:id", post(handlers::log_request))
        .route("/bin/:id/inspect", get(handlers::inspect_bin))
        .route("/ping", get(handlers::ping))
        .with_state(app_state);

    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    println!("Listening on http://{}", addr);

    axum::serve(tokio::net::TcpListener::bind(addr).await.unwrap(), app)
        .await
        .unwrap();
}
