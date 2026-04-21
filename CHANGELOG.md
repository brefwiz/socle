# Changelog

## [0.1.0] — 2026-04-20

### Added
- Opinionated axum service bootstrap builder (`ServiceBootstrap`) with telemetry, database, rate limiting, and graceful shutdown
- In-process GCRA rate limiter via `governor` as a Tower layer (`ratelimit-memory` feature)
- `BootstrapCtx` extension map for escape hatches in wrapper crates
- CI pipeline with format, clippy, audit, deny, coverage, cargo-package, and auto-tag jobs
- Auto-tag composite action: tags main on green CI and triggers `release.yml`
- `release.yml`: validate → lint → audit → test → cargo-publish → Gitea release

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
