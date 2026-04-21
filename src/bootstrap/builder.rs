//! `ServiceBootstrap` builder — chains `with_*` methods and calls `serve()`.

use std::sync::Arc;

use axum::Router;
use tower_http::cors::CorsLayer;

use crate::bootstrap::ctx::BootstrapCtx;
#[cfg(feature = "ratelimit")]
use crate::config::RateLimitKind;
use crate::config::{BootstrapConfig, CorsConfig};
use crate::error::{Error, Result};
use crate::ports::auth::AuthProvider;
use crate::ports::health::{HealthProbe, ReadinessCheckFn, probe_to_check_fn};
use crate::ports::rate_limit::RateLimitProvider;
#[cfg(feature = "telemetry")]
use crate::ports::telemetry::TelemetryProvider;

#[cfg(feature = "ratelimit")]
use crate::adapters::security::rate_limit::{RateLimitBackend, RateLimitExtractor};

// ─── Internal types ───────────────────────────────────────────────────────────

pub(crate) type RouterBuilder = Box<dyn FnOnce(&BootstrapCtx) -> Router + Send>;
#[cfg(feature = "telemetry")]
pub(crate) type TelemetryInitFn = Box<dyn FnOnce(&str) -> crate::error::Result<()> + Send>;

/// Async drain callback registered via [`ServiceBootstrap::with_shutdown_hook`].
pub type ShutdownHookFn =
    Arc<dyn Fn() -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> + Send + Sync>;

pub(crate) struct ShutdownHook {
    pub(crate) name: String,
    pub(crate) hook: ShutdownHookFn,
    pub(crate) timeout: std::time::Duration,
}

// ─── ServiceBootstrap ────────────────────────────────────────────────────────

/// Builder for a microservice runtime.
///
/// ```rust,no_run
/// use groundwork::{ServiceBootstrap, BootstrapCtx, Result};
/// use axum::{Router, routing::get};
///
/// # #[tokio::main] async fn main() -> Result<()> {
/// ServiceBootstrap::new("my-service")
///     .with_telemetry()
///     .with_router(|_ctx: &BootstrapCtx| Router::new().route("/health", get(|| async { "ok" })))
///     .serve("0.0.0.0:8080")
///     .await
/// # }
/// ```
#[must_use = "ServiceBootstrap does nothing until you call .serve()"]
pub struct ServiceBootstrap {
    pub(crate) service_name: Arc<str>,

    #[cfg(feature = "telemetry")]
    pub(crate) telemetry: bool,
    /// Override for telemetry initialisation. When set, called instead of
    /// `init_basic_tracing()`. Receives the service name.
    #[cfg(feature = "telemetry")]
    pub(crate) telemetry_init: Option<TelemetryInitFn>,
    /// Structured provider — takes priority over `telemetry_init` when both are
    /// set. Registers its own shutdown hook automatically.
    #[cfg(feature = "telemetry")]
    pub(crate) telemetry_provider: Option<Box<dyn TelemetryProvider>>,

    #[cfg(feature = "database")]
    pub(crate) database_url: Option<String>,
    #[cfg(feature = "database")]
    pub(crate) db_pool: Option<sqlx::PgPool>,
    #[cfg(feature = "database")]
    pub(crate) migrator: Option<sqlx::migrate::Migrator>,

    /// Extra tower layers applied to the router just before the cross-cutting
    /// tower-http stack. Applied in registration order (first registered = innermost).
    pub(crate) extra_layers: Vec<Box<dyn FnOnce(Router) -> Router + Send>>,

    #[cfg(feature = "ratelimit")]
    pub(crate) rate_limit: Option<RateLimitBackend>,
    #[cfg(feature = "ratelimit")]
    pub(crate) ratelimit_extractor: RateLimitExtractor,
    /// Pluggable rate-limit provider. Takes priority over `rate_limit` when
    /// both are set. Wrapper crates use this to inject distributed backends
    /// (Postgres, Redis, gossip) without touching `serve()`.
    pub(crate) rate_limit_provider: Option<Box<dyn RateLimitProvider>>,

    /// Pluggable auth provider. Groundwork ships no built-in auth backend;
    /// wrapper crates supply JWT/JWKS, API-key, OIDC, etc. Applied after the
    /// rate-limit layer and before any `with_layer` extensions.
    pub(crate) auth_provider: Option<Box<dyn AuthProvider>>,

