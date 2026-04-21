//! Enrich bare error responses with a Problem+JSON body.

use axum::{
    body::Body,
    http::{Request, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};

use api_bones::error::{ApiError, ErrorCode};

/// Axum middleware: enriches bare error responses with a Problem+JSON body.
pub async fn enrich_error_response(req: Request<Body>, next: Next) -> Response {
    let resp = next.run(req).await;

    let status = resp.status();

    if !status.is_client_error() && !status.is_server_error() {
        return resp;
    }

    if resp.headers().get(header::CONTENT_TYPE).is_some() {
        return resp;
    }

    let (parts, _body) = resp.into_parts();

    let err = match status {
        StatusCode::UNAUTHORIZED => ApiError::unauthorized("Authentication required"),
        StatusCode::FORBIDDEN => ApiError::forbidden("Insufficient permissions"),
        other => ApiError::new(
            ErrorCode::InternalServerError,
            other.canonical_reason().unwrap_or("Unexpected error"),
        ),
    };

    let mut new_resp = err.into_response();
    for (name, value) in parts
        .headers
        .into_iter()
        .flat_map(|(n, v)| n.map(|n| (n, v)))
    {
        if name != header::CONTENT_TYPE && name != header::CONTENT_LENGTH {
            new_resp.headers_mut().insert(name, value);
        }
    }
    *new_resp.status_mut() = status;
    new_resp
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Response, StatusCode};
    use axum::middleware;
    use axum::routing::get;
    use http_body_util::BodyExt as _;
    use tower::ServiceExt as _;

    async fn body_bytes(resp: Response<Body>) -> bytes::Bytes {
        resp.into_body().collect().await.unwrap().to_bytes()
    }

    fn make_app(status: StatusCode, content_type: Option<&'static str>) -> Router {
        Router::new()
            .route(
                "/",
                get(move || async move {
                    let mut b = axum::http::Response::builder().status(status);
                    if let Some(ct) = content_type {
                        b = b.header("content-type", ct);
                    }
                    b.body(Body::empty()).unwrap()
                }),
            )
            .layer(middleware::from_fn(enrich_error_response))
    }

    #[tokio::test]
    async fn passes_through_success_response() {
        let app = make_app(StatusCode::OK, None);
        let resp = app
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(resp.headers().get("content-type").is_none());
    }

    #[tokio::test]
    async fn passes_through_response_with_content_type() {
        let app = make_app(StatusCode::NOT_FOUND, Some("application/json"));
        let resp = app
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        assert_eq!(
            resp.headers().get("content-type").unwrap(),
            "application/json"
        );
    }

    #[tokio::test]
    async fn enriches_bare_401() {
        let app = make_app(StatusCode::UNAUTHORIZED, None);
        let resp = app
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        let ct = resp
            .headers()
            .get("content-type")
            .unwrap()
            .to_str()
            .unwrap();
        assert!(ct.contains("application/problem+json") || ct.contains("application/json"));
    }

    #[tokio::test]
    async fn enriches_bare_403() {
        let app = make_app(StatusCode::FORBIDDEN, None);
        let resp = app
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
        let bytes = body_bytes(resp).await;
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert!(
            json["title"]
                .as_str()
                .unwrap()
                .to_lowercase()
                .contains("forbidden")
                || json["detail"]
                    .as_str()
                    .unwrap()
                    .to_lowercase()
                    .contains("permission")
        );
    }

    #[tokio::test]
    async fn enriches_bare_500() {
        let app = make_app(StatusCode::INTERNAL_SERVER_ERROR, None);
        let resp = app
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let bytes = body_bytes(resp).await;
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert!(json.get("status").is_some());
    }

    #[tokio::test]
    async fn preserves_custom_headers_on_enriched_response() {
        let app = Router::new()
            .route(
                "/",
                get(|| async {
                    axum::http::Response::builder()
                        .status(StatusCode::UNAUTHORIZED)
                        .header("x-custom", "value")
                        .body(Body::empty())
                        .unwrap()
                }),
            )
            .layer(middleware::from_fn(enrich_error_response));
        let resp = app
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.headers().get("x-custom").unwrap(), "value");
    }
}
