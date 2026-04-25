use std::future::Future;
use std::pin::Pin;

use crate::audit::{AuditEvent, AuditSink, AuditSinkError};

/// Writes audit events to `tracing` at `INFO` level with target `"audit"`.
///
/// Every [`AuditEvent`] field is emitted as a structured key so log
/// processors and tracing subscribers can index them individually.
pub struct TracingAuditSink;

impl AuditSink for TracingAuditSink {
    fn emit(
        &self,
        e: AuditEvent,
    ) -> Pin<Box<dyn Future<Output = Result<(), AuditSinkError>> + Send + '_>> {
        Box::pin(async move {
            tracing::info!(
                target: "audit",
                request_id = e.request_id.as_deref().unwrap_or(""),
                traceparent = e.traceparent.as_deref().unwrap_or(""),
                principal_id = e.principal_id.as_deref().unwrap_or(""),
                org_path = e.org_path.as_deref().unwrap_or(""),
                method = %e.method,
                path_template = %e.path_template,
                status = e.status,
                started_at = %e.started_at,
                duration_ms = e.duration_ms,
                resource_type = e.resource_type.as_deref().unwrap_or(""),
                resource_id = e.resource_id.as_deref().unwrap_or(""),
                action = e.action.as_deref().unwrap_or(""),
                "audit"
            );
            Ok(())
        })
    }
}
