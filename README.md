# groundwork

[![CI](https://github.com/brefwiz/groundwork/actions/workflows/ci.yml/badge.svg)](https://github.com/brefwiz/groundwork/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/groundwork.svg)](https://crates.io/crates/groundwork)
[![docs.rs](https://docs.rs/groundwork/badge.svg)](https://docs.rs/groundwork)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Rust 1.85+](https://img.shields.io/badge/rust-1.85%2B-orange.svg)](https://www.rust-lang.org)

Every new service in a platform starts the same way. Someone wires up `tracing-subscriber`. Someone connects the database pool. Someone adds a `/health` endpoint, a graceful shutdown hook, a request-ID middleware, a body-size limit. They copy it from the last service, or they write it from scratch, and it comes out slightly different every time. By service five the telemetry setup alone exists in four different variants.

This is the problem that gets worse in the AI era. An agent scaffolding a new service will invent its own `main.rs` from scratch — different shutdown ordering, different observability setup, different error handling — unless there is a single bootstrap to reach for.

**groundwork is that bootstrap.** One builder, one call to `serve()`, and your service gets: structured tracing, Postgres connection + migrations, in-process GCRA rate limiting, request-ID propagation, health endpoints, graceful shutdown with drain hooks, CORS, body-size limiting, panic recovery, and OpenAPI + Swagger UI — all wired in the correct order, consistently, every time.

## Who this is for

- **Platform engineers** who want every service in their estate to boot the same way, with the same observability and operational guarantees
- **Founding engineers** who don't want to reinvent service plumbing on every new microservice
- **AI-assisted teams** building services that need to follow a shared convention without per-service bespoke wiring
- **Wrapper-crate authors** who want a stable, escape-hatched foundation to build opinionated layers on top of

## Usage

```toml
[dependencies]
groundwork = "0.1"
```

```rust
use groundwork::{ServiceBootstrap, BootstrapCtx, Result};
use axum::{Router, routing::get};

#[tokio::main]
async fn main() -> Result<()> {
    ServiceBootstrap::new("billing-service")
        .with_telemetry()
        .with_database("postgres://localhost/billing")
        .with_router(|_ctx: &BootstrapCtx| {
            Router::new().route("/orders", get(|| async { "[]" }))
        })
        .serve("0.0.0.0:8080")
        .await
}
```

See [`examples/`](examples/) for runnable examples.

## Use cases

### Config-driven bootstrap

Load all settings from environment variables or a TOML file, then pass the config to the builder. Useful for 12-factor services and Kubernetes deployments.

```rust
use groundwork::{ServiceBootstrap, BootstrapConfig, Result};
use axum::{Router, routing::get};

#[tokio::main]
async fn main() -> Result<()> {
    // Reads GROUNDWORK_* env vars, DATABASE_URL, OTEL_EXPORTER_OTLP_ENDPOINT
    let cfg = BootstrapConfig::from_env()?;
    ServiceBootstrap::from_config("payments-service", cfg)?
        .with_router(|_| Router::new().route("/ping", get(|| async { "pong" })))
        .run()  // uses bind_addr from config
        .await
}
```

Or from a TOML file with env-var overrides:

```toml
# service.toml
bind_addr = "0.0.0.0:8080"
health_path = "/health"
shutdown_timeout_secs = 30

[rate_limit]
kind = "memory"
limit = 100
window_secs = 60
```

```rust
let cfg = BootstrapConfig::load("service.toml")?;
```

### Health endpoints — always mounted

Every service gets `/health/live` and `/health/ready` automatically. No routes to define.

```
GET /health/live
→ 200 { "status": "pass", "version": "1.2.3", "service_id": "billing-service" }

GET /health/ready
→ 200 { "db": [{ "status": "pass" }] }
→ 503 { "db": [{ "status": "fail", "output": "connection refused" }] }
```

Register dependency checks with `with_readiness_check`:

```rust
use groundwork::ServiceBootstrap;
use api_bones::health::HealthCheck;

ServiceBootstrap::new("billing-service")
    .with_readiness_check("database", || async {
        // probe your pool here
        HealthCheck::pass("database")
    });
```

### Rate limiting with zero boilerplate

Add GCRA rate limiting in one line. The limiter is backed by `governor` and applied as a tower layer — no middleware to wire manually.

```rust
use groundwork::{ServiceBootstrap, RateLimitBackend, RateLimitExtractor};

ServiceBootstrap::new("api-gateway")
    .with_rate_limit(RateLimitBackend { limit: 100, window_secs: 60 })
    // Default extractor is the remote IP. Switch to a header behind a proxy:
    .with_rate_limit_extractor(RateLimitExtractor::Header("x-forwarded-for".into()));
```

When the limit is exceeded, clients receive a structured 429 with standard rate-limit headers:

```
HTTP/1.1 429 Too Many Requests
x-ratelimit-limit: 100
x-ratelimit-remaining: 0
x-ratelimit-reset: 1714000060
retry-after: 42

{
  "type": "urn:api-bones:error:rate-limited",
  "title": "Too Many Requests",
  "status": 429,
  "detail": "Rate limit exceeded. Retry after the indicated number of seconds."
}
```

### Graceful shutdown with drain hooks

Register async callbacks that run after the HTTP server stops accepting connections, before the process exits. Hooks run in reverse registration order.

```rust
use std::time::Duration;
use groundwork::ServiceBootstrap;

ServiceBootstrap::new("worker-service")
    .with_shutdown_hook("flush-metrics", Duration::from_secs(5), || async {
        // flush in-flight metrics, drain queues, etc.
    })
    .with_shutdown_hook("close-pool", Duration::from_secs(10), || async {
        // pool.close().await;
    });
```

### CORS — opt-in, never permissive by default

No CORS headers are sent unless you configure them explicitly. There is no "allow all origins" default.

```rust
use groundwork::{ServiceBootstrap, CorsConfig};

ServiceBootstrap::new("public-api")
    .with_cors_config(CorsConfig {
        allowed_origins: vec!["https://app.example.com".into()],
        allow_credentials: true,
        max_age_secs: Some(3600),
        ..Default::default()
    })?;
```

### OpenAPI + Swagger UI

Mount a utoipa-generated spec and Swagger UI with a single call. The health endpoints are merged into the spec automatically.

```rust
use groundwork::ServiceBootstrap;
use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(paths(list_orders))]
struct ApiDoc;

ServiceBootstrap::new("orders-service")
    .with_openapi(ApiDoc::openapi())
    // Spec at /openapi.json, UI at /docs (defaults)
    .with_openapi_paths("/openapi.json", "/docs");
```

### Escape hatches for wrapper crates

Inject arbitrary tower layers and access the fully-constructed context in the router builder. This is how `service-kit` and other internal wrapper crates extend groundwork without forking `serve()`.

```rust
use groundwork::{ServiceBootstrap, BootstrapCtx};
use axum::Router;

ServiceBootstrap::new("platform-service")
    .with_layer(|router| router.layer(my_auth_layer()))
    .with_layer(|router| router.layer(my_tracing_enrichment_layer()))
    .with_router(|ctx: &BootstrapCtx| {
        let pool = ctx.db.clone(); // sqlx::PgPool, if database feature enabled
        Router::new()
    });
```

## Builder reference

### Core

| Method | Description | Default |
|--------|-------------|---------|
| `new(name)` | Start a new bootstrap | — |
| `from_config(name, cfg)` | Build from a [`BootstrapConfig`] | — |
| `serve(addr)` | Bind and run until signal | — |
| `serve_with_shutdown(listener, future)` | Run with a caller-supplied shutdown future | — |
| `run()` | Run using `bind_addr` from `from_config` | — |
| `with_version(v)` | Override the version reported by `/health/live` | `CARGO_PKG_VERSION` |
| `with_health_path(p)` | Override health endpoint base path | `/health` |
| `with_body_limit(bytes)` | Max request body size | `2 MiB` |
| `with_shutdown_timeout(d)` | Graceful shutdown deadline | `30s` |
| `with_shutdown_hook(name, timeout, f)` | Register an async drain callback | — |
| `with_readiness_check(name, f)` | Register a readiness probe | — |
| `with_health_probe(probe)` | Register a typed `HealthProbe` | — |
| `with_router(f)` | Provide the axum router builder closure | — |
| `with_layer(f)` | Inject an arbitrary tower layer | — |

### Telemetry

| Method | Description |
|--------|-------------|
| `with_telemetry()` | Enable `tracing-subscriber` JSON/pretty setup |
| `with_telemetry_init(f)` | Override the telemetry init function (e.g. full OTel SDK) |

### Database (`database` feature)

| Method | Description |
|--------|-------------|
| `with_database(url)` | Connect to Postgres and build a `PgPool` |
| `with_db_pool(pool)` | Provide a pre-built `PgPool` (takes precedence over `with_database`) |
| `with_migrations(migrator)` | Run sqlx migrations at startup |

### Rate limiting (`ratelimit-memory` feature)

| Method | Description |
|--------|-------------|
| `with_rate_limit(backend)` | Enable in-process GCRA rate limiting |
| `with_rate_limit_extractor(e)` | Override the key extractor (default: remote IP) |

`RateLimitExtractor` variants:

| Variant | Keys on |
|---------|---------|
| `Ip` (default) | L4 remote address — use `Header("x-forwarded-for")` behind a proxy |
| `Header(name)` | Arbitrary request header value |

### CORS

| Method | Description |
|--------|-------------|
| `with_cors(layer)` | Provide a raw `tower_http::cors::CorsLayer` |
| `with_cors_config(cfg)` | Configure CORS from a structured [`CorsConfig`] |

### OpenAPI (`openapi` feature)

| Method | Description |
|--------|-------------|
| `with_openapi(api)` | Mount a utoipa `OpenApi` spec and Swagger UI |
| `with_openapi_paths(spec, ui)` | Override the spec and UI mount paths |

### Config (file + env)

| Method | Description |
|--------|-------------|
| `BootstrapConfig::from_env()` | Load from `GROUNDWORK_*` env vars |
| `BootstrapConfig::load(path)` | Load from TOML file with env-var overrides |
| `BootstrapConfig::validate()` | Validate cross-field invariants |

Environment variables honored automatically:

| Env var | Field |
|---------|-------|
| `GROUNDWORK_BIND_ADDR` | `bind_addr` |
| `GROUNDWORK_HEALTH_PATH` | `health_path` |
| `GROUNDWORK_LOG_LEVEL` | `log_level` |
| `GROUNDWORK_LOG_FORMAT` | `log_format` (`pretty` or `json`) |
| `GROUNDWORK_SHUTDOWN_TIMEOUT_SECS` | `shutdown_timeout_secs` |
| `GROUNDWORK_BODY_LIMIT_BYTES` | `body_limit_bytes` |
| `DATABASE_URL` | `database_url` |
| `OTEL_EXPORTER_OTLP_ENDPOINT` | `otel_endpoint` |

## Middleware stack

Layers are applied in this order, outermost first (i.e. request processing goes top-to-bottom, response bottom-to-top):

| Layer | Always? | Notes |
|-------|---------|-------|
| `SetRequestIdLayer` | ✓ | Generates `x-request-id` (UUID v7) if absent |
| `RequestIdTaskLocalLayer` | ✓ | Propagates request ID to a task-local for logging |
| `PropagateRequestIdLayer` | ✓ | Copies `x-request-id` to responses |
| `TraceLayer` | ✓ | `tracing` span per request with method, URI, request ID |
| `CatchPanicLayer` | ✓ | Returns 500 on handler panics; does not echo panic payload |
| `CorsLayer` | opt-in | Only when `with_cors_config()` / `with_cors()` is called |
| `RequestBodyLimitLayer` | ✓ | Rejects bodies over `body_limit_bytes` (default 2 MiB) |
| `CompressionLayer` | ✓ | gzip / br / zstd response compression |
| `enrich_error_response` | ✓ | Upgrades bare 4xx/5xx bodies to RFC 9457 Problem+JSON |
| Extra layers (via `with_layer`) | opt-in | Applied innermost first |
| `RateLimitLayer` | opt-in | GCRA limiter, only when `with_rate_limit()` is called |
| User router | ✓ | Routes registered via `with_router()` |
| Health router | ✓ | `/health/live`, `/health/ready` |
| 404 fallback | ✓ | RFC 9457 Problem+JSON for unmatched routes |

## Features

| Feature | Default | What it adds |
|---------|:-------:|--------------|
| `telemetry` | ✓ | `tracing-subscriber` JSON/pretty setup via `with_telemetry()` |
| `database` | ✓ | `sqlx::PgPool` construction and migrations |
| `ratelimit-memory` | ✓ | In-process GCRA rate limiter via `governor` |
| `openapi` | ✓ | utoipa OpenAPI spec + Swagger UI |
| `dotenv` | ✓ | `.env` file loading via `dotenvy` |
| `ratelimit` | — | Rate-limit types only (no backend) |
| `validation` | — | `Valid<T>` extractor via `validator` |
| `cursor` | — | HMAC-signed opaque pagination cursors |
| `testing` | — | `reqwest`-based test helpers |

## MSRV

Rust **1.85** (edition 2024). Tested on stable.

## License

[MIT](LICENSE)
