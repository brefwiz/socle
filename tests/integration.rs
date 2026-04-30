//! End-to-end integration tests — spin up a real `ServiceBootstrap` server on a
//! random port, hit it with `reqwest`, assert responses.

use axum::{Router, routing::get};
use socle::{BootstrapCtx, ServiceBootstrap, assert_span, testing::TestClient};
use tokio::sync::oneshot;

// ── helpers ───────────────────────────────────────────────────────────────────

/// Bind on a random port, spawn the service in the background, return a
/// `TestClient` pre-pointed at the bound address and a shutdown sender.
/// Dropping the sender stops the server.
async fn spawn_service(
    build: impl FnOnce(ServiceBootstrap) -> ServiceBootstrap,
) -> (TestClient, oneshot::Sender<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("local_addr");
    let (tx, rx) = oneshot::channel::<()>();

    let svc = build(ServiceBootstrap::new("test-service"));
    tokio::spawn(async move {
        svc.serve_with_shutdown(listener, async {
            rx.await.ok();
        })
        .await
        .ok();
    });

    // Give the Tokio task a moment to start accepting connections.
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;

    (TestClient::new(format!("http://{addr}")), tx)
}

// ── health endpoint ───────────────────────────────────────────────────────────

#[tokio::test]
async fn health_liveness_returns_200() {
    let (client, _stop) = spawn_service(|s| s.with_router(|_: &BootstrapCtx| Router::new())).await;

    let resp = client.get("/health/live").await;
    assert_eq!(resp.status(), 200);
}

#[tokio::test]
async fn health_readiness_returns_200_when_no_checks_registered() {
    let (client, _stop) = spawn_service(|s| s.with_router(|_: &BootstrapCtx| Router::new())).await;

    let resp = client.get("/health/ready").await;
    assert_eq!(resp.status(), 200);
}

// ── user routes ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn user_route_is_reachable() {
    let (client, _stop) = spawn_service(|s| {
        s.with_router(|_: &BootstrapCtx| Router::new().route("/hello", get(|| async { "world" })))
    })
    .await;

    let resp = client.get("/hello").await;
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.text().await.unwrap(), "world");
}

// ── 404 fallback ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn unknown_route_returns_404_problem_json() {
    let (client, _stop) = spawn_service(|s| s.with_router(|_: &BootstrapCtx| Router::new())).await;

    let resp = client.get("/does-not-exist").await;
    assert_eq!(resp.status(), 404);

    let ct = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(
        ct.contains("application/problem+json"),
        "expected problem+json, got {ct}"
    );
}

// ── rate limiting ─────────────────────────────────────────────────────────────

#[cfg(feature = "ratelimit-memory")]
#[tokio::test]
async fn rate_limit_blocks_after_limit_exceeded() {
    use socle::RateLimitBackend;

    let (client, _stop) = spawn_service(|s| {
        s.with_rate_limit(RateLimitBackend {
            limit: 2,
            window_secs: 60,
        })
        .with_router(|_: &BootstrapCtx| Router::new().route("/", get(|| async { "ok" })))
    })
    .await;

    assert_eq!(client.get("/").await.status(), 200);
    assert_eq!(client.get("/").await.status(), 200);
    assert_eq!(client.get("/").await.status(), 429);
}

// ── span capture ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn span_capture_records_closed_span() {
    use socle::testing::trace::init_capture_tracing;

    let exporter = init_capture_tracing();
    let _ = exporter.drain(); // clear any spans from prior tests

    tracing::info_span!("test_op").in_scope(|| {});

    let spans = exporter.spans();
    assert_span!(spans, name = "test_op");
}

#[tokio::test]
async fn capture_exporter_drain_empties_buffer() {
    use socle::testing::trace::init_capture_tracing;

    let exporter = init_capture_tracing();
    let _ = exporter.drain();

    tracing::info_span!("drain_op").in_scope(|| {});

    let drained = exporter.drain();
    assert!(drained.iter().any(|s| s.name == "drain_op"));
    assert!(exporter.spans().is_empty());
}

#[tokio::test]
async fn test_app_builder_serves_and_shuts_down() {
    use axum::http::StatusCode;
    use socle::testing::{TestApp, TestClient};

    let router = Router::new().route("/ping", get(|| async { StatusCode::OK }));
    let app = TestApp::builder().router(router).build().await;

    let client: TestClient = app.client();
    let resp = client.get("/ping").await;
    assert_eq!(resp.status(), 200);

    app.shutdown().await;
}
