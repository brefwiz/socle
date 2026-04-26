# Changelog

## [3.0.1] — 2026-04-26

### Added

- **`testing::handler_assert`** — typed assertion helpers for `RfcOk<T>`-returning handler unit tests (gated on `rfc-types`, no extra feature flag required). Covers the full api-bones payload surface:
  - `payload<T>` / `list_payload<T>` / `cursor_payload<T>` / `keyset_payload<T>` / `bulk_payload<T>` — extract typed data from `RfcOk<T>` without `T: Debug`
  - `status` / `headers` / `etag_header` — low-level header accessors
  - `unwrap_ok` / `unwrap_list` / `unwrap_cursor` / `unwrap_keyset` / `unwrap_bulk` — convenience unwraps for `Result<RfcOk<T>, HandlerError>`
  - `unwrap_status` — returns `(StatusCode, T)` in one call
  - `unwrap_created` — returns `(StatusCode, HeaderMap, T)` for `CreatedAtResponse` handlers
  - `unwrap_err` / `unwrap_err_status` — extract the error without a `Debug` bound on `T`

## [3.0.0] — 2026-04-26

### Breaking changes

Handler success arms are now sealed — direct `Ok((StatusCode, Json(...)))` construction no longer compiles when the `rfc-types` feature is active (the new default).

**Before:**
```rust
async fn get_item(id: Uuid) -> HandlerResponse<Item> {
    let item = fetch(id).await?;
    Ok((StatusCode::OK, Json(ApiResponse::builder(item).build())))  // ← no longer compiles
}
```

**After:** Use the existing builder functions, which already produce the correct type:
```rust
async fn get_item(id: Uuid) -> HandlerResponse<Item> {
    let item = fetch(id).await?;
    ok(item)  // ← unchanged call site; now the only valid path
}
```

The `Ok` arm of each type alias has changed:

| Type alias | Before | After |
|---|---|---|
| `HandlerResponse<T>` | `(StatusCode, Json<ApiResponse<T>>)` | `RfcOk<T>` |
| `CreatedResponse<T>` | `(StatusCode, Json<ApiResponse<T>>)` | `RfcOk<T>` |
| `CreatedAtResponse<T>` | `(StatusCode, HeaderMap, Json<ApiResponse<T>>)` | `RfcOk<T>` |
| `EtaggedHandlerResponse<T>` | `(StatusCode, ETag, Json<ApiResponse<T>>)` | `RfcOk<T>` |
| `HandlerListResponse<T>` | `Json<ApiResponse<PaginatedResponse<T>>>` | `RfcOk<PaginatedResponse<T>>` |

`created_under` now requires `T: Serialize` (previously deferred to axum).

### Added

- **`RfcOk<T>`** — sealed success wrapper produced by all builder functions.
  Exposes `.status()`, `.headers()`, and `.body_json()` for inspection.
  Cannot be constructed outside this crate.  Gated on `rfc-types`.

- **`UnconstrainedResponse`** — explicit, always-available opt-out for routes
  whose wire format is externally mandated.  Each use must be documented at the
  call site with a product-level justification.  See ADR platform/0020.

- **`rfc-types` feature** (default: enabled) — enables the sealed `RfcOk<T>`
  types.  Disable only during a migration window; prefer `UnconstrainedResponse`
  for per-handler opt-outs.

### Migration

1. Replace `Ok((StatusCode::..., Json(...)))` in handler bodies with the
   builder functions (`ok`, `created`, etc.).  Handlers already using the
   builders need no changes.

2. Update tests that destructured the `Ok` arm:
   ```rust
   // before
   let (status, body) = ok(x).unwrap();
   // after
   let resp = ok(x).unwrap();
   assert_eq!(resp.status(), StatusCode::OK);
   assert_eq!(resp.body_json()["data"], expected);
   ```

3. Routes that bypass the envelope (e.g. OpenAI-compatible endpoints) must
   return `UnconstrainedResponse` with an explanatory comment.

4. To restore old behaviour during a migration window:
   ```toml
   socle = { version = "3", default-features = false, features = ["telemetry", "database", ...] }
   ```

## [2.6.0] — 2026-04-25

### Added

- `created_under(prefix, value)` — composes a 201 Created Location header from
  a route prefix + `value.id()` (requires `api_bones::HasId`). Decouples DTOs
  from HTTP route paths while keeping the call site to one line.
- Bump `api-bones` `4.0.1` → `4.5.0` (adds `HasId`).

## [2.5.1] — 2026-04-25

### Security

- Bump `async-nats` `0.38` → `0.47` to pull in `rustls-webpki >=0.103.13`, fixing [RUSTSEC-2026-0104](https://rustsec.org/advisories/RUSTSEC-2026-0104) (reachable panic in CRL parsing). Affects the `nats` feature only.

## [2.5.0] — 2026-04-25

### Added

- `OrgContextExtractor` — Axum `FromRequestParts` extractor that resolves `OrganizationContext` from a principal extension or `X-Org-Id`/`X-Org-Path` headers, with cross-tenant conflict detection.
- `OrgIsolationLayer` — Tower middleware that short-circuits with `401 Unauthorized` when `OrganizationContext` is absent from request extensions.
- `OrgContextSource` — enum recording which mechanism resolved the org context (`PrincipalClaim` or `Header`).
- `OrgPolicy` trait + `AncestryOrgPolicy` — policy trait for org-scoped access control, with a default ancestry-based implementation.
- `CreatedAtResponse<T>` + `created_at(location, value)` helper — returns `201 Created` with a `Location` header alongside the JSON body, mirroring `ok` / `created` / `listed` ergonomics.
- `AuditLayer` — Tower middleware that captures request/response audit events to a pluggable `AuditSink`, with built-in sinks for tracing and in-memory collection.

## [2.4.0] — 2026-04-24

### Added

- `listed_page<T, U>(items, params)` — ergonomic helper that paginates a fully-loaded `Vec<T>`, maps each item to `U` via `Into`, and returns a `HandlerListResponse<U>`. Eliminates the skip/take/total boilerplate repeated across client-side pagination handlers.

## [2.3.1] — 2026-04-24

### Fixed

- `openapi` feature: add `vendored` to `utoipa-swagger-ui` dependency so Swagger UI assets are bundled at compile time rather than downloaded from GitHub at build time — prevents build failures in air-gapped CI environments.

## [2.3.0] — 2026-04-24

### Fixed

- `EtaggedHandlerResponse<T>` tuple order corrected to `(StatusCode, ETag, Json<ApiResponse<T>>)` — axum 0.8 requires `StatusCode` as the first element; the previous `(ETag, StatusCode, Json<...>)` order caused the `Handler` trait bound to fail at call sites.
- `etagged(etag, value)` return tuple updated to match.

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
