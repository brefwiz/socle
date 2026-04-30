//! Rate limiting middleware backed by [`governor`] (GCRA algorithm).
//!
//! The in-memory store uses a keyed `RateLimiter` with a `DashMap` backend,
//! making it safe and efficient under concurrent load. Consumers needing
//! distributed rate limiting (Postgres, Redis) should implement their own
//! tower layer and register it via the escape-hatch builder method.

#[cfg(feature = "ratelimit-memory")]
use std::future::Future;
#[cfg(feature = "ratelimit")]
use std::net::SocketAddr;
#[cfg(feature = "ratelimit-memory")]
use std::pin::Pin;
#[cfg(feature = "ratelimit-memory")]
use std::sync::Arc;
#[cfg(feature = "ratelimit-memory")]
use std::task::{Context, Poll};
#[cfg(feature = "ratelimit-memory")]
use std::time::Duration;

#[cfg(feature = "ratelimit")]
use axum::extract::ConnectInfo;
#[cfg(any(feature = "ratelimit", feature = "ratelimit-memory"))]
use axum::http::{Request, StatusCode};
#[cfg(any(feature = "ratelimit", feature = "ratelimit-memory"))]
use axum::response::{IntoResponse, Response};
#[cfg(feature = "ratelimit-memory")]
use tower::{Layer, Service};

// ── Backend config ────────────────────────────────────────────────────────────

/// In-process GCRA rate limiter configuration.
///
/// Consumers needing distributed rate limiting (Postgres, Redis) should
/// implement their own tower layer and register it via the builder's
/// escape-hatch method.
#[cfg(feature = "ratelimit")]
#[derive(Debug, Clone)]
pub struct RateLimitBackend {
    /// Max requests per window.
    pub limit: u32,
    /// Window duration in seconds.
    pub window_secs: u64,
}

// ── Key extraction ────────────────────────────────────────────────────────────

/// Determines what gets rate-limited.
#[cfg(feature = "ratelimit")]
#[derive(Debug, Clone, Default)]
pub enum RateLimitExtractor {
    /// Remote IP address (reads `ConnectInfo` extension; falls back to
    /// `x-forwarded-for` → `x-real-ip` → `"unknown"`).
    #[default]
    Ip,
    /// Arbitrary header (case-insensitive name).
    Header(String),
}

#[cfg(feature = "ratelimit")]
impl RateLimitExtractor {
    fn extract<B>(&self, req: &Request<B>) -> String {
        match self {
            Self::Ip => {
                if let Some(ConnectInfo(addr)) = req.extensions().get::<ConnectInfo<SocketAddr>>() {
                    return addr.ip().to_string();
                }
                if let Some(v) = req.headers().get("x-forwarded-for")
                    && let Ok(s) = v.to_str()
                {
                    return s.split(',').next().unwrap_or(s).trim().to_string();
                }
                if let Some(v) = req.headers().get("x-real-ip")
                    && let Ok(s) = v.to_str()
                {
                    return s.to_string();
                }
                "unknown".to_string()
            }
            Self::Header(name) => extract_header(req, name),
        }
    }
}

#[cfg(feature = "ratelimit")]
fn extract_header<B>(req: &Request<B>, name: &str) -> String {
    req.headers()
        .get(name)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("unknown")
        .to_string()
}

// ── Governor-backed in-memory limiter ─────────────────────────────────────────

#[cfg(feature = "ratelimit-memory")]
type KeyedLimiter = governor::RateLimiter<
    String,
    governor::state::keyed::DefaultKeyedStateStore<String>,
    governor::clock::DefaultClock,
>;

#[cfg(feature = "ratelimit-memory")]
fn build_limiter(limit: u32, window_secs: u64) -> KeyedLimiter {
    // GCRA quota: replenish 1 token every (window_secs / limit) seconds,
    // with a burst ceiling of `limit`. This allows up to `limit` requests
    // in any `window_secs`-second window at steady state.
    let period = Duration::from_secs(window_secs) / limit;
    let quota = governor::Quota::with_period(period)
        .expect("quota period must be non-zero")
        .allow_burst(std::num::NonZeroU32::new(limit).expect("rate limit must be > 0"));
    governor::RateLimiter::keyed(quota)
}

// ── Tower Layer ───────────────────────────────────────────────────────────────

