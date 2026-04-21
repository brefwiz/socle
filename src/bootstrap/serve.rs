//! `serve()` implementation — wires adapters, binds the listener, runs until
//! shutdown, then drains.

use std::future::Future;
use std::net::SocketAddr;

use tower_http::trace::TraceLayer;

use crate::bootstrap::builder::{ServiceBootstrap, ShutdownHook};
use crate::bootstrap::ctx::BootstrapCtx;
use crate::error::{Error, Result};

impl ServiceBootstrap {
    /// Run the service. Initialises every enabled integration in dependency
    /// order, binds the listener, serves until SIGINT/SIGTERM, then drains.
    pub async fn serve(self, addr: impl Into<String>) -> Result<()> {
        let addr: SocketAddr = addr
            .into()
            .parse()
            .map_err(|e: std::net::AddrParseError| Error::Config(e.to_string()))?;
        let listener = tokio::net::TcpListener::bind(addr)
            .await
            .map_err(|e| Error::Bind(e.to_string()))?;
        tracing::info!(%addr, service = %self.service_name, "groundwork: listening");
        self.serve_with_shutdown(listener, shutdown_signal()).await
    }

    /// Run the service using a pre-bound listener and a caller-supplied shutdown
    /// future. Useful for integration tests where you need to bind on port 0
    /// and control when the server stops.
    ///
    /// ```rust,no_run
    /// use axum::{Router, routing::get};
    /// use groundwork::{BootstrapCtx, Result, ServiceBootstrap};
    /// use tokio::net::TcpListener;
    ///
    /// # #[tokio::main] async fn main() -> Result<()> {
    /// let listener = TcpListener::bind("127.0.0.1:0").await?;
    /// let addr = listener.local_addr()?;
    /// ServiceBootstrap::new("my-service")
    ///     .with_router(|_: &BootstrapCtx| Router::new().route("/", get(|| async { "ok" })))
    ///     .serve_with_shutdown(listener, std::future::pending())
    ///     .await
    /// # }
    /// ```
    pub async fn serve_with_shutdown(
        self,
        listener: tokio::net::TcpListener,
        shutdown: impl Future<Output = ()> + Send + 'static,
    ) -> Result<()> {
        // Destructure early so we can mutate shutdown_hooks without partial-move issues.
        let service_name = self.service_name;
        #[cfg_attr(not(feature = "telemetry"), allow(unused_mut))]
        let mut shutdown_hooks = self.shutdown_hooks;
        let shutdown_timeout = self.shutdown_timeout;
        let extra_layers = self.extra_layers;
        let rate_limit_provider = self.rate_limit_provider;
        let auth_provider = self.auth_provider;
        let cors = self.cors;
        let router_builder = self.router_builder;
        let version = self.version;
        let health_path = self.health_path;
        let body_limit_bytes = self.body_limit_bytes;
        let readiness_checks = self.readiness_checks;

        #[cfg(feature = "database")]
        let database_url = self.database_url;
        #[cfg(feature = "database")]
        let db_pool = self.db_pool;
        #[cfg(feature = "database")]
        let migrator = self.migrator;

        #[cfg(feature = "ratelimit")]
        let rate_limit = self.rate_limit;
        #[cfg(feature = "ratelimit")]
        let ratelimit_extractor = self.ratelimit_extractor;

        #[cfg(feature = "openapi")]
        let openapi = self.openapi;
        #[cfg(feature = "openapi")]
        let openapi_spec_path = self.openapi_spec_path;
        #[cfg(feature = "openapi")]
        let openapi_ui_path = self.openapi_ui_path;

        #[cfg(feature = "telemetry")]
        let telemetry_enabled = self.telemetry;
        #[cfg(feature = "telemetry")]
        let telemetry_provider = self.telemetry_provider;
        #[cfg(feature = "telemetry")]
        let telemetry_init = self.telemetry_init;

        // 1. Telemetry first — priority: provider > init_fn > builtin.
        #[cfg(feature = "telemetry")]
        if telemetry_enabled {
            if let Some(provider) = telemetry_provider {
                provider
                    .init(&service_name)
                    .map_err(|e| Error::Telemetry(e.to_string()))?;
                // Register the provider's flush as the last drain hook so it
                // runs after all user-registered hooks have completed.
                let provider = std::sync::Arc::new(provider);
                let hook: crate::bootstrap::builder::ShutdownHookFn =
                    std::sync::Arc::new(move || {
                        let p = provider.clone();
                        Box::pin(async move { p.on_shutdown().await })
                    });
                shutdown_hooks.push(ShutdownHook {
                    name: "telemetry-flush".into(),
                    hook,
                    timeout: std::time::Duration::from_secs(30),
                });
            } else {
                match telemetry_init {
                    Some(init_fn) => {
                        init_fn(&service_name).map_err(|e| Error::Telemetry(e.to_string()))?
                    }
                    None => crate::adapters::observability::telemetry::init_basic_tracing(),
                }
            }
        }

        // 2. Database pool — prefer pre-built pool over URL construction.
        #[cfg(feature = "database")]
        let db: Option<sqlx::PgPool> = if let Some(pool) = db_pool {
            if let Some(ref migrator) = migrator {
                tracing::warn!(
                    service = %service_name,
                    "groundwork: running migrations in-process"
                );
                migrator
                    .run(&pool)
                    .await
                    .map_err(|e| Error::Database(format!("migrate: {e}")))?;
                tracing::info!("groundwork: migrations applied successfully");
            }
            Some(pool)
        } else if let Some(ref url) = database_url {
            let pool = sqlx::PgPool::connect(url)
                .await
                .map_err(|e| Error::Database(e.to_string()))?;

            if let Some(ref migrator) = migrator {
                tracing::warn!(
                    service = %service_name,
                    "groundwork: running migrations in-process"
                );
                migrator
                    .run(&pool)
                    .await
                    .map_err(|e| Error::Database(format!("migrate: {e}")))?;
                tracing::info!("groundwork: migrations applied successfully");
            }

            Some(pool)
        } else if migrator.is_some() {
            return Err(Error::Config(
                "with_migrations(...) requires with_database(...) to be called first".into(),
            ));
        } else {
            None
        };

        // 3. Build the user router via ctx.
        let ctx = BootstrapCtx {
            service_name: service_name.clone(),
            #[cfg(feature = "database")]
            db: db.clone(),
            extensions: std::collections::HashMap::new(),
        };

        let user_router = router_builder
            .ok_or_else(|| Error::Config("with_router(...) was never called".into()))?(
            &ctx
        );

        // 4. Mount health endpoints.
        let health_router = crate::adapters::health::build_health_router(
            &health_path,
            &service_name,
            &version,
            readiness_checks.clone(),
        );
        #[cfg_attr(not(feature = "openapi"), allow(unused_mut))]
        let mut user_router = user_router.merge(health_router);

        // OpenAPI spec + Swagger UI.
        #[cfg(feature = "openapi")]
        if let Some(mut api) = openapi.clone() {
            api = crate::adapters::openapi::merge_health_paths(api, &health_path);
            user_router = crate::adapters::openapi::mount_openapi(
                user_router,
                api,
                &openapi_spec_path,
                &openapi_ui_path,
            );
        }

        let user_router = user_router.fallback(crate::adapters::health::not_found_fallback);

        // 5. Apply layers.
        let mut app = user_router;

        // Rate limit — priority: provider > built-in memory backend.
        if let Some(provider) = rate_limit_provider {
            app = provider.apply(app);
        } else {
            #[cfg(feature = "ratelimit-memory")]
            if let Some(cfg) = rate_limit {
                use crate::adapters::security::rate_limit::RateLimitLayer;
                app = app.layer(RateLimitLayer::new_memory(
                    cfg.limit,
                    cfg.window_secs,
                    ratelimit_extractor,
                ));
            }
        }

        // Auth — applied after rate-limit (unauthenticated requests still
        // counted) and before extra_layers.
        if let Some(provider) = auth_provider {
            app = provider.apply(app);
        }

        // Extra layers registered via with_layer() — applied innermost first.
        for layer_fn in extra_layers {
            app = layer_fn(app);
        }

        // Enrich bare error responses.
        app = app.layer(axum::middleware::from_fn(
            crate::adapters::security::enrich_error::enrich_error_response,
        ));

        // Cross-cutting tower-http layers.
        use tower_http::catch_panic::CatchPanicLayer;
        use tower_http::compression::CompressionLayer;
        use tower_http::limit::RequestBodyLimitLayer;
        use tower_http::request_id::{PropagateRequestIdLayer, SetRequestIdLayer};

        let request_id_header = axum::http::HeaderName::from_static("x-request-id");

        let trace_layer =
            TraceLayer::new_for_http().make_span_with(|req: &axum::http::Request<_>| {
                let request_id = crate::request_id::extract_request_id(req);
                tracing::info_span!(
                    "request",
                    method = %req.method(),
                    uri = %req.uri(),
                    "request.id" = request_id,
                )
            });

        // CORS is opt-in. Omitting with_cors_config() means no CORS headers are
        // sent, which is safe for APIs not accessed from browsers.
        if let Some(cors) = cors {
            app = app.layer(cors);
        }

        app = app
            .layer(CompressionLayer::new())
            .layer(RequestBodyLimitLayer::new(body_limit_bytes))
            .layer(CatchPanicLayer::custom(crate::handler_error::panic_handler))
            .layer(trace_layer)
            .layer(PropagateRequestIdLayer::new(request_id_header.clone()))
            .layer(crate::request_id::RequestIdTaskLocalLayer)
            .layer(SetRequestIdLayer::new(
                request_id_header,
                crate::request_id::MakeRequestUuidV7,
            ));

        // 6. Serve with caller-supplied shutdown signal.
        let make_service = app.into_make_service_with_connect_info::<std::net::SocketAddr>();
        let server = axum::serve(listener, make_service).with_graceful_shutdown(shutdown);

        server.await.map_err(|e| Error::Serve(e.to_string()))?;

        run_shutdown_hooks(shutdown_hooks, shutdown_timeout).await;
        tracing::info!(service = %service_name, "groundwork: shutdown complete");
        Ok(())
    }
}

async fn run_shutdown_hooks(hooks: Vec<ShutdownHook>, _default_timeout: std::time::Duration) {
    for hook in hooks.into_iter().rev() {
        tracing::info!(hook = %hook.name, "groundwork: running shutdown hook");
        match tokio::time::timeout(hook.timeout, (hook.hook)()).await {
            Ok(()) => tracing::info!(hook = %hook.name, "groundwork: shutdown hook completed"),
            Err(_) => tracing::error!(
                hook = %hook.name,
                timeout_secs = hook.timeout.as_secs(),
                "groundwork: shutdown hook timed out"
            ),
        }
    }
}

pub(crate) async fn shutdown_signal() {
    use tokio::signal;
    let ctrl_c = async {
        signal::ctrl_c().await.ok();
    };
    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut sig) = signal::unix::signal(signal::unix::SignalKind::terminate()) {
            sig.recv().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}
