use axum::{
    routing::{get, post, delete, any, options},
    Router,
};
use crate::{handlers, state::AppState};
use crate::websocket::ws_handler;

pub fn bin_routes(app_state: AppState) -> Router {
    Router::new()
        .route("/create", post(handlers::create_bin))
        .route("/bin/:id", options(handlers::log_request))  // Explicit OPTIONS handler
        .route("/bin/:id", any(handlers::log_request))      // All other methods
        .route("/bin/:id/inspect", get(handlers::inspect_bin))
        .route("/bin/:id/clear", options(handlers::options_handler))  // OPTIONS for CORS preflight
        .route("/bin/:id/clear", delete(handlers::clear_bin_requests))  // Clear all requests
        .route("/delete/:id", delete(handlers::delete_bin))
        .route("/request/:id", options(handlers::options_handler))  // OPTIONS for CORS preflight
        .route("/request/:id", delete(handlers::delete_request))
        .with_state(app_state)
}

pub fn websocket_routes(app_state: AppState) -> Router {
    Router::new()
        .route("/bin/:id/ws", get(ws_handler))
        .with_state(app_state)
}
