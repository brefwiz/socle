//! Handler-level error type: wraps api_bones::error::ApiError with axum IntoResponse.

pub use api_bones::error::{ApiError, ErrorCode, ProblemJson, ValidationError};
use axum::response::{IntoResponse, Response};

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
    pub fn with_request_id(mut self, id: uuid::Uuid) -> Self {
        self.0 = self.0.with_request_id(id);
        self
    }

    /// Add validation errors to the error response.
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
                // Postgres unique violation = 23505
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

/// Return type for handlers that return a single resource wrapped in the platform envelope.
pub type HandlerResponse<T> = Result<
    (
        axum::http::StatusCode,
        axum::Json<api_bones::ApiResponse<T>>,
    ),
    HandlerError,
>;

/// Return type for handlers that return a paginated collection wrapped in the platform envelope.
pub type HandlerListResponse<T> =
    Result<axum::Json<api_bones::ApiResponse<api_bones::PaginatedResponse<T>>>, HandlerError>;

/// Return type for handlers that create a resource and return it with a 201 status.
pub type CreatedResponse<T> = Result<
    (
        axum::http::StatusCode,
        axum::Json<api_bones::ApiResponse<T>>,
    ),
    HandlerError,
>;

/// Return type for handlers that create a resource and return it with a 201 status and Location header.
pub type CreatedAtResponse<T> = Result<
    (
        axum::http::StatusCode,
        axum::http::HeaderMap,
        axum::Json<api_bones::ApiResponse<T>>,
    ),
    HandlerError,
>;

/// Return type for read/update handlers that carry an ETag response header.
pub type EtaggedHandlerResponse<T> = Result<
    (
        axum::http::StatusCode,
        api_bones::etag::ETag,
        axum::Json<api_bones::ApiResponse<T>>,
    ),
    HandlerError,
>;

/// Build the success response for a [`CreatedResponse`] handler (201 Created).
pub fn created<T>(value: T) -> CreatedResponse<T> {
    Ok((
        axum::http::StatusCode::CREATED,
        axum::Json(api_bones::ApiResponse::builder(value).build()),
    ))
}

/// Build the success response for a [`CreatedAtResponse`] handler (201 Created + Location header).
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

/// Build the success response for a [`HandlerResponse`] handler (200 OK).
pub fn ok<T>(value: T) -> HandlerResponse<T> {
    Ok((
        axum::http::StatusCode::OK,
        axum::Json(api_bones::ApiResponse::builder(value).build()),
    ))
}

/// Build the success response for a [`HandlerListResponse`] handler.
pub fn listed<T>(page: api_bones::PaginatedResponse<T>) -> HandlerListResponse<T> {
    Ok(axum::Json(api_bones::ApiResponse::builder(page).build()))
}

/// Paginate a fully-loaded `Vec<T>`, map each item to `U`, and return a [`HandlerListResponse`].
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
        .skip(params.offset.unwrap_or(0) as usize)
        .take(params.limit.unwrap_or(20) as usize)
        .map(Into::into)
        .collect();
    listed(api_bones::PaginatedResponse::new(page, total, params))
}

/// Build the success response for an [`EtaggedHandlerResponse`] handler (200 OK + ETag header).
pub fn etagged<T>(etag: api_bones::etag::ETag, value: T) -> EtaggedHandlerResponse<T> {
    Ok((
        axum::http::StatusCode::OK,
        etag,
        axum::Json(api_bones::ApiResponse::builder(value).build()),
    ))
}

/// Build a Problem+JSON 500 response from a panic payload. Used in catch-panic layer.
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

    #[test]
    fn created_builds_201_with_envelope() {
        let (status, body) = created("x").unwrap();
        assert_eq!(status, axum::http::StatusCode::CREATED);
        let json = serde_json::to_value(body.0).unwrap();
        assert_eq!(json["data"], "x");
    }

    #[test]
    fn ok_builds_200_with_envelope() {
        let (status, body) = ok(42u32).unwrap();
        assert_eq!(status, axum::http::StatusCode::OK);
        let json = serde_json::to_value(body.0).unwrap();
        assert_eq!(json["data"], 42);
    }

    #[test]
    fn etagged_builds_200_with_etag_and_envelope() {
        use api_bones::etag::ETag;
        let etag = ETag::strong("abc123");
        let (status, out_etag, body) = etagged(etag.clone(), 99u32).unwrap();
        assert_eq!(status, axum::http::StatusCode::OK);
        assert_eq!(out_etag, etag);
        let json = serde_json::to_value(body.0).unwrap();
        assert_eq!(json["data"], 99);
    }

    #[test]
    fn listed_wraps_paginated_response() {
        use api_bones::{PaginatedResponse, pagination::PaginationParams};
        let page: PaginatedResponse<u32> =
            PaginatedResponse::new(vec![1, 2], 2, &PaginationParams::default());
        let body = listed(page).unwrap();
        let json = serde_json::to_value(body.0).unwrap();
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
        let body = listed_page::<u32, u64>(items, &params).unwrap();
        let json = serde_json::to_value(body.0).unwrap();
        assert_eq!(json["data"]["items"], serde_json::json!([2, 3]));
    }

    #[test]
    fn listed_page_uses_defaults_when_params_are_none() {
        use api_bones::pagination::PaginationParams;
        let items: Vec<u32> = (1..=25).collect();
        let params = PaginationParams::default();
        let body = listed_page::<u32, u64>(items, &params).unwrap();
        let json = serde_json::to_value(body.0).unwrap();
        assert_eq!(json["data"]["items"].as_array().unwrap().len(), 20);
    }
}
