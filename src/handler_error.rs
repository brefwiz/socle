//! Handler-level error type and RFC-enforced response types.
//!
//! # Response types
//!
//! With the `rfc-types` feature (default), every handler success arm is a
//! sealed [`RfcOk<T>`] value.  It can only be constructed via the provided
//! builder functions and always produces an `ApiResponse<T>` JSON body —
//! enforcing ADR-0016 and ADR-0020 at compile time.
//!
//! ## Builder functions
//!
//! | Function | Status | Adds |
//! |---|---|---|
//! | [`ok`] | 200 | — |
//! | [`created`] | 201 | — |
//! | [`created_at`] | 201 | `Location` |
//! | [`created_under`] | 201 | `Location` (from `HasId`) |
//! | [`etagged`] | 200 | `ETag` |
//! | [`listed`] | 200 | — (paginated body) |
//! | [`listed_page`] | 200 | — (paginates a `Vec`) |
//!
//! ## Opt-out
//!
//! For routes where the wire format is externally mandated (e.g. `OpenAI`-
//! compatible data-plane endpoints), return [`UnconstrainedResponse`] and
//! document the product-level reason at the call site.

pub use api_bones::error::{ApiError, ErrorCode, ProblemJson, ValidationError};
use axum::response::{IntoResponse, Response};

// ── HandlerError ─────────────────────────────────────────────────────────────

/// Error type for axum handlers. Wraps [`ApiError`] and implements [`IntoResponse`].
///
/// Serializes as RFC 9457 Problem Details with `Content-Type: application/problem+json`.
#[derive(Debug)]
pub struct HandlerError(pub ApiError);

impl HandlerError {
    /// Create a new handler error with the given error code and detail message.
    pub fn new(code: ErrorCode, detail: impl Into<String>) -> Self {
        Self(ApiError::new(code, detail))
    }

    /// Add a request ID to the error (serialized as `instance` field in RFC 9457).
    #[must_use]
    pub fn with_request_id(mut self, id: uuid::Uuid) -> Self {
        self.0 = self.0.with_request_id(id);
        self
    }

    /// Add validation errors to the error response.
    #[must_use]
    pub fn with_errors(mut self, errors: Vec<ValidationError>) -> Self {
        self.0 = self.0.with_errors(errors);
        self
    }

    /// Create from a sqlx error.
    #[cfg(feature = "database")]
    pub fn from_sqlx(err: &sqlx::Error) -> Self {
        match err {
            sqlx::Error::RowNotFound => {
                Self::new(ErrorCode::ResourceNotFound, "resource not found")
            }
            sqlx::Error::Database(db_err) => {
                if db_err.code().as_deref() == Some("23505") {
                    Self::new(ErrorCode::ResourceAlreadyExists, "resource already exists")
                } else {
                    tracing::error!(error = %err, "database error");
                    Self::new(ErrorCode::InternalServerError, "internal server error")
                }
            }
            _ => {
                tracing::error!(error = %err, "database error");
                Self::new(ErrorCode::InternalServerError, "internal server error")
            }
        }
    }
}

impl From<ApiError> for HandlerError {
    fn from(e: ApiError) -> Self {
        Self(e)
    }
}

impl IntoResponse for HandlerError {
    fn into_response(self) -> Response {
        ProblemJson::from(self.0).into_response()
    }
}

// ── UnconstrainedResponse — always available ──────────────────────────────────

/// Explicit opt-out from RFC-oriented response enforcement.
///
/// Handlers returning this type are intentionally exempt from the
/// [`ApiResponse`](api_bones::ApiResponse) envelope and RFC 9457 error shape.
/// **Every use must be accompanied by a comment explaining the product-level
/// constraint that makes the standard types impossible.**
///
/// # Legitimate uses
///
/// - **OpenAI-compatible data-plane endpoints** (e.g. routewiz `/v1/chat/completions`):
///   the wire format is mandated by client tooling for drop-in compatibility.
/// - **Webhook delivery** where the response shape is contractually fixed by a
///   third-party specification.
///
/// For every other route, use the standard response type aliases and builder
/// functions ([`ok`], [`created`], [`created_at`], [`created_under`],
/// [`listed`], [`listed_page`], [`etagged`]).
///
/// # Example
///
/// ```rust,no_run
/// use axum::{Json, http::StatusCode};
/// use socle::UnconstrainedResponse;
/// use serde_json::{json, Value};
///
/// // PRODUCT CONSTRAINT: /v1/chat/completions must be wire-compatible with the
/// // OpenAI API so that existing client SDKs work without modification.
/// // RFC 9457 Problem+JSON responses would break those clients.
/// async fn chat_completions() -> UnconstrainedResponse {
///     UnconstrainedResponse::new((StatusCode::OK, Json(json!({"id": "chatcmpl-123"}))))
/// }
/// ```
pub struct UnconstrainedResponse(Response);

