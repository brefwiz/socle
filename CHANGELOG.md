# Changelog

## [Unreleased]

## [2.1.0] — 2026-04-24

### Added

- `etagged(etag, value)` — builder for [`EtaggedHandlerResponse`], mirrors `ok(v)` / `created(v)` / `listed(page)` ergonomics.

## [2.0.0] — 2026-04-24

### Changed

- **Breaking:** `ok()`, `created()`, and `listed()` now return `HandlerResponse<T>`, `CreatedResponse<T>`, and `HandlerListResponse<T>` respectively instead of bare tuples. Handlers that previously wrote `Ok(ok(value))` now write `ok(value)`. Callers that destructured the return value directly must add `.unwrap()` or handle the `Result`.

## [1.3.0] — 2026-04-24

### Added

- `http_client::ClientBuilder::with(middleware)` — chainable extension point that appends a caller-supplied `reqwest_middleware::Middleware` to the stack after the built-in trace and request-id middleware. Multiple calls compose in call order.

## [1.2.1] — 2026-04-24

### Changed

- Bumped `opentelemetry_sdk` and `opentelemetry-prometheus` from `0.29` to `0.31`.

## [1.2.0] — 2026-04-23

### Added

- `EphemeralPostgres` — spins up a throwaway Postgres container for integration tests and exposes `connection_url()`.
- `TestApp` / `TestAppBuilder` — ephemeral axum test-server harness; builds a live server from a `Router` and returns a `TestClient` bound to it.
- `SpanRecord` / `CaptureExporter` / `init_capture_tracing()` — in-process OpenTelemetry span-capture helpers for asserting trace behaviour in tests.
- `MetricsLayer` / `MetricsService` — Tower middleware that records RED metrics (request count, error count, latency histogram) into a Prometheus registry.
- `counter()` — convenience constructor for a pre-registered `Counter<u64>`.
- `socle::http_client` module — trace-propagating `reqwest` client builder (`builder()`, `ClientBuilder`, `Client`) that injects W3C trace-context headers on every outgoing request.
- `socle::openapi` module — OpenAPI 3.0.3 helpers: `merge_health_paths`, `rewrite_nullable_for_progenitor`, `to_3_0_pretty_json`, and `BearerAuthAddon`.

### Changed

- `http_client` implementation aligned with the service-kit reference implementation.

## [1.1.0] — 2026-04-23

### Added

- `created<T>(value)` — builds `(StatusCode::CREATED, Json(ApiResponse::builder(value).build()))` for `CreatedResponse` handlers.
- `ok<T>(value)` — builds `(StatusCode::OK, Json(ApiResponse::builder(value).build()))` for `HandlerResponse` handlers.
- `listed<T>(page)` — builds `Json(ApiResponse::builder(page).build())` for `HandlerListResponse` handlers.
- `CreatedResponse<T>` — return type alias for handlers that create a resource and return it with a 201 status.
- `EtaggedHandlerResponse<T>` — return type alias for handlers that carry an ETag response header alongside the platform envelope.

### Removed

- `CreatedResult<T>` — pre-envelope bare `Json<T>` alias. Migrate to `CreatedResponse<T>` + `created()`.

## [1.0.0] — 2026-04-21

### Added

- `HandlerResponse<T>` — envelope-aware return type for single-resource handlers: `Result<(StatusCode, Json<ApiResponse<T>>), HandlerError>`.
- `HandlerListResponse<T>` — envelope-aware return type for collection handlers: `Result<Json<ApiResponse<PaginatedResponse<T>>>, HandlerError>`.

### Removed

- `HandlerResult<T>` — bare `Json<T>` wrapper removed. Migrate to `HandlerResponse<T>` or `HandlerListResponse<T>`.

## [0.1.2] — 2026-04-21

### Fixed
- docs.rs builds now succeed: excluded the `openapi` feature from `[package.metadata.docs.rs]` to avoid `utoipa-swagger-ui`'s network-dependent build script (blocked in the docs.rs sandbox).
- Resolved broken intra-doc links (`with_telemetry`, `with_database`, `Modify`) that rustdoc flagged as unresolved.

### Changed
- Added a `docs` CI job that runs `cargo doc --no-deps -D warnings` with the same feature set as docs.rs, preventing future doc regressions.

## [0.1.1] — 2026-04-21

### Changed
- Bumped `api-bones` dependency to `4.0.1`.

## [0.1.0] — 2026-04-20

### Added
- `AuthProvider` trait (`ports::auth::AuthProvider`) — extension point for pluggable authentication middleware (JWT, OIDC, API key, mTLS). Groundwork ships no built-in auth backend; wrapper crates supply their own.
- `ServiceBootstrap::with_auth_provider(P: AuthProvider)` — registers an auth provider. The layer is applied after rate-limit (so unauthenticated requests are still rate-counted) and before `with_layer` extensions.
- Opinionated axum service bootstrap builder (`ServiceBootstrap`) with telemetry, database, rate limiting, and graceful shutdown
- In-process GCRA rate limiter via `governor` as a Tower layer (`ratelimit-memory` feature)
- `BootstrapCtx` extension map for escape hatches in wrapper crates
- `TelemetryProvider` trait (`Send + Sync`) with `init(&self, service_name)` and `on_shutdown()` — plug in any OTel SDK from a wrapper crate without forking `serve()`; `on_shutdown` is automatically registered as a 30s drain hook after the HTTP server stops
- `RateLimitProvider` trait with `apply(Box<Self>, router) -> Router` — inject any tower-compatible rate-limit layer (Postgres, Redis, gossip) without using the raw `with_layer` escape hatch
- `BasicTelemetryProvider` (built-in impl: `tracing_subscriber`, no-op shutdown)
- `RateLimitBackend` implements `RateLimitProvider` under `ratelimit-memory`
- `ServiceBootstrap::with_telemetry_provider(P: TelemetryProvider)` — takes priority over `with_telemetry_init`
- `ServiceBootstrap::with_rate_limit_provider(P: RateLimitProvider)` — takes priority over `with_rate_limit`
- CI pipeline with format, clippy, audit, deny, coverage, cargo-package, and auto-tag jobs
- Auto-tag composite action: tags main on green CI and triggers `release.yml`
- `release.yml`: validate → lint → audit → test → cargo-publish → GitHub release

### Changed
- Replaced hand-rolled fixed-window `HashMap` rate limiter with `governor` GCRA algorithm
- Rate limit 429 responses use `api_bones::RateLimitInfo` for consistent body and headers
- CORS layer is now opt-in — no CORS headers are sent unless `with_cors_config()` or `with_cors()` is called (previously defaulted to `CorsLayer::permissive()`)
- 404 fallback no longer echoes the request path in the response body
- `BootstrapConfig::validate()` now returns `Error::Config` when `rate_limit.limit` or `rate_limit.window_secs` is zero

### Removed
- `RateLimitExtractor::OrgId`, `UserId`, `ApiKey` — redundant named shortcuts for `Header("x-org-id")` etc.; use `RateLimitExtractor::Header(name)` directly
- Postgres/Redis rate limiter variants and their dependencies
- Dead features: auth stub, `secret_vault`
