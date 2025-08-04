mod config;
mod handlers;
mod models;
mod routes;
mod state;
mod tasks;
mod utils;
mod websocket;

use std::net::SocketAddr;
use std::sync::Arc;
use tower_http::trace::{TraceLayer, DefaultMakeSpan, DefaultOnResponse};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};
use tower_governor::{governor::GovernorConfigBuilder, GovernorLayer};

use config::RustbinConfig;

#[tokio::main]
async fn main() {
    // Load configuration (creates default config file if it doesn't exist)
    const CONFIG_PATH: &str = "rustbin.toml";
    if let Err(err) = RustbinConfig::create_default_config_if_missing(CONFIG_PATH) {
        eprintln!("Failed to create default config: {}", err);
    }
    
    let config = RustbinConfig::from_file_or_default(CONFIG_PATH);
    
    // Initialize logging with config
    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| EnvFilter::new(&config.logging.filter)))
        .with(tracing_subscriber::fmt::layer())
        .init();

    tracing::info!("Starting rustbin with configuration from {}", CONFIG_PATH);

    let app_state = state::AppState::new(&config.database, &config.limits).await.expect("Failed to init DB");
    tasks::cleanup::start_cleanup_task(
        app_state.db.clone(), 
        app_state.bin_channels.clone(),
        &config.cleanup
    ).await;

    let governor_conf = Arc::new(
       GovernorConfigBuilder::default()
           .per_second(config.rate_limiting.requests_per_second.into())
           .burst_size(config.rate_limiting.burst_size.into())
           .finish()
           .unwrap(),
   );
    tasks::limit::start_rate_limit_cleanup(&governor_conf, &config.rate_limiting).await;

    let trace = TraceLayer::new_for_http()
        .make_span_with(DefaultMakeSpan::new().include_headers(true))
        .on_response(DefaultOnResponse::new().include_headers(true));

    // Create rate-limited routes (everything except WebSocket)
    let rate_limited_routes = routes::bin::bin_routes(app_state.clone())
        .merge(routes::health::health_routes())
        .layer(GovernorLayer {
            config: governor_conf,
        });
    
    // Create WebSocket routes without rate limiting
    let websocket_routes = routes::bin::websocket_routes(app_state.clone());
    
    // Combine all routes
    let app = rate_limited_routes
        .merge(websocket_routes)
        .layer(trace);

    let addr = SocketAddr::from((
        config.server.host.parse::<std::net::IpAddr>()
            .unwrap_or_else(|_| [0, 0, 0, 0].into()),
        config.server.port
    ));
    tracing::info!("Listening on http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
        .await
        .unwrap();
}
