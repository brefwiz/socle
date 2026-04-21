//! Health endpoint adapter.

use std::collections::HashMap;
use std::sync::Arc;

use axum::{Json, Router, http::StatusCode, response::IntoResponse, routing::get};

use crate::ports::health::ReadinessCheckFn;

/// Build the health sub-router mounted under `base` (default `/health`).
pub(crate) fn build_health_router(
    base: &str,
    service_id: &str,
    version: &str,
    checks: Vec<(String, ReadinessCheckFn)>,
) -> Router {
    use api_bones::health::{HealthCheck, HealthStatus, LivenessResponse};

    let service_id = service_id.to_string();
    let version = version.to_string();

    let live_path = format!("{base}/live");
    let ready_path = format!("{base}/ready");

    let checks = Arc::new(checks);

    Router::new()
        .route(
            &live_path,
            get(move || {
                let body = LivenessResponse::pass(version.clone(), service_id.clone());
                async move { (StatusCode::OK, Json(body)).into_response() }
            }),
        )
        .route(
            &ready_path,
            get(move || {
                let checks = checks.clone();
                async move {
                    let mut results: Vec<HealthCheck> = Vec::with_capacity(checks.len());
                    let mut worst = HealthStatus::Pass;
                    for (_name, check) in checks.iter() {
                        let result = check().await;
                        worst = worst_of(worst, result.status.clone());
                        results.push(result);
                    }
                    let mut by_name: HashMap<String, Vec<HealthCheck>> = HashMap::new();
                    for ((name, _), result) in checks.iter().zip(results) {
                        by_name.entry(name.clone()).or_default().push(result);
                    }
                    let status = StatusCode::from_u16(worst.http_status())
                        .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
                    (status, Json(by_name)).into_response()
                }
            }),
        )
}

/// Fallback handler for unmatched routes. Returns RFC 9457 Problem+JSON 404.
pub(crate) async fn not_found_fallback(req: axum::extract::Request) -> axum::response::Response {
    use api_bones::ApiError;
    let path = req.uri().path().to_string();
    let rid = crate::request_id::extract_request_id(&req);
    let _ = path; // path not echoed — avoid information disclosure
    let mut err = ApiError::not_found("route not found");
    if let Ok(uuid) = uuid::Uuid::parse_str(rid) {
        err = err.with_request_id(uuid);
    }
    err.into_response()
}

fn worst_of(
    a: api_bones::health::HealthStatus,
    b: api_bones::health::HealthStatus,
) -> api_bones::health::HealthStatus {
    use api_bones::health::HealthStatus::*;
    match (a, b) {
        (Fail, _) | (_, Fail) => Fail,
        (Warn, _) | (_, Warn) => Warn,
        _ => Pass,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use api_bones::health::HealthStatus;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt as _;
    use tower::ServiceExt as _;

    async fn body_json(resp: axum::response::Response) -> serde_json::Value {
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        serde_json::from_slice(&bytes).unwrap()
    }

    fn req(uri: &str) -> Request<Body> {
        Request::builder().uri(uri).body(Body::empty()).unwrap()
    }

    #[test]
    fn worst_of_pass_pass_is_pass() {
        assert_eq!(
            worst_of(HealthStatus::Pass, HealthStatus::Pass),
            HealthStatus::Pass
        );
    }

    #[test]
    fn worst_of_pass_warn_is_warn() {
        assert_eq!(
            worst_of(HealthStatus::Pass, HealthStatus::Warn),
            HealthStatus::Warn
        );
    }

    #[test]
    fn worst_of_warn_fail_is_fail() {
        assert_eq!(
            worst_of(HealthStatus::Warn, HealthStatus::Fail),
            HealthStatus::Fail
        );
    }

    #[test]
    fn worst_of_fail_pass_is_fail() {
        assert_eq!(
            worst_of(HealthStatus::Fail, HealthStatus::Pass),
            HealthStatus::Fail
        );
    }

    #[tokio::test]
    async fn liveness_returns_200() {
        let router = build_health_router("/health", "svc", "1.0.0", vec![]);
        let resp = router.oneshot(req("/health/live")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn liveness_body_has_service_id() {
        let router = build_health_router("/health", "my-svc", "2.0.0", vec![]);
        let resp = router.oneshot(req("/health/live")).await.unwrap();
        let json = body_json(resp).await;
        assert!(json.to_string().contains("my-svc"));
    }

    #[tokio::test]
    async fn readiness_no_checks_returns_200() {
        let router = build_health_router("/health", "svc", "1.0.0", vec![]);
        let resp = router.oneshot(req("/health/ready")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn readiness_all_passing_returns_200() {
        use api_bones::health::HealthCheck;
        use std::sync::Arc;
        let check: ReadinessCheckFn = Arc::new(|| Box::pin(async { HealthCheck::pass("ok") }));
        let router = build_health_router("/health", "svc", "1.0.0", vec![("db".into(), check)]);
        let resp = router.oneshot(req("/health/ready")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn readiness_failing_check_returns_503() {
        use api_bones::health::HealthCheck;
        use std::sync::Arc;
        let check: ReadinessCheckFn =
            Arc::new(|| Box::pin(async { HealthCheck::fail("db", "down") }));
        let router = build_health_router("/health", "svc", "1.0.0", vec![("db".into(), check)]);
        let resp = router.oneshot(req("/health/ready")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn not_found_fallback_returns_404() {
        use axum::routing::get;
        let router = axum::Router::new()
            .route("/exists", get(|| async { "ok" }))
            .fallback(not_found_fallback);
        let r = Request::builder()
            .uri("/missing")
            .body(Body::empty())
            .unwrap();
        let resp = router.oneshot(r).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }
}