impl UnconstrainedResponse {
    /// Wrap any [`IntoResponse`] value, bypassing the RFC envelope check.
    pub fn new(r: impl IntoResponse) -> Self {
        Self(r.into_response())
    }
}

impl IntoResponse for UnconstrainedResponse {
    fn into_response(self) -> Response {
        self.0
    }
}

// ── RfcOk — sealed success type (rfc-types feature) ──────────────────────────

#[cfg(feature = "rfc-types")]
pub use rfc_ok::RfcOk;

#[cfg(feature = "rfc-types")]
mod rfc_ok {
    use axum::{
        body::Body,
        http::{HeaderMap, HeaderValue, StatusCode, header},
        response::{IntoResponse, Response},
    };
    use std::marker::PhantomData;

    /// Sealed success response for RFC-compliant handlers.
    ///
    /// The body is always `ApiResponse<T>` JSON; the status code and any
    /// supplemental headers (e.g. `Location`, `ETag`) are set by the builder
    /// function that produced this value.
    ///
    /// **You cannot construct this type directly.** Use the builder functions
    /// in the parent module: [`ok`](super::ok), [`created`](super::created),
    /// [`created_at`](super::created_at), [`created_under`](super::created_under),
    /// [`etagged`](super::etagged), [`listed`](super::listed),
    /// [`listed_page`](super::listed_page).
    ///
    /// `T` is the payload type (`ApiResponse<T>` is the wire envelope).
    /// For list responses the payload is `PaginatedResponse<Item>`, so
    /// `HandlerListResponse<Item>` expands to
    /// `Result<RfcOk<PaginatedResponse<Item>>, HandlerError>`.
    pub struct RfcOk<T> {
        pub(super) status: StatusCode,
        pub(super) headers: HeaderMap,
        /// Eagerly-serialized JSON body. `T` is phantom-only after construction.
        pub(super) body: Vec<u8>,
        pub(super) _data: PhantomData<fn() -> T>,
    }

    impl<T> RfcOk<T> {
        pub(super) fn new(status: StatusCode, headers: HeaderMap, body: Vec<u8>) -> Self {
            Self {
                status,
                headers,
                body,
                _data: PhantomData,
            }
        }

        /// HTTP status code of this response.
        #[must_use]
        pub fn status(&self) -> StatusCode {
            self.status
        }

        /// Response headers (e.g. `Location`, `ETag`).
        #[must_use]
        pub fn headers(&self) -> &HeaderMap {
            &self.headers
        }

        /// Parse the serialized body as a JSON value.
        ///
        /// Useful for assertions in unit tests.
        ///
        /// # Panics
        ///
        /// Panics if the body is not valid JSON (which cannot happen for responses built by this crate).
        #[must_use]
        pub fn body_json(&self) -> serde_json::Value {
            serde_json::from_slice(&self.body).expect("body is always valid JSON")
        }
    }

    // T is phantom-only (body is Vec<u8>) and PhantomData<fn() -> T> is always
    // Send + Sync, so no manual impls are required.

    impl<T> IntoResponse for RfcOk<T> {
        fn into_response(self) -> Response {
            let mut resp = Response::new(Body::from(self.body));
            *resp.status_mut() = self.status;
            *resp.headers_mut() = self.headers;
            resp.headers_mut().insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            );
            resp
        }
    }
}

// ── Type aliases ──────────────────────────────────────────────────────────────

#[cfg(feature = "rfc-types")]
mod type_aliases {
    use super::{HandlerError, RfcOk};
    use api_bones::PaginatedResponse;

    /// Return type for handlers that return a single resource (200 OK).
    pub type HandlerResponse<T> = Result<RfcOk<T>, HandlerError>;

    /// Return type for handlers that return a paginated collection (200 OK).
    pub type HandlerListResponse<T> = Result<RfcOk<PaginatedResponse<T>>, HandlerError>;

