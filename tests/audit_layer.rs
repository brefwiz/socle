#![cfg(feature = "test-util")]

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::Extension;
use axum::http::{Request, StatusCode};
use axum::response::IntoResponse;
use axum::routing::post;
use tower::ServiceExt;

use socle::{AuditAnnotation, AuditAnnotationSlot, AuditLayer, ChannelAuditSink};

async fn create_handler(Extension(slot): Extension<AuditAnnotationSlot>) -> impl IntoResponse {
    slot.annotate(
        AuditAnnotation::default()
            .set_resource("order", "ord-1")
            .set_action("create"),
    );
    StatusCode::CREATED
}

async fn get_handler() -> impl IntoResponse {
    StatusCode::OK
}

fn build_router(sink: Arc<ChannelAuditSink>) -> Router {
    Router::new()
        .route("/orders", post(create_handler))
        .route("/orders", axum::routing::get(get_handler))
        .layer(AuditLayer::new(sink))
}

#[tokio::test]
async fn emits_one_event_per_matching_post() {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let sink = Arc::new(ChannelAuditSink::new(tx));
    let app = build_router(sink);

    let req = Request::builder()
        .method("POST")
        .uri("/orders")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    tokio::task::yield_now().await;

    let event = rx.try_recv().expect("one audit event expected");
    assert_eq!(event.method, "POST");
    assert_eq!(event.status, 201);
    assert_eq!(event.resource_type.as_deref(), Some("order"));
    assert_eq!(event.resource_id.as_deref(), Some("ord-1"));
    assert_eq!(event.action.as_deref(), Some("create"));
}

#[tokio::test]
async fn no_event_for_get_request() {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let sink = Arc::new(ChannelAuditSink::new(tx));
    let app = build_router(sink);

    let req = Request::builder()
        .method("GET")
        .uri("/orders")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    tokio::task::yield_now().await;

    assert!(rx.try_recv().is_err(), "GET should not emit an audit event");
}

#[tokio::test]
async fn no_event_for_healthz_path() {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let sink = Arc::new(ChannelAuditSink::new(tx));

    let app = Router::new()
        .route("/healthz", axum::routing::get(get_handler))
        .route("/healthz", post(create_handler))
        .layer(AuditLayer::new(sink));

    let req = Request::builder()
        .method("POST")
        .uri("/healthz")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    tokio::task::yield_now().await;

    assert!(
        rx.try_recv().is_err(),
        "POST /healthz should not emit an audit event"
    );
}

async fn bare_handler() -> impl IntoResponse {
    StatusCode::CREATED
}

#[tokio::test]
async fn annotation_without_slot_does_not_panic() {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let sink = Arc::new(ChannelAuditSink::new(tx));

    let app = Router::new()
        .route("/items", post(bare_handler))
        .layer(AuditLayer::new(sink));

    let req = Request::builder()
        .method("POST")
        .uri("/items")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    tokio::task::yield_now().await;

    let event = rx
        .try_recv()
        .expect("event expected even without annotation");
    assert!(event.resource_type.is_none());
}
