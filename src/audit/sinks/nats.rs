use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};

use async_nats::jetstream;
use tokio::sync::mpsc;

use crate::audit::{AuditEvent, AuditSink, AuditSinkError};

static AUDIT_DROPPED_TOTAL: AtomicU64 = AtomicU64::new(0);

/// Returns the number of audit events dropped due to backpressure since process start.
pub fn audit_dropped_total() -> u64 {
    AUDIT_DROPPED_TOTAL.load(Ordering::Relaxed)
}

/// Publishes audit events to NATS `JetStream`.
///
/// Subject pattern: `audit.{service_name}.{resource_type|"unknown"}`.
///
/// A bounded internal queue (default 1024) decouples the request path from
/// the NATS publish call. When the queue is full the oldest event is dropped
/// and `socle_audit_dropped_total` is incremented.
pub struct NatsJetStreamAuditSink {
    tx: mpsc::Sender<AuditEvent>,
}

impl NatsJetStreamAuditSink {
    pub fn new(
        context: jetstream::Context,
        service_name: impl Into<String>,
        capacity: usize,
    ) -> Self {
        let service_name = service_name.into();
        let (tx, mut rx) = mpsc::channel::<AuditEvent>(capacity);

        tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                let resource_type = event
                    .resource_type
                    .as_deref()
                    .unwrap_or("unknown")
                    .to_owned();
                let subject = format!("audit.{service_name}.{resource_type}");
                match serde_json::to_vec(&event) {
                    Ok(payload) => {
                        if let Err(e) = context.publish(subject, payload.into()).await {
                            tracing::error!(
                                error = %e,
                                event = ?serde_json::to_value(&event).unwrap_or_default(),
                                "audit nats publish failed"
                            );
                        }
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "audit event serialize failed");
                    }
                }
            }
        });

        Self { tx }
    }

    pub fn with_default_capacity(
        context: jetstream::Context,
        service_name: impl Into<String>,
    ) -> Self {
        Self::new(context, service_name, 1024)
    }
}

impl AuditSink for NatsJetStreamAuditSink {
    fn emit(
        &self,
        event: AuditEvent,
    ) -> Pin<Box<dyn Future<Output = Result<(), AuditSinkError>> + Send + '_>> {
        let result = match self.tx.try_send(event) {
            Ok(()) => Ok(()),
            Err(mpsc::error::TrySendError::Full(dropped)) => {
                AUDIT_DROPPED_TOTAL.fetch_add(1, Ordering::Relaxed);
                metrics_api::counter!("socle_audit_dropped_total").increment(1);
                tracing::warn!(
                    event = ?serde_json::to_value(&dropped).unwrap_or_default(),
                    "audit event dropped: nats queue full"
                );
                Ok(())
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                Err(AuditSinkError("nats worker channel closed".to_owned()))
            }
        };
        Box::pin(async move { result })
    }
}
