//! Telemetry port — initialisation and shutdown abstraction.
//!
//! Implement [`TelemetryProvider`] to plug in a full OTel SDK (e.g.
//! `otel-bootstrap`) without forking [`crate::ServiceBootstrap::serve`].
//!
//! The built-in provider (`BasicTelemetryProvider`) initialises
//! `tracing_subscriber` from `RUST_LOG` and is used when
//! [`crate::ServiceBootstrap::with_telemetry`] is called without an explicit
//! provider.

use std::future::Future;
use std::pin::Pin;

/// Extension point for telemetry initialisation.
///
/// Wrapper crates supply a concrete implementation; groundwork calls `init` at
/// the start of `serve()` and registers `on_shutdown` as a drain hook after the
/// HTTP server stops.
///
/// ```rust,no_run
/// use groundwork::ports::telemetry::TelemetryProvider;
/// use groundwork::Result;
///
/// struct MyOtelProvider;
///
/// impl TelemetryProvider for MyOtelProvider {
///     fn init(&self, service_name: &str) -> Result<()> {
///         // wire up the OTel SDK …
///         Ok(())
///     }
///
///     fn on_shutdown(&self) -> Pin<Box<dyn Future<Output = ()> + Send>> {
///         Box::pin(async {
///             // flush spans and metrics …
///         })
///     }
/// }
/// ```
pub trait TelemetryProvider: Send + Sync + 'static {
    /// Initialise telemetry for `service_name`. Called once before the HTTP
    /// server starts. Return an error to abort startup.
    fn init(&self, service_name: &str) -> crate::error::Result<()>;

    /// Async drain to run after the HTTP server stops (flush spans, metrics,
    /// etc.). Default is a no-op.
    fn on_shutdown(&self) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        Box::pin(async {})
    }
}

/// Built-in provider — initialises `tracing_subscriber` from `RUST_LOG`.
///
/// Used when [`crate::ServiceBootstrap::with_telemetry`] is called without an
/// explicit provider. Override with
/// [`crate::ServiceBootstrap::with_telemetry_provider`] to plug in OTLP.
pub struct BasicTelemetryProvider;

impl TelemetryProvider for BasicTelemetryProvider {
    fn init(&self, _service_name: &str) -> crate::error::Result<()> {
        crate::adapters::observability::telemetry::init_basic_tracing();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct NoopProvider;

    impl TelemetryProvider for NoopProvider {
        fn init(&self, _: &str) -> crate::error::Result<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn basic_provider_init_is_idempotent() {
        let p = BasicTelemetryProvider;
        assert!(p.init("svc").is_ok());
        assert!(p.init("svc").is_ok());
    }

    #[tokio::test]
    async fn default_on_shutdown_completes() {
        let p = NoopProvider;
        p.on_shutdown().await; // must not hang
    }

    #[tokio::test]
    async fn basic_provider_on_shutdown_is_noop() {
        let p = BasicTelemetryProvider;
        p.on_shutdown().await;
    }
}
