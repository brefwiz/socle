#![cfg(feature = "test-util")]

use std::sync::Arc;

use chrono::Utc;
use socle::{AuditEvent, AuditSink, ChannelAuditSink};

fn sample_event() -> AuditEvent {
    AuditEvent {
        request_id: Some("req-1".to_owned()),
        traceparent: None,
        principal_id: Some("user-1".to_owned()),
        org_path: Some("org-a".to_owned()),
        method: "POST".to_owned(),
        path_template: "/orders".to_owned(),
        status: 201,
        started_at: Utc::now(),
        duration_ms: 5,
        resource_type: Some("order".to_owned()),
        resource_id: Some("ord-1".to_owned()),
        action: Some("create".to_owned()),
        changes: None,
    }
}

#[tokio::test]
async fn channel_sink_delivers_event() {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let sink = Arc::new(ChannelAuditSink::new(tx));

    let event = sample_event();
    sink.emit(event.clone()).await.unwrap();

    let received = rx.try_recv().expect("channel should have one event");
    assert_eq!(received.request_id, event.request_id);
    assert_eq!(received.path_template, "/orders");
    assert_eq!(received.status, 201);
    assert_eq!(received.resource_type.as_deref(), Some("order"));
}

#[tokio::test]
async fn channel_sink_error_on_closed_receiver() {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<AuditEvent>();
    drop(rx);
    let sink = Arc::new(ChannelAuditSink::new(tx));
    let result = sink.emit(sample_event()).await;
    assert!(result.is_err());
}
