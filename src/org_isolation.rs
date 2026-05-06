//! Org isolation extractor and enforcement middleware.
//!
//! # Middleware ordering
//!
//! See [`crate`] module doc for the recommended stack.

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::response::{IntoResponse, Response};
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use tower::{Layer, Service};

use api_bones::audit::Principal;
use api_bones::error::ApiError;
use crate::org_context::OrganizationContext;
use api_bones::org_id::{OrgId, OrgPath};
use api_bones::request_id::RequestId;

use crate::handler_error::HandlerError;

// ── OrgContextSource ─────────────────────────────────────────────────────────

/// Records which mechanism resolved the [`OrganizationContext`] for this request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OrgContextSource {
    /// Resolved from an authenticated principal extension (auth layer).
    PrincipalClaim,
    /// Resolved from the `X-Org-Id` / `X-Org-Path` headers.
    Header,
}

// ── OrgContextExtractor ───────────────────────────────────────────────────────

/// Axum extractor that resolves [`OrganizationContext`] from the request.
///
/// Resolution order:
/// 1. Principal extension (`OrganizationContext` inserted by auth layer).
/// 2. `X-Org-Id` header parsed via [`OrgId::try_from_headers`].
/// 3. Reject with `401 Unauthorized` when neither yields an org.
///
/// When both (1) and (2) are present and their `org_id` disagrees the extractor
/// rejects with `403 Forbidden` and emits a `tracing::warn!`.
///
/// On success the `OrganizationContext` and [`OrgContextSource`] are inserted
/// into request extensions so downstream middleware (e.g. [`OrgIsolationLayer`])
/// can find them.
#[derive(Debug)]
pub struct OrgContextExtractor(pub OrganizationContext);

impl<S: Send + Sync> FromRequestParts<S> for OrgContextExtractor {
    type Rejection = HandlerError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let auth_ctx = parts.extensions.get::<OrganizationContext>().cloned();
        let header_org_id = OrgId::try_from_headers(&parts.headers).ok();

        if let (Some(ctx), Some(hdr_id)) = (&auth_ctx, &header_org_id) {
            if &ctx.org_id != hdr_id {
                let req_id = ctx.request_id.as_uuid().to_string();
                tracing::warn!(
                    request_id = %req_id,
                    claim_org = %ctx.org_id,
                    header_org = %hdr_id,
                    "org isolation: principal claim and X-Org-Id header disagree"
                );
                return Err(HandlerError(ApiError::forbidden(
                    "cross-tenant request rejected",
                )));
            }
        }

        if let Some(ctx) = auth_ctx {
            parts.extensions.insert(OrgContextSource::PrincipalClaim);
            return Ok(OrgContextExtractor(ctx));
        }

        if let Some(org_id) = header_org_id {
            let org_path: Vec<OrgId> = parts
                .headers
                .get("x-org-path")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse::<OrgPath>().ok())
                .map_or_else(|| vec![org_id], OrgPath::into_inner);

            let request_id = extract_request_id_from_parts(parts);

            let ctx =
                OrganizationContext::new(org_id, Principal::system("unauthenticated"), request_id)
                    .with_org_path(org_path);

            parts.extensions.insert(ctx.clone());
            parts.extensions.insert(OrgContextSource::Header);
            return Ok(OrgContextExtractor(ctx));
        }

        Err(HandlerError(ApiError::unauthorized("missing org context")))
    }
}

fn extract_request_id_from_parts(parts: &Parts) -> RequestId {
    parts
        .extensions
        .get::<tower_http::request_id::RequestId>()
        .and_then(|id| id.header_value().to_str().ok())
        .and_then(|s| s.parse::<uuid::Uuid>().ok())
        .map(RequestId::from_uuid)
        .unwrap_or_default()
}

// ── OrgIsolationLayer ────────────────────────────────────────────────────────

/// Tower layer that short-circuits with `401 Unauthorized` when an
/// [`OrganizationContext`] extension is absent from the request.
///
/// Place this layer *after* auth middleware (or after a service that runs
/// [`OrgContextExtractor`] logic) so the extension is guaranteed to be present.
#[derive(Clone, Default)]
pub struct OrgIsolationLayer;

impl<S> Layer<S> for OrgIsolationLayer {
    type Service = OrgIsolationService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        OrgIsolationService { inner }
    }
}

/// Service produced by [`OrgIsolationLayer`].
#[derive(Clone)]
pub struct OrgIsolationService<S> {
    inner: S,
}

impl<S, ReqBody> Service<axum::http::Request<ReqBody>> for OrgIsolationService<S>
where
    S: Service<axum::http::Request<ReqBody>, Response = Response> + Send + Clone + 'static,
    S::Future: Send + 'static,
    ReqBody: Send + 'static,
{
    type Response = Response;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Response, S::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), S::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: axum::http::Request<ReqBody>) -> Self::Future {
        if req.extensions().get::<OrganizationContext>().is_none() {
            let response =
                HandlerError(ApiError::unauthorized("missing org context")).into_response();
            return Box::pin(async move { Ok(response) });
        }
        let fut = self.inner.call(req);
        Box::pin(fut)
    }
}
