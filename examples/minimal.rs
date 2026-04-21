//! Minimal service: telemetry + one route.

use axum::{Router, routing::get};
use socle::{BootstrapCtx, Result, ServiceBootstrap};

#[tokio::main]
async fn main() -> Result<()> {
    ServiceBootstrap::new("my-service")
        .with_telemetry()
        .with_router(|_ctx: &BootstrapCtx| Router::new().route("/health", get(|| async { "ok" })))
        .serve("0.0.0.0:8080")
        .await
}
