//! Service with PostgreSQL: pool construction, migrations, and pool access in handlers.
//!
//! Requires `DATABASE_URL` in the environment (or a `.env` file).

use axum::{Router, extract::State, routing::get};
use groundwork::{BootstrapCtx, Result, ServiceBootstrap};
use sqlx::PgPool;

async fn health(State(pool): State<PgPool>) -> &'static str {
    let _ = sqlx::query("SELECT 1").fetch_one(&pool).await;
    "ok"
}

#[tokio::main]
async fn main() -> Result<()> {
    ServiceBootstrap::new("my-service")
        .with_telemetry()
        .with_database(std::env::var("DATABASE_URL").expect("DATABASE_URL must be set"))
        .with_router(|ctx: &BootstrapCtx| {
            let pool = ctx.db().clone();
            Router::new().route("/health", get(health)).with_state(pool)
        })
        .serve("0.0.0.0:8080")
        .await
}
