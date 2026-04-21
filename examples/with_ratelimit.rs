//! Service with in-process GCRA rate limiting.
//!
//! Limits each IP to 100 requests per 60-second window.

use axum::{Router, routing::get};
use socle::{BootstrapCtx, RateLimitBackend, RateLimitExtractor, Result, ServiceBootstrap};

#[tokio::main]
async fn main() -> Result<()> {
    ServiceBootstrap::new("my-service")
        .with_telemetry()
        .with_rate_limit(RateLimitBackend {
            limit: 100,
            window_secs: 60,
        })
        .with_rate_limit_extractor(RateLimitExtractor::Ip)
        .with_router(|_ctx: &BootstrapCtx| Router::new().route("/", get(|| async { "hello" })))
        .serve("0.0.0.0:8080")
        .await
}
