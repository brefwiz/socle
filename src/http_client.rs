use std::collections::HashMap;

use async_trait::async_trait;
use opentelemetry::{Context, global, propagation::Injector};
use reqwest::header::{HeaderName, HeaderValue};
use reqwest_middleware::{
    ClientBuilder as MiddlewareClientBuilder, ClientWithMiddleware, Middleware, Next,
    Result as MiddlewareResult,
};

use crate::request_id::CURRENT_REQUEST_ID;

struct HashMapInjector<'a>(&'a mut HashMap<String, String>);

impl Injector for HashMapInjector<'_> {
    fn set(&mut self, key: &str, value: String) {
        self.0.insert(key.to_owned(), value);
    }
}

struct TraceContextMiddleware;

#[async_trait]
impl Middleware for TraceContextMiddleware {
    async fn handle(
        &self,
        mut req: reqwest::Request,
        extensions: &mut http::Extensions,
        next: Next<'_>,
    ) -> MiddlewareResult<reqwest::Response> {
        let cx = Context::current();
        let mut carrier: HashMap<String, String> = HashMap::new();
        global::get_text_map_propagator(|propagator| {
            propagator.inject_context(&cx, &mut HashMapInjector(&mut carrier));
        });
        for (key, value) in carrier {
            if let (Ok(name), Ok(val)) = (
                HeaderName::from_bytes(key.as_bytes()),
                HeaderValue::from_str(&value),
            ) {
                req.headers_mut().insert(name, val);
            }
        }
        next.run(req, extensions).await
    }
}

struct RequestIdMiddleware;

#[async_trait]
impl Middleware for RequestIdMiddleware {
    async fn handle(
        &self,
        mut req: reqwest::Request,
        extensions: &mut http::Extensions,
        next: Next<'_>,
    ) -> MiddlewareResult<reqwest::Response> {
        if let Ok(id) = CURRENT_REQUEST_ID.try_with(|id| id.clone()) {
            if !id.is_empty() {
                if let Ok(val) = HeaderValue::from_str(&id) {
                    req.headers_mut().insert("x-request-id", val);
                }
            }
        }
        next.run(req, extensions).await
    }
}

/// Entry point: returns a [`ClientBuilder`] with sensible defaults.
pub fn builder() -> ClientBuilder {
    ClientBuilder {
        inner: reqwest::ClientBuilder::new(),
    }
}

/// Thin wrapper around [`reqwest::ClientBuilder`].
pub struct ClientBuilder {
    inner: reqwest::ClientBuilder,
}

impl ClientBuilder {
    pub fn timeout(mut self, duration: std::time::Duration) -> Self {
        self.inner = self.inner.timeout(duration);
        self
    }

    pub fn connect_timeout(mut self, duration: std::time::Duration) -> Self {
        self.inner = self.inner.connect_timeout(duration);
        self
    }

    pub fn user_agent(mut self, value: impl AsRef<str>) -> Self {
        self.inner = self.inner.user_agent(value.as_ref());
        self
    }

    pub fn default_headers(mut self, headers: reqwest::header::HeaderMap) -> Self {
        self.inner = self.inner.default_headers(headers);
        self
    }

    pub fn from_reqwest_builder(builder: reqwest::ClientBuilder) -> Self {
        ClientBuilder { inner: builder }
    }

    pub fn build(self) -> Result<Client, reqwest::Error> {
        let reqwest_client = self.inner.build()?;
        let client = MiddlewareClientBuilder::new(reqwest_client)
            .with(TraceContextMiddleware)
            .with(RequestIdMiddleware)
            .build();
        Ok(Client { inner: client })
    }
}

/// Cloneable wrapper around [`reqwest_middleware::ClientWithMiddleware`].
#[derive(Clone)]
pub struct Client {
    inner: ClientWithMiddleware,
}

impl Client {
    pub fn get(&self, url: impl reqwest::IntoUrl) -> reqwest_middleware::RequestBuilder {
        self.inner.get(url)
    }

    pub fn post(&self, url: impl reqwest::IntoUrl) -> reqwest_middleware::RequestBuilder {
        self.inner.post(url)
    }

    pub fn put(&self, url: impl reqwest::IntoUrl) -> reqwest_middleware::RequestBuilder {
        self.inner.put(url)
    }

    pub fn patch(&self, url: impl reqwest::IntoUrl) -> reqwest_middleware::RequestBuilder {
        self.inner.patch(url)
    }