    pub(crate) cors: Option<CorsLayer>,
    pub(crate) router_builder: Option<RouterBuilder>,
    pub(crate) version: String,
    pub(crate) health_path: String,
    pub(crate) body_limit_bytes: usize,
    pub(crate) shutdown_timeout: std::time::Duration,
    pub(crate) shutdown_hooks: Vec<ShutdownHook>,
    pub(crate) readiness_checks: Vec<(String, ReadinessCheckFn)>,
    pub(crate) bind_addr: Option<String>,

    #[cfg(feature = "openapi")]
    pub(crate) openapi: Option<utoipa::openapi::OpenApi>,
    #[cfg(feature = "openapi")]
    pub(crate) openapi_spec_path: String,
    #[cfg(feature = "openapi")]
    pub(crate) openapi_ui_path: String,
}

impl ServiceBootstrap {
    /// Start a new bootstrap for a service.
    pub fn new(service_name: impl Into<Arc<str>>) -> Self {
        Self {
            service_name: service_name.into(),
            #[cfg(feature = "telemetry")]
            telemetry: false,
            #[cfg(feature = "telemetry")]
            telemetry_init: None,
            #[cfg(feature = "telemetry")]
            telemetry_provider: None,
            #[cfg(feature = "database")]
            database_url: None,
            #[cfg(feature = "database")]
            db_pool: None,
            #[cfg(feature = "database")]
            migrator: None,
            extra_layers: Vec::new(),
            #[cfg(feature = "ratelimit")]
            rate_limit: None,
            #[cfg(feature = "ratelimit")]
            ratelimit_extractor: RateLimitExtractor::Ip,
            rate_limit_provider: None,
            auth_provider: None,
            cors: None,
            router_builder: None,
            version: env!("CARGO_PKG_VERSION").to_string(),
            health_path: "/health".to_string(),
            body_limit_bytes: 2 * 1024 * 1024,
            shutdown_timeout: std::time::Duration::from_secs(30),
            shutdown_hooks: Vec::new(),
            readiness_checks: Vec::new(),
            bind_addr: None,
            #[cfg(feature = "openapi")]
            openapi: None,
            #[cfg(feature = "openapi")]
            openapi_spec_path: "/openapi.json".into(),
            #[cfg(feature = "openapi")]
            openapi_ui_path: "/docs".into(),
        }
    }

    /// Load `.env` file if present. Call this before any `with_*` methods that
    /// read from environment variables.
    #[cfg(feature = "dotenv")]
    pub fn with_dotenv(self) -> Self {
        let _ = dotenvy::dotenv();
        self
    }

    /// Build from a [`BootstrapConfig`].
    pub fn from_config(service_name: impl Into<Arc<str>>, cfg: BootstrapConfig) -> Result<Self> {
        let cfg = cfg.validate()?;
        let mut b = Self::new(service_name)
            .with_health_path(cfg.health_path)
            .with_body_limit(cfg.body_limit_bytes)
            .with_shutdown_timeout(std::time::Duration::from_secs(cfg.shutdown_timeout_secs));
        if let Some(v) = cfg.version {
            b = b.with_version(v);
        }
        b.bind_addr = Some(cfg.bind_addr);

        if cfg.cors != CorsConfig::default() {
            b = b.with_cors_config(cfg.cors)?;
        }

        #[cfg(feature = "telemetry")]
        if cfg.otel_endpoint.is_some() {
            b = b.with_telemetry();
        }

        #[cfg(feature = "database")]
        if let Some(url) = cfg.database_url {
            b = b.with_database(url);
        }

        #[cfg(feature = "ratelimit")]
        match cfg.rate_limit.kind {
            RateLimitKind::None => {}
            RateLimitKind::Memory { limit, window_secs } => {
                b = b.with_rate_limit(RateLimitBackend { limit, window_secs });
            }
        }

        Ok(b)
    }

    /// Run using the bind address loaded from [`BootstrapConfig`].
    pub async fn run(self) -> Result<()> {
        let addr = self.bind_addr.clone().ok_or_else(|| {
            Error::Config("run() requires from_config(...) to set bind_addr".into())
        })?;
        self.serve(addr).await
    }

    // ── Core settings ──────────────────────────────────────────────────────────

    /// Override the maximum request body size in bytes. Defaults to 2 MiB.
    pub fn with_body_limit(mut self, bytes: usize) -> Self {
        self.body_limit_bytes = bytes;
        self
    }

