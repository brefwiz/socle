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

/// Convenience return type for handlers that return `201 Created` with a JSON body.
pub type CreatedResult<T> = Result<(axum::http::StatusCode, axum::Json<T>), HandlerError>;

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
}
