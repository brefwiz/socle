# Changelog

## [Unreleased]

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