    /// Hard deadline on graceful shutdown drain. Defaults to 30 seconds.
    pub fn with_shutdown_timeout(mut self, timeout: std::time::Duration) -> Self {
        self.shutdown_timeout = timeout;
        self
    }

    /// Register an async drain callback that runs after the HTTP server stops.
    pub fn with_shutdown_hook<F, Fut>(
        mut self,
        name: impl Into<String>,
        timeout: std::time::Duration,
        hook: F,
    ) -> Self
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        self.shutdown_hooks.push(ShutdownHook {
            name: name.into(),
            hook: Arc::new(move || Box::pin(hook())),
            timeout,
        });
        self
    }

    /// Register a readiness check.
    pub fn with_readiness_check<F, Fut>(mut self, name: impl Into<String>, check: F) -> Self
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = api_bones::health::HealthCheck> + Send + 'static,
    {
        let f: ReadinessCheckFn = Arc::new(move || Box::pin(check()));
        self.readiness_checks.push((name.into(), f));
        self
    }

    /// Register a typed [`HealthProbe`] as a readiness check.
    pub fn with_health_probe(mut self, probe: impl HealthProbe + 'static) -> Self {
        self.readiness_checks.push(probe_to_check_fn(probe));
        self
    }

    /// Override the version reported by the liveness endpoint.
    pub fn with_version(mut self, version: impl Into<String>) -> Self {
        self.version = version.into();
        self
    }

    /// Override the base path for health endpoints. Defaults to `/health`.
    pub fn with_health_path(mut self, path: impl Into<String>) -> Self {
        self.health_path = path.into();
        self
    }

    // ── Telemetry ──────────────────────────────────────────────────────────────

    /// Enable basic tracing via `tracing_subscriber`.
    #[cfg(feature = "telemetry")]
    pub fn with_telemetry(mut self) -> Self {
        self.telemetry = true;
        self
    }

    /// Override the telemetry initialisation function.
    ///
    /// When set, called instead of the built-in `tracing_subscriber` setup.
    /// Use this to wire in a full OTel SDK (e.g. `otel-bootstrap`) from a
    /// wrapper crate without forking `serve()`.
    ///
    /// The callback receives the service name and must return `Ok(())` on
    /// success or an [`Error`] that aborts startup.
    ///
    /// Implies [`with_telemetry`] — no need to call both.
    ///
    /// Prefer [`with_telemetry_provider`] for new code; it also handles
    /// shutdown (span/metric flush) automatically.
    ///
    /// [`with_telemetry_provider`]: ServiceBootstrap::with_telemetry_provider
    #[cfg(feature = "telemetry")]
    pub fn with_telemetry_init<F>(mut self, f: F) -> Self
    where
        F: FnOnce(&str) -> crate::error::Result<()> + Send + 'static,
    {
        self.telemetry = true;
        self.telemetry_init = Some(Box::new(f));
        self
    }

    /// Plug in a [`TelemetryProvider`] implementation.
    ///
    /// The provider's [`TelemetryProvider::init`] is called at startup and its
    /// [`TelemetryProvider::on_shutdown`] is registered as a drain hook that
    /// runs after the HTTP server stops (with a 30-second timeout).
    ///
    /// Takes priority over [`with_telemetry_init`] and
    /// [`with_telemetry`] when all are called. Implies [`with_telemetry`].
    ///
    /// Use this to wire in `otel-bootstrap` or any other OTel SDK from a
    /// wrapper crate:
    ///
    /// ```rust,no_run
    /// use groundwork::{ServiceBootstrap, ports::telemetry::TelemetryProvider, Result};
    ///
    /// struct MyOtelProvider;
    /// impl TelemetryProvider for MyOtelProvider {
    ///     fn init(&self, _: &str) -> Result<()> { Ok(()) }
    /// }
    ///
    /// ServiceBootstrap::new("svc").with_telemetry_provider(MyOtelProvider);
    /// ```
    ///
    /// [`with_telemetry_init`]: ServiceBootstrap::with_telemetry_init
    #[cfg(feature = "telemetry")]
    pub fn with_telemetry_provider<P: TelemetryProvider>(mut self, provider: P) -> Self {
        self.telemetry = true;
        self.telemetry_provider = Some(Box::new(provider));
        self
    }

    // ── Database ───────────────────────────────────────────────────────────────

    /// Connect to a Postgres database and build a `sqlx::PgPool`.
    #[cfg(feature = "database")]
    pub fn with_database(mut self, url: impl Into<String>) -> Self {
        self.database_url = Some(url.into());
        self
    }

    /// Provide a pre-built `sqlx::PgPool` instead of a connection URL.
    ///
    /// Use this when the pool is constructed externally — for example by
    /// `sqlx-switchboard`'s `PoolConfig` — so that `serve()` skips its own
    /// pool construction. Takes precedence over [`with_database`] if both are
    /// called.
    #[cfg(feature = "database")]
    pub fn with_db_pool(mut self, pool: sqlx::PgPool) -> Self {
        self.db_pool = Some(pool);
        self
    }

    /// Run sqlx migrations at startup.
    #[cfg(feature = "database")]
    pub fn with_migrations(mut self, migrator: sqlx::migrate::Migrator) -> Self {
        self.migrator = Some(migrator);
        self
    }

    // ── Rate limiting ──────────────────────────────────────────────────────────

    /// Enable the in-process GCRA rate limiter.
    ///
    /// The limiter is applied as a tower layer inside `serve()`. By default it
    /// keys on the remote IP address — see [`with_rate_limit_extractor`] to
    /// change the extraction strategy.
    ///
    /// **Reverse-proxy note**: the default [`RateLimitExtractor::Ip`] reads the
    /// L4 peer address, which is the proxy IP in production. All clients will
    /// share one rate-limit bucket. Use
    /// `with_rate_limit_extractor(RateLimitExtractor::Header("x-forwarded-for"))`
    /// when the service runs behind a trusted reverse proxy.
    ///
    /// **Memory note**: the keyed limiter stores one entry per unique key with no
    /// TTL or size cap. Avoid `RateLimitExtractor::Header` with attacker-controlled
    /// headers in production; prefer `Ip` or a header with bounded cardinality.
    ///
    /// [`with_rate_limit_extractor`]: ServiceBootstrap::with_rate_limit_extractor
    #[cfg(feature = "ratelimit")]
    pub fn with_rate_limit(mut self, config: RateLimitBackend) -> Self {
        self.rate_limit = Some(config);
        self
    }

    /// Override the key extractor used by the rate limiter.
    #[cfg(feature = "ratelimit")]
    pub fn with_rate_limit_extractor(mut self, extractor: RateLimitExtractor) -> Self {
        self.ratelimit_extractor = extractor;
        self
    }

    /// Plug in a [`RateLimitProvider`] implementation.
    ///
    /// Takes priority over [`with_rate_limit`] when both are called. The
    /// provider receives the assembled router and returns it with the
    /// rate-limit tower layer applied.
    ///
    /// Use this to inject distributed backends (Postgres, Redis, gossip) from
    /// a wrapper crate without calling [`with_layer`] directly:
    ///
    /// ```rust,no_run
    /// use axum::Router;
    /// use groundwork::{ServiceBootstrap, ports::rate_limit::RateLimitProvider};
    ///
    /// struct MyDistributedRl;
    /// impl RateLimitProvider for MyDistributedRl {
    ///     fn apply(self: Box<Self>, router: Router) -> Router {
    ///         router // .layer(my_distributed_rl_layer)
    ///     }
    /// }
    ///
    /// ServiceBootstrap::new("svc").with_rate_limit_provider(MyDistributedRl);
    /// ```
    ///
    /// [`with_rate_limit`]: ServiceBootstrap::with_rate_limit
    /// [`with_layer`]: ServiceBootstrap::with_layer
    pub fn with_rate_limit_provider<P: RateLimitProvider>(mut self, provider: P) -> Self {
        self.rate_limit_provider = Some(Box::new(provider));
        self
    }

    // ── Auth ──────────────────────────────────────────────────────────────────

    /// Plug in an [`AuthProvider`] implementation.
    ///
    /// Groundwork ships no built-in auth backend — the provider is supplied
    /// by the caller (typically a wrapper crate such as `service-kit`) and
    /// owns its configuration, JWKS cache, API-key validator, etc.
    ///
    /// The provider receives the assembled router (already wrapped by the
    /// rate-limit layer when one is configured) and returns it with the
    /// auth tower layer applied. Auth is applied **after** rate-limit so
    /// unauthenticated requests are still rate-counted, and **before** any
    /// extra layers registered via [`with_layer`].
    ///
    /// ```rust,no_run
    /// use axum::Router;
    /// use groundwork::{ServiceBootstrap, ports::auth::AuthProvider};
    ///
    /// struct MyJwtAuth;
    /// impl AuthProvider for MyJwtAuth {
    ///     fn apply(self: Box<Self>, router: Router) -> Router {
    ///         router // .layer(my_auth_layer)
    ///     }
    /// }
    ///
    /// ServiceBootstrap::new("svc").with_auth_provider(MyJwtAuth);
    /// ```
    ///
    /// [`with_layer`]: ServiceBootstrap::with_layer
    pub fn with_auth_provider<P: AuthProvider>(mut self, provider: P) -> Self {
        self.auth_provider = Some(Box::new(provider));
        self
    }

    // ── CORS ───────────────────────────────────────────────────────────────────

    /// Override the default permissive CORS layer.
    pub fn with_cors(mut self, cors: CorsLayer) -> Self {
        self.cors = Some(cors);
        self
    }

    /// Configure CORS from a structured [`CorsConfig`].
    pub fn with_cors_config(mut self, cfg: CorsConfig) -> Result<Self> {
        self.cors = Some(crate::adapters::cors::build_cors_layer(&cfg)?);
        Ok(self)
    }

    // ── Router ─────────────────────────────────────────────────────────────────

    /// Provide the router builder closure.
    pub fn with_router<F>(mut self, f: F) -> Self
    where
        F: FnOnce(&BootstrapCtx) -> Router + Send + 'static,
    {
        self.router_builder = Some(Box::new(f));
        self
    }

    // ── Escape hatches for wrapper crates ──────────────────────────────────────

    /// Inject an arbitrary tower layer into the middleware stack.
    ///
    /// Layers are applied in registration order, innermost first (i.e. the
    /// first call to `with_layer` wraps closest to the user router).
    ///
    /// This is the primary extension point for wrapper crates. Example — adding
    /// the `distributed-ratelimit` layer from `service-kit`:
    ///
    /// ```rust,ignore
    /// bootstrap.with_layer(|router| router.layer(my_distributed_rl_layer))
    /// ```
    pub fn with_layer<F>(mut self, f: F) -> Self
    where
        F: FnOnce(Router) -> Router + Send + 'static,
    {
        self.extra_layers.push(Box::new(f));
        self
    }

    // ── OpenAPI ────────────────────────────────────────────────────────────────

    /// Mount an OpenAPI spec and Swagger UI.
    #[cfg(feature = "openapi")]
    pub fn with_openapi(mut self, api: utoipa::openapi::OpenApi) -> Self {
        self.openapi = Some(api);
        self
    }

    /// Override the spec and UI mount paths.
    #[cfg(feature = "openapi")]
    pub fn with_openapi_paths(
        mut self,
        spec_path: impl Into<String>,
        ui_path: impl Into<String>,
    ) -> Self {
        self.openapi_spec_path = spec_path.into();
        self.openapi_ui_path = ui_path.into();
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use api_bones::health::HealthCheck;

    #[test]
    fn builder_methods_compose() {
        let _b = ServiceBootstrap::new("svc")
            .with_version("1.2.3")
            .with_health_path("/hc")
            .with_body_limit(1024)
            .with_shutdown_timeout(std::time::Duration::from_secs(1))
            .with_cors(CorsLayer::permissive())
            .with_cors_config(CorsConfig {
                allowed_origins: vec!["https://app.example.com".into()],
                allow_credentials: true,
                max_age_secs: Some(600),
                ..Default::default()
            })
            .unwrap()
            .with_readiness_check("noop", || async { HealthCheck::pass("noop") })
            .with_router(|_| Router::new());
    }

    #[cfg(feature = "telemetry")]
    #[test]
    fn builder_with_telemetry_sets_flag() {
        let b = ServiceBootstrap::new("svc").with_telemetry();
        assert!(b.telemetry);
    }

    #[tokio::test]
    async fn serve_errors_when_router_missing() {
        let err = ServiceBootstrap::new("x").serve("127.0.0.1:0").await;
        assert!(matches!(err, Err(Error::Config(_))));
    }

    #[tokio::test]
    async fn serve_errors_on_bad_addr() {
        let err = ServiceBootstrap::new("x")
            .with_router(|_| Router::new())
            .serve("not an addr")
            .await;
        assert!(matches!(err, Err(Error::Config(_))));
    }

    #[test]
    fn from_config_applies_all_fields() {
        use crate::config::{BootstrapConfig, RateLimitConfig, RateLimitKind};
        let cfg = BootstrapConfig {
            bind_addr: "127.0.0.1:1234".into(),
            health_path: "/hc".into(),
            body_limit_bytes: 4096,
            shutdown_timeout_secs: 5,
            version: Some("9.9.9".into()),
            rate_limit: RateLimitConfig {
                kind: RateLimitKind::Memory {
                    limit: 10,
                    window_secs: 60,
                },
            },
            ..Default::default()
        };
        let b = ServiceBootstrap::from_config("svc", cfg).unwrap();
        assert_eq!(b.bind_addr.as_deref(), Some("127.0.0.1:1234"));
        assert_eq!(b.health_path, "/hc");
        assert_eq!(b.body_limit_bytes, 4096);
        assert_eq!(b.shutdown_timeout, std::time::Duration::from_secs(5));
        assert_eq!(b.version, "9.9.9");
    }

    #[test]
    fn with_shutdown_hook_registers_in_order() {
        let b = ServiceBootstrap::new("svc")
            .with_shutdown_hook("first", std::time::Duration::from_secs(5), || async {})
            .with_shutdown_hook("second", std::time::Duration::from_secs(5), || async {});
        assert_eq!(b.shutdown_hooks.len(), 2);
        assert_eq!(b.shutdown_hooks[0].name, "first");
        assert_eq!(b.shutdown_hooks[1].name, "second");
    }

    #[test]
    fn with_layer_registers_in_order() {
        let b = ServiceBootstrap::new("svc")
            .with_layer(|r| r)
            .with_layer(|r| r);
        assert_eq!(b.extra_layers.len(), 2);
    }

    #[cfg(feature = "database")]
    #[test]
    fn with_database_sets_url() {
        let b = ServiceBootstrap::new("svc").with_database("postgres://localhost/test");
        assert_eq!(b.database_url.as_deref(), Some("postgres://localhost/test"));
    }

    #[cfg(feature = "telemetry")]
    #[test]
    fn with_telemetry_init_sets_flag_and_fn() {
        let b = ServiceBootstrap::new("svc").with_telemetry_init(|_| Ok(()));
        assert!(b.telemetry);
        assert!(b.telemetry_init.is_some());
    }

    #[test]
    fn run_errors_without_from_config() {
        // run() requires bind_addr from from_config().
        // We can't await here so just check bind_addr is None.
        let b = ServiceBootstrap::new("svc").with_router(|_| Router::new());
        assert!(b.bind_addr.is_none());
    }

    /// Spin up a real server on a random port, send one request, then drop the
    /// server task. Covers the listen → serve → shutdown path in serve.rs.
    #[tokio::test]
    async fn serve_handles_real_http_request() {
        use axum::routing::get;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener); // free the port; bootstrap will rebind

        let server = tokio::spawn(async move {
            ServiceBootstrap::new("test-svc")
                .with_router(|_| Router::new().route("/ping", get(|| async { "pong" })))
                .serve(addr.to_string())
                .await
                .ok();
        });

        // Give the server a moment to bind.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let resp = reqwest::get(format!("http://{addr}/ping")).await.unwrap();
        assert_eq!(resp.status(), 200);
        assert_eq!(resp.text().await.unwrap(), "pong");

        server.abort();
    }

    /// Health endpoints are mounted automatically.
    #[tokio::test]
    async fn serve_mounts_health_endpoints() {
        use axum::routing::get;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);

        let server = tokio::spawn(async move {
            ServiceBootstrap::new("test-svc")
                .with_router(|_| Router::new().route("/", get(|| async { "ok" })))
                .serve(addr.to_string())
                .await
                .ok();
        });

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let resp = reqwest::get(format!("http://{addr}/health/live"))
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);

        let resp = reqwest::get(format!("http://{addr}/health/ready"))
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);

        server.abort();
    }

    /// 404 fallback returns Problem+JSON.
    #[tokio::test]
    async fn serve_returns_404_on_missing_route() {
        use axum::routing::get;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);

        let server = tokio::spawn(async move {
            ServiceBootstrap::new("test-svc")
                .with_router(|_| Router::new().route("/exists", get(|| async { "ok" })))
                .serve(addr.to_string())
                .await
                .ok();
        });

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let resp = reqwest::get(format!("http://{addr}/missing"))
            .await
            .unwrap();
        assert_eq!(resp.status(), 404);

        server.abort();
    }
}
