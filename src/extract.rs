//! Axum extractors provided by socle.

#[cfg(feature = "validation")]
pub use validation::Valid;

#[cfg(feature = "validation")]
mod validation {
    use axum::Json;
    use axum::extract::{FromRequest, Request};
    use axum::response::{IntoResponse, Response};
    use validator::Validate;

    use crate::handler_error::{ErrorCode, HandlerError, ValidationError};

    /// Axum extractor that deserializes a JSON body and then validates it.
    pub struct Valid<T>(pub T);

    /// Rejection type returned when deserialization or validation fails.
    pub struct ValidRejection(Response);

    impl IntoResponse for ValidRejection {
        fn into_response(self) -> Response {
            self.0
        }
    }

    impl<T, S> FromRequest<S> for Valid<T>
    where
        T: serde::de::DeserializeOwned + Validate + Send + 'static,
        S: Send + Sync,
    {
        type Rejection = ValidRejection;

        async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
            let Json(value) = Json::<T>::from_request(req, state)
                .await
                .map_err(|rejection| {
                    let err = HandlerError::new(ErrorCode::BadRequest, rejection.to_string());
                    ValidRejection(err.into_response())
                })?;

            value.validate().map_err(|errs| {
                let field_errors = errs
                    .field_errors()
                    .into_iter()
                    .flat_map(|(field, errors)| {
                        errors.iter().map(move |e| ValidationError {
                            field: format!("/{field}"),
                            message: e.message.as_deref().unwrap_or("invalid value").to_string(),
                            rule: Some(e.code.to_string()),
                        })
                    })
                    .collect::<Vec<_>>();

                let err =
                    HandlerError::new(ErrorCode::ValidationFailed, "request validation failed")
                        .with_errors(field_errors);
                ValidRejection(err.into_response())
            })?;

            Ok(Valid(value))
        }
    }
}