    pub fn delete(&self, url: impl reqwest::IntoUrl) -> reqwest_middleware::RequestBuilder {
        self.inner.delete(url)
    }

    pub fn head(&self, url: impl reqwest::IntoUrl) -> reqwest_middleware::RequestBuilder {
        self.inner.head(url)
    }

    pub fn request(
        &self,
        method: reqwest::Method,
        url: impl reqwest::IntoUrl,
    ) -> reqwest_middleware::RequestBuilder {
        self.inner.request(method, url)
    }

    pub fn inner(&self) -> &ClientWithMiddleware {
        &self.inner
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Router, extract::Request as AxumRequest, routing::get};
    use std::sync::{Arc, Mutex};
    use tokio::net::TcpListener;

    async fn capture_headers(
        axum::extract::State(captured): axum::extract::State<Arc<Mutex<Vec<(String, String)>>>>,
        req: AxumRequest,
    ) -> &'static str {
        let mut guard = captured.lock().unwrap();
        for (k, v) in req.headers() {
            if let Ok(v) = v.to_str() {
                guard.push((k.to_string(), v.to_string()));
            }
        }
        "ok"
    }

    async fn start_server(captured: Arc<Mutex<Vec<(String, String)>>>) -> String {
        let app = Router::new()
            .route("/", get(capture_headers))
            .with_state(captured);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn builder_builds_successfully() {
        let client = builder().build();
        assert!(client.is_ok());
    }

    #[tokio::test]
    async fn from_reqwest_builder_preserves_configuration() {
        let rb = reqwest::ClientBuilder::new();
        let client = ClientBuilder::from_reqwest_builder(rb).build();
        assert!(client.is_ok());
    }

    #[tokio::test]
    async fn no_traceparent_without_propagator() {
        let captured: Arc<Mutex<Vec<(String, String)>>> = Arc::new(Mutex::new(vec![]));
        let url = start_server(captured.clone()).await;
        let client = builder().build().unwrap();
        client.get(&url).send().await.unwrap();
        let headers = captured.lock().unwrap();
        assert!(!headers.iter().any(|(k, _)| k == "traceparent"));
    }

    #[tokio::test]
    async fn traceparent_injected_with_propagator() {
        use opentelemetry::trace::{TraceContextExt as _, Tracer as _, TracerProvider as _};
        use opentelemetry_sdk::propagation::TraceContextPropagator;
        use opentelemetry_sdk::trace::TracerProvider;

        let provider = TracerProvider::builder().build();
        let tracer = provider.tracer("test");
        opentelemetry::global::set_text_map_propagator(TraceContextPropagator::new());

        let captured: Arc<Mutex<Vec<(String, String)>>> = Arc::new(Mutex::new(vec![]));
        let url = start_server(captured.clone()).await;
        let client = builder().build().unwrap();

        let span = tracer.start("test-span");
        let cx = opentelemetry::Context::current_with_span(span);
        let _guard = cx.attach();

        client.get(&url).send().await.unwrap();

        let headers = captured.lock().unwrap();
        assert!(
            headers.iter().any(|(k, _)| k == "traceparent"),
            "expected traceparent header, got: {headers:?}"
        );
    }

    #[tokio::test]
    async fn request_id_forwarded_when_set() {
        let captured: Arc<Mutex<Vec<(String, String)>>> = Arc::new(Mutex::new(vec![]));
        let url = start_server(captured.clone()).await;
        let client = builder().build().unwrap();

        CURRENT_REQUEST_ID
            .scope("test-req-id".to_owned(), async {
                client.get(&url).send().await.unwrap();
            })
            .await;

        let headers = captured.lock().unwrap();
        assert!(
            headers
                .iter()
                .any(|(k, v)| k == "x-request-id" && v == "test-req-id"),
            "expected x-request-id: test-req-id, got: {headers:?}"
        );
    }

    #[tokio::test]
    async fn no_request_id_when_not_set() {
        let captured: Arc<Mutex<Vec<(String, String)>>> = Arc::new(Mutex::new(vec![]));
        let url = start_server(captured.clone()).await;
        let client = builder().build().unwrap();
        client.get(&url).send().await.unwrap();
        let headers = captured.lock().unwrap();
        assert!(!headers.iter().any(|(k, _)| k == "x-request-id"));
    }
}
