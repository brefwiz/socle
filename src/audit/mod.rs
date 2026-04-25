//! Audit capture Tower layer with pluggable sinks.
//!
//! Emits one [`AuditEvent`] per mutating request without requiring handlers
//! to explicitly emit audit records.
//!
//! ## Required middleware ordering
//!
//! ```text
//! RequestIdLayer → auth → OrgContextExtractor → AuditLayer → handler
//! ```
//!
//! `RequestIdLayer` (from `tower-http`) must run first so the request-id
//! extension is populated when `AuditLayer` captures it.
//! `OrgContextExtractor` must run before `AuditLayer` so the principal and
//! org-path are available.
//!
//! ## ADR references
//!
//! - platform/0005 — NATS JetStream as event broker
//! - platform/0016 — handler conventions and infra-probe exemptions

pub mod sinks;

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use axum::extract::MatchedPath;
use axum::http::Request;
use axum::response::Response;
use chrono::{DateTime, Utc};
use tower::{Layer, Service};

pub use sinks::TracingAuditSink;

#[cfg(feature = "test-util")]
pub use sinks::ChannelAuditSink;

#[cfg(feature = "nats")]
pub use sinks::{NatsJetStreamAuditSink, audit_dropped_total};

// ── AuditSinkError ────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
#[error("audit sink error: {0}")]
pub struct AuditSinkError(pub String);

// ── AuditSink ─────────────────────────────────────────────────────────────────

/// Pluggable sink for audit events.
///
/// Implement this trait to route [`AuditEvent`]s to any backend (tracing,
/// NATS JetStream, a channel for testing, etc.).
///
/// Implementations must be `Send + Sync + 'static` so they can be shared
/// across requests via `Arc`.
pub trait AuditSink: Send + Sync + 'static {
    fn emit(
        &self,
        event: AuditEvent,
    ) -> Pin<Box<dyn Future<Output = Result<(), AuditSinkError>> + Send + '_>>;
}

// ── AuditEvent ────────────────────────────────────────────────────────────────

/// The audit record emitted once per matching request.
#[derive(Debug, Clone, serde::Serialize)]
pub struct AuditEvent {
    pub request_id: Option<String>,
    pub traceparent: Option<String>,
    pub principal_id: Option<String>,
    pub org_path: Option<String>,
    pub method: String,
    pub path_template: String,
    pub status: u16,
    pub started_at: DateTime<Utc>,
    pub duration_ms: u64,
    pub resource_type: Option<String>,
    pub resource_id: Option<String>,
    pub action: Option<String>,
    pub changes: Option<serde_json::Value>,
}

// ── AuditAnnotation ───────────────────────────────────────────────────────────

/// Domain-level enrichment attached by handlers.
///
/// Retrieve the [`AuditAnnotationSlot`] from axum `Extension`s and call the
/// builder methods to add context before returning a response.
#[derive(Debug, Clone, Default)]
pub struct AuditAnnotation {
    pub resource_type: Option<String>,
    pub resource_id: Option<String>,
    pub action: Option<String>,
    pub changes: Option<serde_json::Value>,
}

impl AuditAnnotation {
    pub fn set_resource(mut self, r_type: impl Into<String>, r_id: impl Into<String>) -> Self {
        self.resource_type = Some(r_type.into());
        self.resource_id = Some(r_id.into());
        self
    }

    pub fn set_action(mut self, action: impl Into<String>) -> Self {
        self.action = Some(action.into());
        self
    }

    pub fn set_changes(mut self, value: serde_json::Value) -> Self {
        self.changes = Some(value);
        self
    }
}

// ── AuditAnnotationSlot ───────────────────────────────────────────────────────

/// Shared slot pre-inserted by [`AuditLayer`] into request extensions.
///
/// Handlers extract this via `Extension<AuditAnnotationSlot>` and call
/// `annotate(annotation)` to attach domain-level enrichment.
#[derive(Clone, Default)]
pub struct AuditAnnotationSlot(Arc<Mutex<Option<AuditAnnotation>>>);

impl AuditAnnotationSlot {
    pub fn annotate(&self, annotation: AuditAnnotation) {
        *self.0.lock().expect("audit annotation mutex poisoned") = Some(annotation);
    }

    fn take(&self) -> Option<AuditAnnotation> {
        self.0
            .lock()
            .expect("audit annotation mutex poisoned")
            .take()
    }
}

// ── AuditFilter ───────────────────────────────────────────────────────────────

/// Controls which requests are audited.
///
/// By default includes mutating methods (`POST`, `PUT`, `PATCH`, `DELETE`)
/// and excludes infrastructure probe paths (`/healthz`, `/readyz`, `/livez`,
/// paths ending in `/status`).
#[derive(Clone, Debug)]
pub struct AuditFilter {
    included_methods: Vec<axum::http::Method>,
    excluded_path_prefixes: Vec<String>,
    excluded_path_suffixes: Vec<String>,
    additional_included_paths: Vec<String>,
}

impl Default for AuditFilter {
    fn default() -> Self {
        use axum::http::Method;
        Self {
            included_methods: vec![Method::POST, Method::PUT, Method::PATCH, Method::DELETE],
            excluded_path_prefixes: vec![
                "/healthz".to_owned(),
                "/readyz".to_owned(),
                "/livez".to_owned(),
            ],
            excluded_path_suffixes: vec!["/status".to_owned()],
            additional_included_paths: Vec::new(),
        }
    }
}

