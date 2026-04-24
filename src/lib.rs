//! # socle
//!
//! Opinionated axum service bootstrap: telemetry, database, rate limiting, and
//! shutdown in one builder. Public open-source facade extracted from an internal
//! service kit.
//!
//! ```rust,no_run
//! use socle::{ServiceBootstrap, BootstrapCtx, Result};
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

#[cfg(feature = "http-client")]
pub mod http_client;

// ── Public surface ────────────────────────────────────────────────────────────

pub use bootstrap::{BootstrapCtx, ServiceBootstrap, ShutdownHookFn};
pub use config::{BootstrapConfig, CorsConfig, LogFormat, RateLimitConfig, RateLimitKind};
pub use error::{Error, Result};
pub use etag::{ETag, IfMatch, IfNoneMatch, check_if_match, etag_from_updated_at};
pub use handler_error::{
    ApiError, CreatedResponse, ErrorCode, EtaggedHandlerResponse, HandlerError,
    HandlerListResponse, HandlerResponse, ProblemJson, ValidationError, created, listed, ok,
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

pub use ports::auth::AuthProvider;
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
/// socle::generate_openapi_binary!(ApiDoc);
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
            let mut doc = <$api_doc>::openapi();
            merge_health_paths(&mut doc, $health_path);
            match to_3_0_pretty_json(&doc) {
                Ok(json) => println!("{json}"),
                Err(e) => {
                    eprintln!("generate-openapi: serialize failed: {e}");
                    std::process::exit(1);
                }
            }
        }
    };
}

/// OpenAPI 3.0.3 helpers for `axum` + `utoipa` + `progenitor` consumers.
#[cfg(feature = "openapi")]
pub mod openapi;

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