    /// Return type for handlers that create a resource (201 Created).
    pub type CreatedResponse<T> = Result<RfcOk<T>, HandlerError>;

    /// Return type for handlers that create a resource with a `Location` header (201 Created).
    pub type CreatedAtResponse<T> = Result<RfcOk<T>, HandlerError>;

    /// Return type for read/update handlers that carry an `ETag` response header (200 OK).
    pub type EtaggedHandlerResponse<T> = Result<RfcOk<T>, HandlerError>;
}

#[cfg(not(feature = "rfc-types"))]
mod type_aliases {
    use super::HandlerError;

    /// Return type for handlers that return a single resource (200 OK).
    pub type HandlerResponse<T> = Result<
        (
            axum::http::StatusCode,
            axum::Json<api_bones::ApiResponse<T>>,
        ),
        HandlerError,
    >;

    /// Return type for handlers that return a paginated collection (200 OK).
    pub type HandlerListResponse<T> =
        Result<axum::Json<api_bones::ApiResponse<api_bones::PaginatedResponse<T>>>, HandlerError>;

    /// Return type for handlers that create a resource (201 Created).
    pub type CreatedResponse<T> = Result<
        (
            axum::http::StatusCode,
            axum::Json<api_bones::ApiResponse<T>>,
        ),
        HandlerError,
    >;

    /// Return type for handlers that create a resource with a `Location` header (201 Created).
    pub type CreatedAtResponse<T> = Result<
        (
            axum::http::StatusCode,
            axum::http::HeaderMap,
            axum::Json<api_bones::ApiResponse<T>>,
        ),
        HandlerError,
    >;

    /// Return type for read/update handlers that carry an `ETag` response header (200 OK).
    pub type EtaggedHandlerResponse<T> = Result<
        (
            axum::http::StatusCode,
            api_bones::etag::ETag,
            axum::Json<api_bones::ApiResponse<T>>,
        ),
        HandlerError,
    >;
}

pub use type_aliases::{
    CreatedAtResponse, CreatedResponse, EtaggedHandlerResponse, HandlerListResponse,
    HandlerResponse,
};

// ── Builder functions ─────────────────────────────────────────────────────────

/// Build the success response for a [`CreatedResponse`] handler (201 Created).
///
/// # Errors
///
/// Never returns `Err`; the `Result` wrapper exists for `?`-ergonomics in handlers.
///
/// # Panics
///
/// Panics if `T` fails to serialize (not possible for valid `serde::Serialize` impls).
#[cfg(feature = "rfc-types")]
pub fn created<T: serde::Serialize>(value: T) -> CreatedResponse<T> {
    let body = serde_json::to_vec(&api_bones::ApiResponse::builder(value).build())
        .expect("ApiResponse<T> is always serializable");
    Ok(RfcOk::new(
        axum::http::StatusCode::CREATED,
        axum::http::HeaderMap::new(),
        body,
    ))
}

/// # Errors
///
/// Never returns `Err`; the `Result` wrapper exists for `?`-ergonomics in handlers.
#[cfg(not(feature = "rfc-types"))]
pub fn created<T>(value: T) -> CreatedResponse<T> {
    Ok((
        axum::http::StatusCode::CREATED,
        axum::Json(api_bones::ApiResponse::builder(value).build()),
    ))
}

/// Build the success response for a [`CreatedAtResponse`] handler (201 Created + `Location`).
///
/// # Errors
///
/// Never returns `Err`; the `Result` wrapper exists for `?`-ergonomics in handlers.
///
/// # Panics
///
/// Panics if `location` is not a valid header value, or if `T` fails to serialize.
#[cfg(feature = "rfc-types")]
pub fn created_at<T: serde::Serialize>(location: &str, value: T) -> CreatedAtResponse<T> {
    let mut headers = axum::http::HeaderMap::new();
    headers.insert(
        axum::http::header::LOCATION,
        location.parse().expect("valid Location URI"),
    );
    let body = serde_json::to_vec(&api_bones::ApiResponse::builder(value).build())
        .expect("ApiResponse<T> is always serializable");
    Ok(RfcOk::new(axum::http::StatusCode::CREATED, headers, body))
}

