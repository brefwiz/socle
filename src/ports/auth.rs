//! Auth port — extension point for pluggable authentication middleware.
//!
//! Unlike rate-limit, socle ships **no** built-in auth backend. The
//! authentication landscape is too varied (JWT/JWKS, OIDC, OAuth2, API keys,
//! mTLS, custom headers) and a default would either be useless or dangerous.
//!
//! Wrapper crates (e.g. `service-kit`) that need JWT + JWKS, API-key
//! validation, org-context reconciliation, etc. implement [`AuthProvider`]
//! directly:
//!
//! ```rust,no_run
//! use axum::Router;
//! use socle::ports::auth::AuthProvider;
//!
//! struct JwtAuthProvider { /* JWKS cache, issuer, audience, api-key store */ }
//!
//! impl AuthProvider for JwtAuthProvider {
//!     fn apply(&self, router: Router) -> Router {
//!         router.layer(/* your tower layer */)
//!     }
//! }
//! ```
//!
//! The provider is responsible for:
//! - Parsing `Authorization` / `X-Api-Key` headers (or whatever scheme applies)
//! - Validating credentials (JWKS lookup, signature check, expiry, etc.)
//! - Returning the appropriate HTTP response on auth failure (typically 401/403)
//! - Inserting verified identity into request extensions for downstream handlers
//! - Recording span fields (`auth.sub`, `auth.scope`, …) for observability
//!
//! Register with [`crate::ServiceBootstrap::with_auth_provider`].
//!
//! # Layer order
//!
//! The auth layer is applied **after** the rate-limit layer and **before**
//! any extra layers registered via `with_layer`. This means unauthenticated
//! requests are still counted against rate limits (preventing token-brute-force
//! DoS from escaping rate limits), and `with_layer` extensions run either
//! inside or outside auth depending on registration order.

/// Extension point for auth layer injection.
///
/// Implementors receive the fully assembled user router (already wrapped by
/// the rate-limit layer when one is configured) and must return it with the
/// authentication tower layer applied. Tower layers are idiomatically
/// `Clone`, so `&self` is sufficient — the implementor clones any internal
/// state (e.g. `Arc<AuthConfig>`, a `JwksCache`) into the layer.
pub trait AuthProvider: Send + Sync + 'static {
    /// Wrap `router` with an authentication layer and return the result.
    fn apply(&self, router: axum::Router) -> axum::Router;
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Router;

    struct PassthroughAuth;

    impl AuthProvider for PassthroughAuth {
        fn apply(&self, router: Router) -> Router {
            router
        }
    }

    #[test]
    fn provider_can_be_boxed_and_called() {
        let provider: Box<dyn AuthProvider> = Box::new(PassthroughAuth);
        let _ = provider.apply(Router::new());
    }

    struct StatefulAuth {
        _issuer: String,
        _audience: Vec<String>,
    }

    impl AuthProvider for StatefulAuth {
        fn apply(&self, router: Router) -> Router {
            router
        }
    }

    #[test]
    fn provider_supports_arbitrary_state() {
        let provider = StatefulAuth {
            _issuer: "https://accounts.example.com".into(),
            _audience: vec!["my-service".into()],
        };
        let boxed: Box<dyn AuthProvider> = Box::new(provider);
        let _ = boxed.apply(Router::new());
    }
}
