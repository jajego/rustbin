use governor::clock::QuantaInstant;
use governor::middleware::NoOpMiddleware;
use std::sync::Arc;
use std::time::Duration;
use tower_governor::governor::GovernorConfig;
use tower_governor::key_extractor::PeerIpKeyExtractor;

// Prevent unbounded memory growth, and evict stale IPs.
pub async fn start_rate_limit_cleanup(
    conf: &Arc<GovernorConfig<PeerIpKeyExtractor, NoOpMiddleware<QuantaInstant>>>,
) {
    let governor_limiter = conf.limiter().clone();
    let interval = Duration::from_secs(60);
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(interval);
            tracing::info!("rate limiting storage size: {}", governor_limiter.len());
            governor_limiter.retain_recent();
        }
    });
}
