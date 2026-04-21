//! # groundwork
//!
//! Opinionated axum service bootstrap: telemetry, database, rate limiting, and
//! shutdown in one builder. Public open-source facade extracted from an internal
//! service kit.
//!
//! ```rust,no_run
//! use groundwork::{ServiceBootstrap, BootstrapCtx, Result};
//! use axum::{Router, routing::get};
//!
//! # #[tokio::main] async fn main() -> Result<()> {
//! ServiceBootstrap::new("billing-service")
//!     .with_telemetry()
//!     .with_database("postgres://localhost/billing")
//!     .with_router(|_ctx: &BootstrapCtx| Router::new().route("/health", get(|| async { "ok" })))
//!     .serve("0.0.0.0:8080")
//!     .await
//! # }
//! ```

#![allow(clippy::result_large_err)]

// ── Internal modules ──────────────────────────────────────────────────────────

pub(crate) mod adapters;
pub(crate) mod bootstrap;
pub mod ports;

mod config;
mod error;
mod handler_error;
mod request_id;

pub mod etag;
pub mod extract;
pub mod pagination;

#[cfg(feature = "testing")]
pub mod testing;

// ── Public surface ────────────────────────────────────────────────────────────

pub use bootstrap::{BootstrapCtx, ServiceBootstrap, ShutdownHookFn};
pub use config::{BootstrapConfig, CorsConfig, LogFormat, RateLimitConfig, RateLimitKind};
pub use error::{Error, Result};
pub use etag::{ETag, IfMatch, IfNoneMatch, check_if_match, etag_from_updated_at};
pub use handler_error::{
    ApiError, CreatedResult, ErrorCode, HandlerError, HandlerResult, ProblemJson, ValidationError,
};
pub use pagination::{
    CursorPaginatedResponse, CursorPagination, CursorPaginationParams, KeysetPaginatedResponse,
    KeysetPaginationParams, PaginatedResponse, PaginationParams, SortDirection, SortParams,
};

#[cfg(feature = "cursor")]
pub use pagination::{Cursor, CursorError};

#[cfg(feature = "ratelimit")]
pub use adapters::security::rate_limit::{RateLimitBackend, RateLimitExtractor};

#[cfg(feature = "validation")]
pub use extract::Valid;

pub use ports::health::ReadinessCheckFn;
pub use ports::rate_limit::RateLimitProvider;
#[cfg(feature = "telemetry")]
pub use ports::telemetry::{BasicTelemetryProvider, TelemetryProvider};

// ── api-bones re-exports ──────────────────────────────────────────────────────

pub use api_bones::common::{ResourceId, Timestamp};
pub use api_bones::ratelimit::RateLimitInfo;

pub use api_bones::AuditInfo;
pub use api_bones::{ApiResponse, ApiResponseBuilder, ResponseMeta};
pub use api_bones::{BulkItemResult, BulkRequest, BulkResponse};
pub use api_bones::{CorrelationId, CorrelationIdError};
pub use api_bones::{IdempotencyKey, IdempotencyKeyError};
pub use api_bones::{Link, Links};
pub use api_bones::{RequestId, RequestIdParseError};
pub use api_bones::{Slug, SlugError};

/// Generate a `fn main()` for a `generate-openapi` binary in one line.
///
/// # Usage
///
/// ```rust,no_run
/// # #[cfg(feature = "openapi")] {
/// use utoipa::OpenApi;
///
/// #[derive(OpenApi)]
/// #[openapi()]
/// struct ApiDoc;
///
/// groundwork::generate_openapi_binary!(ApiDoc);
/// # }
/// ```
#[cfg(feature = "openapi")]
#[macro_export]
macro_rules! generate_openapi_binary {
    ($api_doc:ty) => {
        $crate::generate_openapi_binary!($api_doc, "/health");
    };
    ($api_doc:ty, $health_path:expr) => {
        fn main() {
            use utoipa::OpenApi as _;
            use $crate::openapi::{merge_health_paths, to_3_0_pretty_json};
            let api = merge_health_paths(<$api_doc>::openapi(), $health_path);
            match to_3_0_pretty_json(api) {
                Ok(json) => println!("{json}"),
                Err(e) => {
                    eprintln!("generate-openapi: serialize failed: {e}");
                    std::process::exit(1);
                }
            }
        }
    };
}

/// OpenAPI helpers for callers generating specs outside of [`ServiceBootstrap::serve`].
#[cfg(feature = "openapi")]
pub mod openapi {
    /// Merge groundwork's built-in health path definitions into `api`.
    pub fn merge_health_paths(
        api: utoipa::openapi::OpenApi,
        health_path: &str,
    ) -> utoipa::openapi::OpenApi {
        crate::adapters::openapi::merge_health_paths(api, health_path)
    }

    /// Rewrite OpenAPI 3.1 nullable type arrays to OpenAPI 3.0 `nullable: true` form.
    pub fn rewrite_nullable_for_progenitor(json: String) -> String {
        crate::adapters::openapi::rewrite_nullable_for_progenitor(json)
    }

    /// Serialize `api` as a valid OpenAPI **3.0.3** pretty-printed JSON string.
    pub fn to_3_0_pretty_json(api: utoipa::openapi::OpenApi) -> serde_json::Result<String> {
        crate::adapters::openapi::to_3_0_pretty_json(api)
    }

    /// Strip `content` from non-2xx response objects in a serialised OpenAPI value.
    ///
    /// Call this explicitly after [`to_3_0_pretty_json`] if desired. This is
    /// opt-in — [`to_3_0_pretty_json`] does **not** call it automatically.
    pub fn strip_non_success_response_content(val: &mut serde_json::Value) {
        crate::adapters::openapi::strip_non_success_response_content(val)
    }

    pub use crate::generate_openapi_binary;

    /// utoipa [`Modify`] plugin that registers a `bearerAuth` HTTP Bearer security scheme.
    pub struct BearerAuthAddon;

    impl utoipa::Modify for BearerAuthAddon {
        fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
            use utoipa::openapi::security::{Http, HttpAuthScheme, SecurityScheme};
            let components = openapi.components.get_or_insert_with(Default::default);
            components.add_security_scheme(
                "bearerAuth",
                SecurityScheme::Http(Http::new(HttpAuthScheme::Bearer)),
            );
        }
    }
}

/// Re-exports of the underlying crates.
pub mod reexports {
    pub use api_bones;
}

#[cfg(test)]
mod tests {
    use crate::error::Error;

    #[test]
    fn error_display_covers_all_variants() {
        assert!(Error::Config("x".into()).to_string().contains("x"));
        assert!(Error::Telemetry("x".into()).to_string().contains("x"));
        assert!(Error::Database("x".into()).to_string().contains("x"));
        assert!(Error::Bind("x".into()).to_string().contains("x"));
        assert!(Error::Serve("x".into()).to_string().contains("x"));
    }

    #[cfg(feature = "telemetry")]
    #[test]
    fn init_basic_tracing_is_idempotent() {
        crate::adapters::observability::telemetry::init_basic_tracing();
        crate::adapters::observability::telemetry::init_basic_tracing();
    }
}