/// # Errors
///
/// Never returns `Err`; the `Result` wrapper exists for `?`-ergonomics in handlers.
///
/// # Panics
///
/// Panics if `location` is not a valid header value.
#[cfg(not(feature = "rfc-types"))]
pub fn created_at<T>(location: &str, value: T) -> CreatedAtResponse<T> {
    let mut headers = axum::http::HeaderMap::new();
    headers.insert(
        axum::http::header::LOCATION,
        location.parse().expect("valid Location URI"),
    );
    Ok((
        axum::http::StatusCode::CREATED,
        headers,
        axum::Json(api_bones::ApiResponse::builder(value).build()),
    ))
}

/// Build a 201 Created response whose `Location` header is composed from
/// `prefix` + `/` + `value.id()`.
///
/// The prefix is typically the route path (e.g. `"/v1/items"`); the id comes
/// from the value's [`HasId`](api_bones::HasId) impl.  Trailing slashes on
/// `prefix` are trimmed.
///
/// # Errors
///
/// Never returns `Err`; the `Result` wrapper exists for `?`-ergonomics in handlers.
#[cfg(feature = "rfc-types")]
pub fn created_under<T: api_bones::HasId + serde::Serialize>(
    prefix: &str,
    value: T,
) -> CreatedAtResponse<T> {
    let location = format!("{}/{}", prefix.trim_end_matches('/'), value.id());
    created_at(&location, value)
}

/// # Errors
///
/// Never returns `Err`; the `Result` wrapper exists for `?`-ergonomics in handlers.
#[cfg(not(feature = "rfc-types"))]
pub fn created_under<T: api_bones::HasId>(prefix: &str, value: T) -> CreatedAtResponse<T> {
    let location = format!("{}/{}", prefix.trim_end_matches('/'), value.id());
    created_at(&location, value)
}

/// Build the success response for a [`HandlerResponse`] handler (200 OK).
///
/// # Errors
///
/// Never returns `Err`; the `Result` wrapper exists for `?`-ergonomics in handlers.
///
/// # Panics
///
/// Panics if `T` fails to serialize (not possible for valid `serde::Serialize` impls).
#[cfg(feature = "rfc-types")]
pub fn ok<T: serde::Serialize>(value: T) -> HandlerResponse<T> {
    let body = serde_json::to_vec(&api_bones::ApiResponse::builder(value).build())
        .expect("ApiResponse<T> is always serializable");
    Ok(RfcOk::new(
        axum::http::StatusCode::OK,
        axum::http::HeaderMap::new(),
        body,
    ))
}

/// # Errors
///
/// Never returns `Err`; the `Result` wrapper exists for `?`-ergonomics in handlers.
#[cfg(not(feature = "rfc-types"))]
pub fn ok<T>(value: T) -> HandlerResponse<T> {
    Ok((
        axum::http::StatusCode::OK,
        axum::Json(api_bones::ApiResponse::builder(value).build()),
    ))
}

/// Build the success response for a [`HandlerListResponse`] handler.
///
/// # Errors
///
/// Never returns `Err`; the `Result` wrapper exists for `?`-ergonomics in handlers.
///
/// # Panics
///
/// Panics if `T` fails to serialize (not possible for valid `serde::Serialize` impls).
#[cfg(feature = "rfc-types")]
pub fn listed<T: serde::Serialize>(
    page: api_bones::PaginatedResponse<T>,
) -> HandlerListResponse<T> {
    let body = serde_json::to_vec(&api_bones::ApiResponse::builder(page).build())
        .expect("ApiResponse<PaginatedResponse<T>> is always serializable");
    Ok(RfcOk::new(
        axum::http::StatusCode::OK,
        axum::http::HeaderMap::new(),
        body,
    ))
}

/// # Errors
///
/// Never returns `Err`; the `Result` wrapper exists for `?`-ergonomics in handlers.
#[cfg(not(feature = "rfc-types"))]
pub fn listed<T>(page: api_bones::PaginatedResponse<T>) -> HandlerListResponse<T> {
    Ok(axum::Json(api_bones::ApiResponse::builder(page).build()))
}

