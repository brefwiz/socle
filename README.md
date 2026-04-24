# socle

[![CI](https://github.com/brefwiz/socle/actions/workflows/ci.yml/badge.svg)](https://github.com/brefwiz/socle/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/socle.svg)](https://crates.io/crates/socle)
[![docs.rs](https://docs.rs/socle/badge.svg)](https://docs.rs/socle)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Rust 1.85+](https://img.shields.io/badge/rust-1.85%2B-orange.svg)](https://www.rust-lang.org)

Every new service in a platform starts the same way. Someone wires up `tracing-subscriber`. Someone connects the database pool. Someone adds a `/health` endpoint, a graceful shutdown hook, a request-ID middleware, a body-size limit. They copy it from the last service, or they write it from scratch, and it comes out slightly different every time. By service five the telemetry setup alone exists in four different variants.

This is the problem that gets worse in the AI era. An agent scaffolding a new service will invent its own `main.rs` from scratch — different shutdown ordering, different observability setup, different error handling — unless there is a single bootstrap to reach for.

**socle is that bootstrap.** One builder, one call to `serve()`, and your service gets: structured tracing, Postgres connection + migrations, in-process GCRA rate limiting, request-ID propagation, health endpoints, graceful shutdown with drain hooks, CORS, body-size limiting, panic recovery, and OpenAPI + Swagger UI — all wired in the correct order, consistently, every time.

## Who this is for

- **Platform engineers** who want every service in their estate to boot the same way, with the same observability and operational guarantees
- **Founding engineers** who don't want to reinvent service plumbing on every new microservice
- **AI-assisted teams** building services that need to follow a shared convention without per-service bespoke wiring
- **Wrapper-crate authors** who want a stable, escape-hatched foundation to build opinionated layers on top of

## Usage

```toml
[dependencies]
socle = "1.2"
```

```rust
use socle::{ServiceBootstrap, BootstrapCtx, Result};
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
use socle::{ServiceBootstrap, BootstrapConfig, Result};
use axum::{Router, routing::get};

#[tokio::main]
async fn main() -> Result<()> {
    // Reads SOCLE_* env vars, DATABASE_URL, OTEL_EXPORTER_OTLP_ENDPOINT
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
→ 200 { "status": "pass", "version": "1.2.0", "service_id": "billing-service" }

GET /health/ready
→ 200 { "db": [{ "status": "pass" }] }
→ 503 { "db": [{ "status": "fail", "output": "connection refused" }] }
```

Register dependency checks with `with_readiness_check` or implement the `HealthProbe` trait:

```rust
use socle::ServiceBootstrap;

ServiceBootstrap::new("billing-service")
    .with_readiness_check("cache", || async {
        // probe your cache here
        api_bones::health::HealthCheck::pass("cache")
    });
```

### Database + migrations

Connect to Postgres and optionally run sqlx migrations at startup. The pool is available in the router builder via `ctx.db()`.

```rust
use socle::{ServiceBootstrap, BootstrapCtx, Result};
use axum::{Router, routing::get, extract::State};
use sqlx::PgPool;

sqlx::migrate!("./migrations"); // generates a static Migrator

#[tokio::main]
async fn main() -> Result<()> {
    ServiceBootstrap::new("billing-service")
        .with_telemetry()
        .with_database("postgres://localhost/billing")
        .with_migrations(sqlx::migrate!())
        .with_router(|ctx: &BootstrapCtx| {
            let pool = ctx.db().clone();
            Router::new()
                .route("/orders", get(list_orders))
                .with_state(pool)
        })
        .serve("0.0.0.0:8080")
        .await
}

async fn list_orders(State(pool): State<PgPool>) -> &'static str { "[]" }
```

If you already own the pool (connection string fetched from a secrets manager, custom connection options, etc.), pass it directly:

```rust
let pool = PgPool::connect_with(options).await?;
ServiceBootstrap::new("billing-service")
    .with_db_pool(pool);
```

### Rate limiting with zero boilerplate

Add GCRA rate limiting in one line. The limiter is backed by `governor` and applied as a tower layer — no middleware to wire manually.

```rust
use socle::{ServiceBootstrap, RateLimitBackend, RateLimitExtractor};

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

Register async callbacks that run after the HTTP server stops accepting connections, before the process exits. Hooks run in reverse registration order, each with its own deadline.

```rust
use std::time::Duration;
use socle::ServiceBootstrap;

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
use socle::{ServiceBootstrap, CorsConfig};

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
use socle::ServiceBootstrap;
use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(paths(list_orders))]
struct ApiDoc;

ServiceBootstrap::new("orders-service")
    .with_openapi(ApiDoc::openapi())
    // Spec at /openapi.json, UI at /docs (defaults)
    .with_openapi_paths("/openapi.json", "/docs");
```

### Handler errors — RFC 9457 Problem+JSON throughout

`HandlerError` is the standard error type for axum handlers. It serializes as `application/problem+json` and maps sqlx errors for you.

```rust
use socle::{HandlerError, HandlerResponse};
use axum::http::StatusCode;
use api_bones::ApiResponse;

async fn get_order(/* ... */) -> HandlerResponse<Order> {
    let row = sqlx::query_as!(Order, "SELECT * FROM orders WHERE id = $1", id)
        .fetch_one(&pool)
        .await
        .map_err(|e| HandlerError::from_sqlx(&e))?; // RowNotFound → 404, unique → 409

    Ok(socle::ok(row))
}
```

The `ok`, `created`, and `listed` helpers build typed `ApiResponse` envelopes:

```rust
Ok(socle::ok(order))                   // 200 with ApiResponse<Order>
Ok(socle::created(order))              // 201 with ApiResponse<Order>
Ok(socle::listed(paginated_response))  // 200 with ApiResponse<PaginatedResponse<Order>>
```

### ETags and conditional updates

Derive a weak ETag from a row's `updated_at` timestamp and validate `If-Match` headers before mutation:

```rust
use socle::{etag_from_updated_at, check_if_match, EtaggedHandlerResponse};
use axum::http::HeaderMap;

async fn update_order(
    headers: HeaderMap,
    // ...
) -> EtaggedHandlerResponse<Order> {
    let order = fetch_order(id).await?;
    let etag = etag_from_updated_at(order.updated_at);
    check_if_match(&headers, &etag)?; // 412 if stale

    let updated = apply_patch(order, patch).await?;
    Ok((etag_from_updated_at(updated.updated_at), StatusCode::OK, Json(ok(updated))))
}
```

### Input validation

The `Valid<T>` extractor (feature `validation`) runs `validator` field validations before the handler is invoked. Invalid requests get a 422 with per-field error details automatically.

```rust
use socle::Valid;
use serde::Deserialize;
use validator::Validate;

#[derive(Deserialize, Validate)]
struct CreateOrder {
    #[validate(length(min = 1, max = 100))]
    description: String,
    #[validate(range(min = 1))]
    quantity: u32,
}

async fn create_order(Valid(body): Valid<CreateOrder>) -> impl IntoResponse {
    // body is already validated; invalid requests never reach here
}
```

### Outgoing HTTP client with trace propagation

The `http-client` feature provides a `reqwest`-based client that automatically forwards `x-request-id` and OpenTelemetry trace headers on every outgoing call — no manual header threading.

```rust
use socle::http_client;
use std::time::Duration;

let client = http_client::builder()
    .timeout(Duration::from_secs(10))
    .connect_timeout(Duration::from_secs(2))
    .user_agent("billing-service/1.0")
    .build()?;

let resp = client.get("https://inventory-service/items/42").send().await?;
```

### RED metrics via Prometheus

The `metrics` feature provides a Tower layer that records request count, latency, and error rate per route/method/status against a Prometheus registry. Mount it alongside your existing `/metrics` endpoint.

```rust
use socle::metrics::MetricsLayer;
use prometheus::Registry;

let registry = Registry::new();
let layer = MetricsLayer::new(registry.clone())?;

ServiceBootstrap::new("billing-service")
    .with_layer(move |router| router.layer(layer.clone()));
```

You can also create named counters from the global OpenTelemetry meter:

```rust
use socle::metrics::counter;

let processed = counter("orders_processed_total");
processed.add(1, &[]);
```

### Opaque pagination cursors

The `cursor` feature provides HMAC-signed, base64-encoded cursors for keyset pagination. Cursors are opaque to clients and tamper-evident.

```rust
use socle::Cursor;

// Encode a cursor from any serializable position
let cursor = Cursor::encode(&last_row_id, &secret_key)?;

// Decode and verify on the next request
let position: Uuid = cursor.decode(&secret_key)?;
```

### Escape hatches for wrapper crates

Inject arbitrary tower layers and access the fully-constructed context in the router builder. This is how `service-kit` and other internal wrapper crates extend socle without forking `serve()`.

```rust
use socle::{ServiceBootstrap, BootstrapCtx};
use axum::Router;

ServiceBootstrap::new("platform-service")
    .with_layer(|router| router.layer(my_auth_layer()))
    .with_layer(|router| router.layer(my_tracing_enrichment_layer()))
    .with_router(|ctx: &BootstrapCtx| {
        // Store and retrieve arbitrary typed values injected by wrapper crates
        let tenant_config = ctx.get::<TenantConfig>();
        Router::new()
    });
```

Four port traits give wrapper crates structured extension points:

| Trait | Purpose |
|-------|---------|
| `TelemetryProvider` | Custom OTel SDK initialisation and flush on shutdown |
| `AuthProvider` | Inject a JWT / JWKS / API-key / mTLS auth layer |
| `RateLimitProvider` | Replace the in-process limiter with a Redis or Postgres backend |
| `HealthProbe` | Typed readiness check with a `name()` and async `check()` |

## Testing

### Ephemeral HTTP test server

The `testing` feature provides `TestApp` — a real Axum server bound to an ephemeral port. Use it in integration tests to exercise the full handler stack with actual HTTP.

```toml
[dev-dependencies]
socle = { version = "1.2", features = ["testing"] }
```

```rust
use socle::testing::{TestApp, TestClient};
use axum::{Router, routing::get};

#[tokio::test]
async fn test_list_orders() {
    let app = TestApp::builder()
        .router(Router::new().route("/orders", get(|| async { "[]" })))
        .build()
        .await;

    let client = app.client();
    let resp = client.get("/orders").await;

    assert_eq!(resp.status(), 200);
    app.shutdown().await;
}
```

`TestClient` wraps `reqwest` and is pre-pointed at the test server's address. Call `.get(path)`, `.post(path, &body)`, etc. without managing base URLs.

### Ephemeral Postgres

The `testing-postgres` feature spins up a real Postgres 16 container (via `testcontainers`) and tears it down when the test completes. No test database configuration to manage.

```toml
[dev-dependencies]
socle = { version = "1.2", features = ["testing-postgres"] }
```

```rust
use socle::testing::EphemeralPostgres;

#[tokio::test]
async fn test_with_real_database() {
    let pg = EphemeralPostgres::start().await;
    let pool = pg.pool().await;

    sqlx::query("CREATE TABLE orders (id BIGSERIAL PRIMARY KEY)")
        .execute(&pool)
        .await
        .unwrap();

    // run your repository code against a real database
}
```

`EphemeralPostgres` also exposes `connection_url()` if you need to pass the URL to a migration tool or a `ServiceBootstrap` under test.

### In-memory span capture

Assert on tracing spans emitted by your handlers without shipping anything to an OTel collector.

```rust
use socle::testing::{init_capture_tracing, CaptureExporter};

#[tokio::test]
async fn test_spans_emitted() {
    let exporter = init_capture_tracing();

    // run code that emits spans
    do_something_instrumented().await;

    let spans = exporter.drain();
    assert!(spans.iter().any(|s| s.name == "orders.list"));

    // SpanRecord exposes name: String and attributes: Vec<(String, String)>
    let span = &spans[0];
    assert_eq!(span.attributes.iter().find(|(k, _)| k == "db.system"), Some(&("db.system".into(), "postgresql".into())));
}
```

## Builder reference

### Core

| Method | Description | Default |
|--------|-------------|---------|
| `new(name)` | Start a new bootstrap | — |
| `from_config(name, cfg)` | Build from a `BootstrapConfig` | — |
| `serve(addr)` | Bind and run until signal | — |
| `serve_with_shutdown(listener, future)` | Run with a caller-supplied shutdown future | — |
| `run()` | Run using `bind_addr` from `from_config` | — |
| `with_version(v)` | Override the version reported by `/health/live` | `CARGO_PKG_VERSION` |
| `with_health_path(p)` | Override health endpoint base path | `/health` |
| `with_body_limit(bytes)` | Max request body size | `2 MiB` |
| `with_shutdown_timeout(d)` | Graceful shutdown deadline | `30s` |
| `with_shutdown_hook(name, timeout, f)` | Register an async drain callback | — |
| `with_readiness_check(name, f)` | Register a readiness probe closure | — |
| `with_health_probe(probe)` | Register a typed `HealthProbe` | — |
| `with_router(f)` | Provide the axum router builder closure | — |
| `with_layer(f)` | Inject an arbitrary tower layer | — |

### Telemetry

| Method | Description |
|--------|-------------|
| `with_telemetry()` | Enable `tracing-subscriber` JSON/pretty setup |
| `with_telemetry_init(f)` | Override the telemetry init function |
| `with_telemetry_provider(p)` | Plug in a custom `TelemetryProvider` (full OTel SDK init + shutdown) |

### Database (`database` feature)

| Method | Description |
|--------|-------------|
| `with_database(url)` | Connect to Postgres and build a `PgPool` |
| `with_db_pool(pool)` | Provide a pre-built `PgPool` |
| `with_migrations(migrator)` | Run sqlx migrations at startup |

### Rate limiting (`ratelimit-memory` feature)

| Method | Description |
|--------|-------------|
| `with_rate_limit(backend)` | Enable in-process GCRA rate limiting |
| `with_rate_limit_extractor(e)` | Override the key extractor (default: remote IP) |
| `with_rate_limit_provider(p)` | Plug in a distributed `RateLimitProvider` |

`RateLimitExtractor` variants:

| Variant | Keys on |
|---------|---------|
| `Ip` (default) | L4 remote address — use `Header("x-forwarded-for")` behind a proxy |
| `Header(name)` | Arbitrary request header value |

### Auth

| Method | Description |
|--------|-------------|
| `with_auth_provider(p)` | Plug in a custom `AuthProvider` (JWT, JWKS, API-key, OIDC, mTLS) |

### CORS

| Method | Description |
|--------|-------------|
| `with_cors(layer)` | Provide a raw `tower_http::cors::CorsLayer` |
| `with_cors_config(cfg)` | Configure CORS from a structured `CorsConfig` |

### OpenAPI (`openapi` feature)

| Method | Description |
|--------|-------------|
| `with_openapi(api)` | Mount a utoipa `OpenApi` spec and Swagger UI |
| `with_openapi_paths(spec, ui)` | Override the spec and UI mount paths (defaults: `/openapi.json`, `/docs`) |

### Config (file + env)

| Method / function | Description |
|--------|-------------|
| `BootstrapConfig::from_env()` | Load from `SOCLE_*` env vars |
| `BootstrapConfig::load(path)` | Load from TOML file with env-var overrides |
| `BootstrapConfig::validate()` | Validate cross-field invariants |
| `ServiceBootstrap::with_dotenv()` | Load `.env` file before config resolution (`dotenv` feature) |

Environment variables honored automatically:

| Env var | Field |
|---------|-------|
| `SOCLE_BIND_ADDR` | `bind_addr` |
| `SOCLE_HEALTH_PATH` | `health_path` |
| `SOCLE_LOG_LEVEL` | `log_level` |
| `SOCLE_LOG_FORMAT` | `log_format` (`pretty` or `json`) |
| `SOCLE_SHUTDOWN_TIMEOUT_SECS` | `shutdown_timeout_secs` |
| `SOCLE_BODY_LIMIT_BYTES` | `body_limit_bytes` |
| `DATABASE_URL` | `database_url` |
| `OTEL_EXPORTER_OTLP_ENDPOINT` | `otel_endpoint` |

## Middleware stack

Layers are applied in this order, outermost first (request processing goes top-to-bottom, response bottom-to-top):

| Layer | Always? | Notes |
|-------|---------|-------|
| `SetRequestIdLayer` | ✓ | Generates `x-request-id` (UUIDv7) if absent; accepts inbound `x-request-id` / `x-correlation-id` |
| `RequestIdTaskLocalLayer` | ✓ | Propagates request ID to a task-local for log enrichment |
| `PropagateRequestIdLayer` | ✓ | Copies `x-request-id` to responses |
| `TraceLayer` | ✓ | `tracing` span per request with method, URI, and request ID |
| `CatchPanicLayer` | ✓ | Returns 500 on handler panics; does not echo panic payload |
| `CorsLayer` | opt-in | Only when `with_cors_config()` / `with_cors()` is called |
| `RequestBodyLimitLayer` | ✓ | Rejects bodies over `body_limit_bytes` (default 2 MiB) |
| `CompressionLayer` | ✓ | gzip / br / zstd response compression |
| `enrich_error_response` | ✓ | Upgrades bare 4xx/5xx bodies to RFC 9457 Problem+JSON |
| Extra layers (via `with_layer`) | opt-in | Applied innermost-first; this is where `AuthProvider` and `RateLimitProvider` inject |
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
| `ratelimit` | — | Rate-limit types and traits only (no backend) |
| `validation` | — | `Valid<T>` extractor with per-field 422 errors via `validator` |
| `cursor` | — | HMAC-signed opaque pagination cursors |
| `http-client` | — | Outgoing HTTP client with automatic trace/request-ID propagation |
| `metrics` | — | RED metrics Tower layer + Prometheus registry integration |
| `testing` | — | `TestApp` ephemeral server + `TestClient` + `CaptureExporter` |
| `testing-postgres` | — | `EphemeralPostgres` Docker-backed Postgres for integration tests |

## Prior art

**[`loco`](https://loco.rs)** is the closest crate in spirit — a batteries-included, Rails-inspired framework with an ORM, mailers, background jobs, CLI generation, and built-in auth. If you want a full-stack analogue to Rails, use loco. socle is a library, not a framework: it imposes no project structure, no ORM, no CLI, and no conventions beyond the bootstrap call itself. You bring axum routes; socle brings consistent plumbing.

**[`shuttle`](https://www.shuttle.rs)** solves a different problem: managed cloud deployment via an annotated `#[shuttle_runtime::main]`. It owns your infrastructure. socle owns nothing outside `serve()` and works with any deployment target.

**Roll-your-own boilerplate** is the real competitor — most teams copy a `main.rs` from the previous service and diverge over time. Four services in, telemetry is initialised four different ways, health endpoints are missing on two of them, and graceful shutdown drains in the wrong order on one. socle is what that shared boilerplate would look like if it were tested, versioned, and depended on rather than copied.

Three things socle does that the alternatives don't:

- **Correct layer ordering, always.** Rate-limiting wraps auth wraps the user router wraps telemetry — in that order, enforced by the builder, not documented somewhere and hoped for.
- **Per-hook shutdown timeouts with drain ordering.** Hooks run in reverse registration order; each has its own deadline. Getting this right in every service individually is the kind of thing that fails silently in production.
- **Port traits for wrapper crates.** `AuthProvider`, `RateLimitProvider`, `TelemetryProvider`, and `HealthProbe` let internal platforms extend socle without forking `serve()`. Add your JWT layer, your distributed rate limiter, your org-context middleware — socle stays upgradeable.

## MSRV

Rust **1.85** (edition 2024). Tested on stable.

## License

[MIT](LICENSE)