impl AuditFilter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn include_method(mut self, method: axum::http::Method) -> Self {
        self.included_methods.push(method);
        self
    }

    pub fn exclude_path_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.excluded_path_prefixes.push(prefix.into());
        self
    }

    pub fn include_path(mut self, path: impl Into<String>) -> Self {
        self.additional_included_paths.push(path.into());
        self
    }

    pub fn matches(&self, method: &axum::http::Method, path: &str) -> bool {
        if self
            .additional_included_paths
            .iter()
            .any(|p| path.starts_with(p.as_str()))
        {
            return true;
        }

        let method_ok = self.included_methods.contains(method);
        if !method_ok {
            return false;
        }

        let path_excluded = self
            .excluded_path_prefixes
            .iter()
            .any(|prefix| path == prefix || path.starts_with(&format!("{prefix}/")))
            || self
                .excluded_path_suffixes
                .iter()
                .any(|suffix| path.ends_with(suffix.as_str()));

        !path_excluded
    }
}

// ── AuditLayer ────────────────────────────────────────────────────────────────

/// Tower layer that auto-emits an [`AuditEvent`] per matching mutating request.
///
/// ## Middleware ordering
///
/// Apply this layer **after** `RequestIdLayer`, auth, and `OrgContextExtractor`
/// so all context is available when the event is built.
///
/// ## ADR references
///
/// - platform/0005 — NATS JetStream as event broker
/// - platform/0016 — handler conventions and infra-probe exemptions
#[derive(Clone)]
pub struct AuditLayer {
    sink: Arc<dyn AuditSink>,
    filter: AuditFilter,
}

impl AuditLayer {
    pub fn new(sink: Arc<dyn AuditSink>) -> Self {
        Self {
            sink,
            filter: AuditFilter::default(),
        }
    }

    pub fn with_filter(mut self, filter: AuditFilter) -> Self {
        self.filter = filter;
        self
    }
}

impl<S> Layer<S> for AuditLayer {
    type Service = AuditService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        AuditService {
            inner,
            sink: Arc::clone(&self.sink),
            filter: self.filter.clone(),
        }
    }
}

// ── AuditService ──────────────────────────────────────────────────────────────

/// Tower service produced by [`AuditLayer`].
#[derive(Clone)]
pub struct AuditService<S> {
    inner: S,
    sink: Arc<dyn AuditSink>,
    filter: AuditFilter,
}

impl<S, ReqBody> Service<Request<ReqBody>> for AuditService<S>
where
    S: Service<Request<ReqBody>, Response = Response> + Send + Clone + 'static,
    S::Future: Send + 'static,
    ReqBody: Send + 'static,
{
    type Response = Response;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Response, S::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), S::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut req: Request<ReqBody>) -> Self::Future {
        let audited = self.filter.matches(req.method(), req.uri().path());

        // Always insert the slot so handlers can safely extract AuditAnnotationSlot
        // regardless of whether auditing is enabled for this request.
        let slot = AuditAnnotationSlot::default();
        req.extensions_mut().insert(slot.clone());

        if !audited {
            let fut = self.inner.call(req);
            return Box::pin(fut);
        }

        let started_at = Utc::now();
        let method = req.method().to_string();

        let path_template = req
            .extensions()
            .get::<MatchedPath>()
            .map(|m| m.as_str().to_owned())
            .unwrap_or_else(|| req.uri().path().to_owned());

        let request_id = req
            .extensions()
            .get::<tower_http::request_id::RequestId>()
            .and_then(|id| id.header_value().to_str().ok())
            .map(ToOwned::to_owned);

        let traceparent = req
            .headers()
            .get("traceparent")
            .and_then(|v| v.to_str().ok())
            .map(ToOwned::to_owned);

        let (principal_id, org_path) = req
            .extensions()
            .get::<api_bones::org_context::OrganizationContext>()
            .map(|ctx| {
                let pid = ctx.principal.as_str().to_owned();
                let path = ctx
                    .org_path
                    .iter()
                    .map(|id: &api_bones::org_id::OrgId| id.to_string())
                    .collect::<Vec<_>>()
                    .join("/");
                (Some(pid), Some(path))
            })
            .unwrap_or((None, None));

        let sink = Arc::clone(&self.sink);
        let fut = self.inner.call(req);

        Box::pin(async move {
            let response = fut.await?;
            let status = response.status().as_u16();
            let duration_ms = Utc::now()
                .signed_duration_since(started_at)
                .num_milliseconds()
                .max(0) as u64;

            let annotation = slot.take();

            let event = AuditEvent {
                request_id,
                traceparent,
                principal_id,
                org_path,
                method,
                path_template,
                status,
                started_at,
                duration_ms,
                resource_type: annotation.as_ref().and_then(|a| a.resource_type.clone()),
                resource_id: annotation.as_ref().and_then(|a| a.resource_id.clone()),
                action: annotation.as_ref().and_then(|a| a.action.clone()),
                changes: annotation.and_then(|a| a.changes),
            };

            tokio::spawn(async move {
                if let Err(e) = sink.emit(event).await {
                    tracing::error!(error = %e, "audit sink emit failed");
                }
            });

            Ok(response)
        })
    }
}
