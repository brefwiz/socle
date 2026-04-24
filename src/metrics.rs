//! RED metrics middleware — `http_server_requests_total` and
//! `http_server_request_duration_seconds` recorded per route/method/status.
//!
//! Enabled by the `metrics` feature. Add [`MetricsLayer`] to your axum router
//! to record RED metrics for every HTTP request.
//!
//! # Custom counters
//!
//! ```rust,no_run
//! # #[cfg(feature = "metrics")]
//! # {
//! let hits = socle::metrics::counter("billing_invoices_generated_total");
//! hits.add(1, &[]);
//! # }
//! ```

use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
    time::Instant,
};

use axum::{body::Body, extract::MatchedPath};
use opentelemetry::{
    KeyValue, global,
    metrics::{Counter, Histogram, MeterProvider as _},
};
use opentelemetry_sdk::metrics::SdkMeterProvider;
use tower::{Layer, Service};

// ── Path normalisation ────────────────────────────────────────────────────────

fn normalize_path(path: &str) -> String {
    path.split('/')
        .map(|seg| {
            if seg.parse::<u64>().is_ok() || is_uuid_like(seg) {
                "{id}"
            } else {
                seg
            }
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn is_uuid_like(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 36
        && b[8] == b'-'
        && b[13] == b'-'
        && b[18] == b'-'
        && b[23] == b'-'
        && b.iter()
            .enumerate()
            .all(|(i, &c)| matches!(i, 8 | 13 | 18 | 23) || c.is_ascii_hexdigit())
}

// ── Layer ─────────────────────────────────────────────────────────────────────

/// Tower layer that records RED metrics for every HTTP request.
///
/// The layer owns a dedicated [`SdkMeterProvider`] built from the supplied
/// Prometheus registry. It also installs this provider as the global OTel
/// meter provider so that [`counter`] helpers in handlers work out of the box.
#[derive(Clone)]
pub struct MetricsLayer {
    requests: Counter<u64>,
    duration: Histogram<f64>,
    _provider: SdkMeterProvider,
}

impl MetricsLayer {
    /// Build the layer from an existing Prometheus registry.
    pub fn new(registry: prometheus::Registry) -> Result<Self, crate::Error> {
        let exporter = opentelemetry_prometheus::exporter()
            .with_registry(registry)
            .build()
            .map_err(|e| crate::Error::Telemetry(format!("prometheus exporter: {e}")))?;

        let provider = SdkMeterProvider::builder().with_reader(exporter).build();

        global::set_meter_provider(provider.clone());

        let meter = provider.meter("socle");
        let requests = meter
            .u64_counter("http_server_requests_total")
            .with_description("Total number of HTTP requests")
            .build();
        let duration = meter
            .f64_histogram("http_server_request_duration_seconds")
            .with_description("HTTP request duration in seconds")
            .with_boundaries(vec![
                0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
            ])
            .build();
        Ok(Self {
            requests,
            duration,
            _provider: provider,
        })
    }
}

impl<S> Layer<S> for MetricsLayer {
    type Service = MetricsService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        MetricsService {
            inner,
            requests: self.requests.clone(),
            duration: self.duration.clone(),
        }
    }
}

// ── Service ───────────────────────────────────────────────────────────────────

/// Inner service produced by [`MetricsLayer`].
#[derive(Clone)]
pub struct MetricsService<S> {
    inner: S,
    requests: Counter<u64>,
    duration: Histogram<f64>,
}

impl<S, ReqBody> Service<axum::http::Request<ReqBody>> for MetricsService<S>
where
    S: Service<axum::http::Request<ReqBody>, Response = axum::http::Response<Body>>
        + Clone
        + Send
        + 'static,
    S::Future: Send + 'static,
    ReqBody: Send + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: axum::http::Request<ReqBody>) -> Self::Future {
        let method = req.method().to_string();
        let route = req
            .extensions()
            .get::<MatchedPath>()
            .map(|mp| mp.as_str().to_owned())
            .unwrap_or_else(|| normalize_path(req.uri().path()));

        let start = Instant::now();
        let requests = self.requests.clone();
        let duration = self.duration.clone();
        let future = self.inner.call(req);

        Box::pin(async move {
            let resp = future.await?;
            let elapsed = start.elapsed().as_secs_f64();
            let status = resp.status().as_u16().to_string();
            let labels = [
                KeyValue::new("method", method),
                KeyValue::new("route", route),
                KeyValue::new("status", status),
            ];
            requests.add(1, &labels);
            duration.record(elapsed, &labels);
            Ok(resp)
        })
    }
}

// ── Public helper ─────────────────────────────────────────────────────────────

/// Create (or retrieve) a named [`Counter<u64>`] from the global OTel meter.
pub fn counter(name: &'static str) -> Counter<u64> {
    global::meter("socle").u64_counter(name).build()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_path_leaves_static_segments() {
        assert_eq!(normalize_path("/api/health"), "/api/health");
    }

    #[test]
    fn normalize_path_replaces_numeric_id() {
        assert_eq!(normalize_path("/users/42/orders"), "/users/{id}/orders");
    }

    #[test]
    fn normalize_path_replaces_uuid() {
        assert_eq!(
            normalize_path("/users/550e8400-e29b-41d4-a716-446655440000"),
            "/users/{id}"
        );
    }

    #[test]
    fn normalize_path_preserves_non_id_hex() {
        assert_eq!(normalize_path("/git/abc123"), "/git/abc123");
    }
}
