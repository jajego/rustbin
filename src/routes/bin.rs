use axum::{
    routing::{get, post, delete, any},
    Router,
};
use crate::{handlers, state::AppState};
use crate::websocket::ws_handler;

pub fn bin_routes(app_state: AppState) -> Router {
    Router::new()
        .route("/create", post(handlers::create_bin))
        .route("/bin/:id", any(handlers::log_request))
        .route("/bin/:id/ws", get(ws_handler))
        .route("/bin/:id/inspect", get(handlers::inspect_bin))
        .route("/delete/:id", delete(handlers::delete_bin))
        .route("/request/:id", delete(handlers::delete_request))
        .with_state(app_state)
}