/// Paginate a fully-loaded `Vec<T>`, map each item to `U`, and return a [`HandlerListResponse`].
///
/// # Errors
///
/// Never returns `Err`; the `Result` wrapper exists for `?`-ergonomics in handlers.
///
/// Combines the common client-side pagination boilerplate (skip/take + total) into one call:
///
/// ```rust,no_run
/// use socle::{HandlerListResponse, listed_page};
/// use socle::pagination::PaginationParams;
///
/// async fn list_items(params: PaginationParams) -> HandlerListResponse<String> {
///     let all: Vec<String> = vec!["a".into(), "b".into()];
///     listed_page(all, &params)
/// }
/// ```
pub fn listed_page<T, U>(
    items: Vec<T>,
    params: &api_bones::pagination::PaginationParams,
) -> HandlerListResponse<U>
where
    T: Into<U>,
    U: serde::Serialize,
{
    let total = items.len() as u64;
    let page: Vec<U> = items
        .into_iter()
        .skip(usize::try_from(params.offset.unwrap_or(0)).unwrap_or(usize::MAX))
        .take(usize::try_from(params.limit.unwrap_or(20)).unwrap_or(usize::MAX))
        .map(Into::into)
        .collect();
    listed(api_bones::PaginatedResponse::new(page, total, params))
}

/// Build the success response for an [`EtaggedHandlerResponse`] handler (200 OK + `ETag`).
///
/// # Errors
///
/// Never returns `Err`; the `Result` wrapper exists for `?`-ergonomics in handlers.
///
/// # Panics
///
/// Panics if `T` fails to serialize (not possible for valid `serde::Serialize` impls).
#[cfg(feature = "rfc-types")]
pub fn etagged<T: serde::Serialize>(
    etag: &api_bones::etag::ETag,
    value: T,
) -> EtaggedHandlerResponse<T> {
    let mut headers = axum::http::HeaderMap::new();
    headers.insert(
        axum::http::header::ETAG,
        axum::http::HeaderValue::from_str(&etag.to_string())
            .expect("ETag is always a valid header value"),
    );
    let body = serde_json::to_vec(&api_bones::ApiResponse::builder(value).build())
        .expect("ApiResponse<T> is always serializable");
    Ok(RfcOk::new(axum::http::StatusCode::OK, headers, body))
}

/// # Errors
///
/// Never returns `Err`; the `Result` wrapper exists for `?`-ergonomics in handlers.
#[cfg(not(feature = "rfc-types"))]
pub fn etagged<T>(etag: &api_bones::etag::ETag, value: T) -> EtaggedHandlerResponse<T> {
    Ok((
        axum::http::StatusCode::OK,
        etag.clone(),
        axum::Json(api_bones::ApiResponse::builder(value).build()),
    ))
}

// ── Panic handler ─────────────────────────────────────────────────────────────

