//! Service with in-process GCRA rate limiting (requires `ratelimit-memory` feature).
//!
//! Limits each IP to 100 requests per 60-second window.

use axum::{Router, routing::get};
use groundwork::{BootstrapCtx, Result, ServiceBootstrap};

#[tokio::main]
async fn main() -> Result<()> {
    #[cfg(not(feature = "ratelimit-memory"))]
    panic!("enable the `ratelimit-memory` feature to run this example");

    #[cfg(feature = "ratelimit-memory")]
    {
        use groundwork::{RateLimitBackend, RateLimitExtractor};

        ServiceBootstrap::new("my-service")
            .with_dotenv()
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

    #[cfg(not(feature = "ratelimit-memory"))]
    Ok(())
}
