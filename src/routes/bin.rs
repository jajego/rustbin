use axum::{
    routing::{get, post, delete, any},
    Router,
};
use crate::{handlers, state::AppState};

pub fn bin_routes(app_state: AppState) -> Router {
    Router::new()
        .route("/create", post(handlers::create_bin))
        .route("/bin/:id", any(handlers::log_request))
        .route("/bin/:id/inspect", get(handlers::inspect_bin))
        .route("/bin/:id/expiry", get(handlers::get_bin_expiration))
        .route("/delete/:id", delete(handlers::delete_bin))
        .route("/request/:id", delete(handlers::delete_request))
        .with_state(app_state)
}
