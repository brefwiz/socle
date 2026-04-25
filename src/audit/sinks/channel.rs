use std::future::Future;
use std::pin::Pin;

use tokio::sync::mpsc::UnboundedSender;

use crate::audit::{AuditEvent, AuditSink, AuditSinkError};

/// Pushes audit events into an unbounded channel for assertion in tests.
///
/// Construct with [`tokio::sync::mpsc::unbounded_channel`] and pass the
/// sender here; receive events from the corresponding receiver.
pub struct ChannelAuditSink {
    tx: UnboundedSender<AuditEvent>,
}

impl ChannelAuditSink {
    pub fn new(tx: UnboundedSender<AuditEvent>) -> Self {
        Self { tx }
    }
}

impl AuditSink for ChannelAuditSink {
    fn emit(
        &self,
        event: AuditEvent,
    ) -> Pin<Box<dyn Future<Output = Result<(), AuditSinkError>> + Send + '_>> {
        let result = self
            .tx
            .send(event)
            .map_err(|e| AuditSinkError(e.to_string()));
        Box::pin(async move { result })
    }
}
