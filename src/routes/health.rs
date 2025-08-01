use axum::{routing::get, Router};
use crate::handlers;

pub fn health_routes() -> Router {
    Router::new().route("/ping", get(handlers::ping))
}