/// Tower [`Layer`] that applies GCRA rate limiting via the `governor` crate.
#[cfg(feature = "ratelimit-memory")]
#[derive(Clone)]
pub struct RateLimitLayer {
    limiter: Arc<KeyedLimiter>,
    extractor: RateLimitExtractor,
    limit: u32,
}

#[cfg(feature = "ratelimit-memory")]
impl RateLimitLayer {
    pub(crate) fn new_memory(limit: u32, window_secs: u64, extractor: RateLimitExtractor) -> Self {
        Self {
            limiter: Arc::new(build_limiter(limit, window_secs)),
            extractor,
            limit,
        }
    }
}

#[cfg(feature = "ratelimit-memory")]
impl<S> Layer<S> for RateLimitLayer {
    type Service = RateLimitService<S>;
    fn layer(&self, inner: S) -> Self::Service {
        RateLimitService {
            inner,
            limiter: self.limiter.clone(),
            extractor: self.extractor.clone(),
            limit: self.limit,
        }
    }
}

/// Tower [`Service`] produced by [`RateLimitLayer`].
#[cfg(feature = "ratelimit-memory")]
#[derive(Clone)]
pub struct RateLimitService<S> {
    inner: S,
    limiter: Arc<KeyedLimiter>,
    extractor: RateLimitExtractor,
    limit: u32,
}

#[cfg(feature = "ratelimit-memory")]
type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;

#[cfg(feature = "ratelimit-memory")]
impl<S, B> Service<Request<B>> for RateLimitService<S>
where
    S: Service<Request<B>, Response = Response> + Clone + Send + 'static,
    S::Future: Send + 'static,
    S::Error: Send + 'static,
    B: Send + 'static,
{
    type Response = Response;
    type Error = S::Error;
    type Future = BoxFuture<Result<Response, S::Error>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<B>) -> Self::Future {
        let key = self.extractor.extract(&req);
        match self.limiter.check_key(&key) {
            Ok(()) => {
                let mut inner = self.inner.clone();
                std::mem::swap(&mut inner, &mut self.inner);
                Box::pin(async move { inner.call(req).await })
            }
            Err(not_until) => {
                use governor::clock::{Clock as _, MonotonicClock};
                let retry_after = not_until
                    .wait_time_from(MonotonicClock.now())
                    .as_secs()
                    .max(1);
                let limit = self.limit;
                Box::pin(async move { Ok(too_many_requests(limit, retry_after)) })
            }
        }
    }
}

#[cfg(feature = "ratelimit-memory")]
fn too_many_requests(limit: u32, retry_after_secs: u64) -> Response {
    use std::time::{SystemTime, UNIX_EPOCH};
    let reset = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() + retry_after_secs)
        .unwrap_or(retry_after_secs);
    let info = api_bones::ratelimit::RateLimitInfo::new(limit.into(), 0, reset)
        .retry_after(retry_after_secs);
    let body = serde_json::json!({
        "type": "about:blank",
        "title": "Too Many Requests",
        "status": 429,
        "detail": "Rate limit exceeded. Retry after the indicated number of seconds.",
        "rate_limit": info,
    });
    let mut res = axum::Json(body).into_response();
    *res.status_mut() = StatusCode::TOO_MANY_REQUESTS;
    info.inject_headers(res.headers_mut());
    res
}