/// Build a Problem+JSON 500 response from a panic payload. Used in catch-panic layer.
#[allow(clippy::needless_pass_by_value)]
pub(crate) fn panic_handler(err: Box<dyn std::any::Any + Send + 'static>) -> Response {
    let detail = if let Some(s) = err.downcast_ref::<String>() {
        s.as_str()
    } else if let Some(s) = err.downcast_ref::<&str>() {
        s
    } else {
        "panic"
    };
    tracing::error!(panic = detail, "handler panicked");
    HandlerError::new(ErrorCode::InternalServerError, "internal server error").into_response()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handler_error_into_response_returns_problem_json() {
        let err = HandlerError::new(ErrorCode::ResourceNotFound, "not found");
        let resp = err.into_response();
        assert_eq!(resp.status(), 404);
    }

    #[test]
    fn handler_error_from_api_error() {
        let api_err = ApiError::new(ErrorCode::InternalServerError, "oops");
        let handler_err = HandlerError::from(api_err);
        let resp = handler_err.into_response();
        assert_eq!(resp.status(), 500);
    }

    #[test]
    fn with_request_id_and_errors() {
        let id = uuid::Uuid::now_v7();
        let err = HandlerError::new(ErrorCode::ValidationFailed, "bad input")
            .with_request_id(id)
            .with_errors(vec![ValidationError {
                field: "name".into(),
                message: "required".into(),
                rule: None,
            }]);
        let resp = err.into_response();
        assert_eq!(resp.status(), 400);
    }

    #[test]
    fn panic_handler_downcasts_string_payload() {
        let payload: Box<dyn std::any::Any + Send + 'static> = Box::new("boom".to_string());
        let resp = panic_handler(payload);
        assert_eq!(resp.status(), 500);
    }

    #[test]
    fn panic_handler_downcasts_static_str_payload() {
        let payload: Box<dyn std::any::Any + Send + 'static> = Box::new("static boom");
        let resp = panic_handler(payload);
        assert_eq!(resp.status(), 500);
    }

    #[test]
    fn panic_handler_handles_unknown_payload() {
        let payload: Box<dyn std::any::Any + Send + 'static> = Box::new(42u32);
        let resp = panic_handler(payload);
        assert_eq!(resp.status(), 500);
    }

    // Builder function tests use RfcOk accessors; only compiled with rfc-types.
    #[cfg(feature = "rfc-types")]
    mod rfc {
        use super::*;

        #[test]
        fn created_builds_201_with_envelope() {
            let resp = created("x").unwrap();
            assert_eq!(resp.status(), axum::http::StatusCode::CREATED);
            assert_eq!(resp.body_json()["data"], "x");
        }

        #[test]
        fn ok_builds_200_with_envelope() {
            let resp = ok(42u32).unwrap();
            assert_eq!(resp.status(), axum::http::StatusCode::OK);
            assert_eq!(resp.body_json()["data"], 42);
        }

        #[test]
        fn etagged_builds_200_with_etag_and_envelope() {
            use api_bones::etag::ETag;
            let etag = ETag::strong("abc123");
            let resp = etagged(&etag, 99u32).unwrap();
            assert_eq!(resp.status(), axum::http::StatusCode::OK);
            assert_eq!(
                resp.headers()
                    .get(axum::http::header::ETAG)
                    .unwrap()
                    .to_str()
                    .unwrap(),
                etag.to_string(),
            );
            assert_eq!(resp.body_json()["data"], 99);
        }

        #[test]
        fn listed_wraps_paginated_response() {
            use api_bones::{PaginatedResponse, pagination::PaginationParams};
            let page: PaginatedResponse<u32> =
                PaginatedResponse::new(vec![1, 2], 2, &PaginationParams::default());
            let json = listed(page).unwrap().body_json();
            assert_eq!(json["data"]["items"], serde_json::json!([1, 2]));
        }

        #[test]
        fn listed_page_maps_and_paginates() {
            use api_bones::pagination::PaginationParams;
            let items: Vec<u32> = (1..=5).collect();
            let params = PaginationParams {
                offset: Some(1),
                limit: Some(2),
            };
            let json = listed_page::<u32, u64>(items, &params).unwrap().body_json();
            assert_eq!(json["data"]["items"], serde_json::json!([2, 3]));
        }

        #[test]
        fn listed_page_uses_defaults_when_params_are_none() {
            use api_bones::pagination::PaginationParams;
            let items: Vec<u32> = (1..=25).collect();
            let json = listed_page::<u32, u64>(items, &PaginationParams::default())
                .unwrap()
                .body_json();
            assert_eq!(json["data"]["items"].as_array().unwrap().len(), 20);
        }

        #[test]
        fn created_under_composes_location() {
            struct R {
                id: u64,
            }
            impl api_bones::HasId for R {
                type Id = u64;
                fn id(&self) -> &u64 {
                    &self.id
                }
            }
            impl serde::Serialize for R {
                fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                    s.serialize_u64(self.id)
                }
            }
            let resp = created_under("/v1/widgets", R { id: 7 }).unwrap();
            assert_eq!(resp.status(), axum::http::StatusCode::CREATED);
            assert_eq!(
                resp.headers().get(axum::http::header::LOCATION).unwrap(),
                "/v1/widgets/7",
            );
        }

        #[test]
        fn created_under_trims_trailing_slash() {
            struct R {
                id: String,
            }
            impl api_bones::HasId for R {
                type Id = String;
                fn id(&self) -> &String {
                    &self.id
                }
            }
            impl serde::Serialize for R {
                fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                    s.serialize_str(&self.id)
                }
            }
            let resp = created_under("/v1/things/", R { id: "abc".into() }).unwrap();
            assert_eq!(
                resp.headers().get(axum::http::header::LOCATION).unwrap(),
                "/v1/things/abc",
            );
        }
    }

    #[test]
    fn unconstrained_response_passes_through() {
        use axum::http::StatusCode;
        let resp = UnconstrainedResponse::new(StatusCode::IM_A_TEAPOT).into_response();
        assert_eq!(resp.status(), StatusCode::IM_A_TEAPOT);
    }
}
