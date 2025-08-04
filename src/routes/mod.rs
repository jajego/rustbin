use axum::Router;
use crate::state::AppState;

pub mod bin;
pub mod health;

pub fn create_router(app_state: AppState) -> Router {
    bin::bin_routes(app_state.clone()).merge(health::health_routes())
}
