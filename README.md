# groundwork

[![CI](https://github.com/brefwiz/groundwork/actions/workflows/ci.yml/badge.svg)](https://github.com/brefwiz/groundwork/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/groundwork.svg)](https://crates.io/crates/groundwork)
[![docs.rs](https://docs.rs/groundwork/badge.svg)](https://docs.rs/groundwork)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Rust 1.85+](https://img.shields.io/badge/rust-1.85%2B-orange.svg)](https://www.rust-lang.org)

Opinionated [axum](https://github.com/tokio-rs/axum) service bootstrap: telemetry, database, rate limiting, and graceful shutdown in one builder.

Extracted from an internal Brefwiz service kit. The API is **0.x — expect breaking changes before 1.0.**

## Quick start

```rust,no_run
use groundwork::{ServiceBootstrap, BootstrapCtx, Result};
use axum::{Router, routing::get};

#[tokio::main]
async fn main() -> Result<()> {
    ServiceBootstrap::new("billing-service")
        .with_telemetry()
        .with_database("postgres://localhost/billing")
        .with_router(|_ctx: &BootstrapCtx| {
            Router::new().route("/health", get(|| async { "ok" }))
        })
        .serve("0.0.0.0:8080")
        .await
}
```

See the [`examples/`](examples/) directory for runnable examples.

## Features

All features are enabled by default. Disable them individually with `default-features = false`.

| Feature | Default | What it adds |
|---|:---:|---|
| `telemetry` | ✓ | `tracing-subscriber` JSON/pretty setup |
| `database` | ✓ | `sqlx::PgPool` construction and migrations |
| `ratelimit-memory` | ✓ | In-process GCRA rate limiter via `governor` |
| `openapi` | ✓ | utoipa OpenAPI spec + Swagger UI at `/docs` |
| `dotenv` | ✓ | `.env` file loading via `dotenvy` |
| `ratelimit` | — | Rate-limit trait only (no backend) |
| `validation` | — | `Valid<T>` extractor via `validator` |
| `cursor` | — | Base64-encoded cursor pagination |
| `testing` | — | Test helpers (`reqwest`-based) |

## MSRV

Rust **1.85** (edition 2024). Tested on stable.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md).

## License

[MIT](LICENSE)
