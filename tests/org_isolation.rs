use axum::Router;
use axum::body::Body;
use axum::extract::FromRequestParts;
use axum::http::{Request, StatusCode};
use axum::response::IntoResponse;
use axum::routing::get;
use http_body_util::BodyExt;
use tower::ServiceExt;

use api_bones::audit::Principal;
use socle::org_context::OrganizationContext;
use api_bones::org_id::OrgId;
use api_bones::request_id::RequestId;

use socle::org_isolation::{OrgContextExtractor, OrgContextSource, OrgIsolationLayer};

fn make_ctx(org_id: OrgId) -> OrganizationContext {
    OrganizationContext::new(org_id, Principal::system("test"), RequestId::new())
        .with_org_path(vec![org_id])
}

// ── OrgContextExtractor ───────────────────────────────────────────────────────

#[tokio::test]
async fn extractor_claim_only_resolves() {
    let org_id = OrgId::generate();
    let mut req = Request::builder().uri("/").body(()).unwrap();
    req.extensions_mut().insert(make_ctx(org_id));
    let (mut parts, ()) = req.into_parts();

    let result = OrgContextExtractor::from_request_parts(&mut parts, &()).await;
    assert!(result.is_ok());
    let OrgContextExtractor(ctx) = result.unwrap();
    assert_eq!(ctx.org_id, org_id);
    assert_eq!(
        parts.extensions.get::<OrgContextSource>(),
        Some(&OrgContextSource::PrincipalClaim)
    );
}

#[tokio::test]
async fn extractor_header_only_resolves() {
    let org_id = OrgId::generate();
    let req = Request::builder()
        .uri("/")
        .header("x-org-id", org_id.to_string())
        .body(())
        .unwrap();
    let (mut parts, ()) = req.into_parts();

    let result = OrgContextExtractor::from_request_parts(&mut parts, &()).await;
    assert!(result.is_ok());
    let OrgContextExtractor(ctx) = result.unwrap();
    assert_eq!(ctx.org_id, org_id);
    assert_eq!(
        parts.extensions.get::<OrgContextSource>(),
        Some(&OrgContextSource::Header)
    );
}

#[tokio::test]
async fn extractor_conflicting_claim_and_header_rejects_403() {
    let claim_org = OrgId::generate();
    let header_org = OrgId::generate();
    assert_ne!(claim_org, header_org);

    let mut req = Request::builder()
        .uri("/")
        .header("x-org-id", header_org.to_string())
        .body(())
        .unwrap();
    req.extensions_mut().insert(make_ctx(claim_org));
    let (mut parts, ()) = req.into_parts();

    let result = OrgContextExtractor::from_request_parts(&mut parts, &()).await;
    assert!(result.is_err());
    let err = result.unwrap_err();
    let resp = err.into_response();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn extractor_missing_both_rejects_401() {
    let req = Request::builder().uri("/").body(()).unwrap();
    let (mut parts, ()) = req.into_parts();

    let result = OrgContextExtractor::from_request_parts(&mut parts, &()).await;
    assert!(result.is_err());
    let resp = result.unwrap_err().into_response();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn extractor_inserts_extension_on_success() {
    let org_id = OrgId::generate();
    let req = Request::builder()
        .uri("/")
        .header("x-org-id", org_id.to_string())
        .body(())
        .unwrap();
    let (mut parts, ()) = req.into_parts();

    OrgContextExtractor::from_request_parts(&mut parts, &())
        .await
        .unwrap();

    assert!(parts.extensions.get::<OrganizationContext>().is_some());
}

// ── OrgIsolationLayer ─────────────────────────────────────────────────────────

#[tokio::test]
async fn isolation_layer_passes_with_extension() {
    let org_id = OrgId::generate();
    let app = Router::new()
        .route("/", get(|| async { "ok" }))
        .layer(OrgIsolationLayer);

    let mut req = Request::builder().uri("/").body(Body::empty()).unwrap();
    req.extensions_mut().insert(make_ctx(org_id));

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn isolation_layer_short_circuits_401_without_extension() {
    let app = Router::new()
        .route("/", get(|| async { "ok" }))
        .layer(OrgIsolationLayer);

    let req = Request::builder().uri("/").body(Body::empty()).unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

// ── Integration: OrgContextExtractor middleware + OrgIsolationLayer ───────────

#[tokio::test]
async fn integration_extractor_middleware_with_isolation_layer() {
    use axum::middleware;

    async fn inject_org_ctx(
        mut req: Request<Body>,
        next: middleware::Next,
    ) -> axum::response::Response {
        let org_id = OrgId::generate();
        req.extensions_mut().insert(make_ctx(org_id));
        next.run(req).await
    }

    let app = Router::new()
        .route("/", get(|| async { "ok" }))
        .layer(OrgIsolationLayer)
        .layer(middleware::from_fn(inject_org_ctx));

    let req = Request::builder().uri("/").body(Body::empty()).unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&bytes[..], b"ok");
}