#[cfg(all(test, feature = "ratelimit-memory"))]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};

    fn make_layer(limit: u32, window_secs: u64) -> RateLimitLayer {
        RateLimitLayer::new_memory(limit, window_secs, RateLimitExtractor::Ip)
    }

    async fn ok_handler(_req: Request<Body>) -> Result<Response, std::convert::Infallible> {
        Ok(axum::http::Response::builder()
            .status(200)
            .body(Body::empty())
            .unwrap())
    }

    fn build_svc(
        limit: u32,
        window_secs: u64,
    ) -> impl tower::Service<Request<Body>, Response = Response, Error = std::convert::Infallible>
    {
        use tower::ServiceBuilder;
        ServiceBuilder::new()
            .layer(make_layer(limit, window_secs))
            .service(tower::service_fn(ok_handler))
    }

    async fn call_n(
        svc: &mut impl tower::Service<
            Request<Body>,
            Response = Response,
            Error = std::convert::Infallible,
        >,
        n: usize,
    ) -> Response {
        let mut last = axum::http::Response::default();
        for _ in 0..n {
            let req = Request::builder().uri("/").body(Body::empty()).unwrap();
            last = tower::ServiceExt::ready(svc)
                .await
                .unwrap()
                .call(req)
                .await
                .unwrap();
        }
        last
    }

    #[tokio::test]
    async fn allows_requests_within_limit() {
        let mut svc = build_svc(5, 60);
        let resp = call_n(&mut svc, 5).await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn rejects_over_limit() {
        let mut svc = build_svc(2, 60);
        call_n(&mut svc, 2).await;
        let req = Request::builder().uri("/").body(Body::empty()).unwrap();
        let resp = tower::ServiceExt::ready(&mut svc)
            .await
            .unwrap()
            .call(req)
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
    }

    #[tokio::test]
    async fn too_many_requests_response_has_rate_limit_headers() {
        let mut svc = build_svc(1, 60);
        call_n(&mut svc, 1).await;
        let req = Request::builder().uri("/").body(Body::empty()).unwrap();
        let resp = tower::ServiceExt::ready(&mut svc)
            .await
            .unwrap()
            .call(req)
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
        assert!(resp.headers().contains_key("x-ratelimit-limit"));
        assert!(resp.headers().contains_key("x-ratelimit-remaining"));
        assert!(resp.headers().contains_key("x-ratelimit-reset"));
        assert!(resp.headers().contains_key("retry-after"));
    }

    #[tokio::test]
    async fn extractor_ip_falls_back_to_forwarded_for() {
        let layer = RateLimitLayer::new_memory(1, 60, RateLimitExtractor::Ip);
        let mut svc = tower::ServiceBuilder::new()
            .layer(layer)
            .service(tower::service_fn(ok_handler));
        let req = Request::builder()
            .uri("/")
            .header("x-forwarded-for", "10.0.0.1")
            .body(Body::empty())
            .unwrap();
        let resp = tower::ServiceExt::ready(&mut svc)
            .await
            .unwrap()
            .call(req)
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let req = Request::builder()
            .uri("/")
            .header("x-forwarded-for", "10.0.0.1")
            .body(Body::empty())
            .unwrap();
        let resp = tower::ServiceExt::ready(&mut svc)
            .await
            .unwrap()
            .call(req)
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
    }

    #[tokio::test]
    async fn extractor_header_uses_named_header() {
        let layer =
            RateLimitLayer::new_memory(1, 60, RateLimitExtractor::Header("x-tenant-id".into()));
        let mut svc = tower::ServiceBuilder::new()
            .layer(layer)
            .service(tower::service_fn(ok_handler));
        let req = Request::builder()
            .uri("/")
            .header("x-tenant-id", "tenant-a")
            .body(Body::empty())
            .unwrap();
        let resp = tower::ServiceExt::ready(&mut svc)
            .await
            .unwrap()
            .call(req)
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let req = Request::builder()
            .uri("/")
            .header("x-tenant-id", "tenant-a")
            .body(Body::empty())
            .unwrap();
        let resp = tower::ServiceExt::ready(&mut svc)
            .await
            .unwrap()
            .call(req)
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
    }

    #[tokio::test]
    async fn different_keys_have_independent_limits() {
        let layer =
            RateLimitLayer::new_memory(1, 60, RateLimitExtractor::Header("x-tenant-id".into()));
        let mut svc = tower::ServiceBuilder::new()
            .layer(layer)
            .service(tower::service_fn(ok_handler));
        let req_a = Request::builder()
            .uri("/")
            .header("x-tenant-id", "a")
            .body(Body::empty())
            .unwrap();
        let req_b = Request::builder()
            .uri("/")
            .header("x-tenant-id", "b")
            .body(Body::empty())
            .unwrap();
        let resp_a = tower::ServiceExt::ready(&mut svc)
            .await
            .unwrap()
            .call(req_a)
            .await
            .unwrap();
        let resp_b = tower::ServiceExt::ready(&mut svc)
            .await
            .unwrap()
            .call(req_b)
            .await
            .unwrap();
        assert_eq!(resp_a.status(), StatusCode::OK);
        assert_eq!(resp_b.status(), StatusCode::OK);
    }

    #[test]
    fn extractor_missing_header_returns_unknown() {
        let ext = RateLimitExtractor::Header("x-missing".into());
        let req = Request::builder().uri("/").body(Body::empty()).unwrap();
        assert_eq!(ext.extract(&req), "unknown");
    }
}
