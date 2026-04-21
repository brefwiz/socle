//! Telemetry adapter — basic tracing setup.

/// Initialise a `tracing_subscriber` using the `RUST_LOG` env var.
/// Safe to call multiple times; subsequent calls are no-ops.
#[cfg(feature = "telemetry")]
pub(crate) fn init_basic_tracing() {
    use tracing_subscriber::{EnvFilter, fmt};
    let _ = fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .try_init();
}
