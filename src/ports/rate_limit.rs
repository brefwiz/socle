//! Rate-limit port — extension point for pluggable rate-limit backends.
//!
//! The built-in backend (`RateLimitBackend`) uses `governor` (GCRA, in-process
//! DashMap store) and is wired in when `ratelimit-memory` is enabled.
//!
//! Wrapper crates (e.g. `service-kit`) that need Postgres, Redis, gossip
//! cluster, or lease modes implement [`RateLimitProvider`] directly:
//!
//! ```rust,no_run
//! use axum::Router;
//! use groundwork::ports::rate_limit::RateLimitProvider;
//!
//! struct DistributedRateLimit { /* distributed-ratelimit config */ }
//!
//! impl RateLimitProvider for DistributedRateLimit {
//!     fn apply(&self, router: Router) -> Router {
//!         router.layer(/* your tower layer */)
//!     }
//! }
//! ```
//!
//! Register with [`crate::ServiceBootstrap::with_rate_limit_provider`].

/// Extension point for rate-limit layer injection.
///
/// Implementors receive the fully assembled user router and must return it
/// with the rate-limit tower layer applied. Tower layers are idiomatically
/// `Clone`, so `&self` is sufficient — the implementor clones any internal
/// state into the layer.
pub trait RateLimitProvider: Send + Sync + 'static {
    /// Wrap `router` with a rate-limit layer and return the result.
    fn apply(&self, router: axum::Router) -> axum::Router;
}

#[cfg(feature = "ratelimit-memory")]
mod memory_impl {
    use super::RateLimitProvider;
    use crate::adapters::security::rate_limit::{
        RateLimitBackend, RateLimitExtractor, RateLimitLayer,
    };

    impl RateLimitProvider for RateLimitBackend {
        fn apply(&self, router: axum::Router) -> axum::Router {
            router.layer(RateLimitLayer::new_memory(
                self.limit,
                self.window_secs,
                RateLimitExtractor::default(),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Router;

    struct PassthroughProvider;

    impl RateLimitProvider for PassthroughProvider {
        fn apply(&self, router: Router) -> Router {
            router
        }
    }

    #[test]
    fn provider_can_be_boxed_and_called() {
        let provider: Box<dyn RateLimitProvider> = Box::new(PassthroughProvider);
        let _ = provider.apply(Router::new());
    }

    #[cfg(feature = "ratelimit-memory")]
    #[test]
    fn rate_limit_backend_implements_provider() {
        use crate::adapters::security::rate_limit::RateLimitBackend;
        let backend = RateLimitBackend {
            limit: 10,
            window_secs: 60,
        };
        let provider: Box<dyn RateLimitProvider> = Box::new(backend);
        let _ = provider.apply(Router::new());
    }
}
