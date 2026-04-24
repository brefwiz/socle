//! Test helpers shared across services built on socle.
//!
//! Enabled by the `testing` feature.

#[cfg(feature = "testing")]
pub use test_client::TestClient;

#[cfg(feature = "testing")]
pub mod app;

#[cfg(feature = "testing")]
pub use app::{TestApp, TestAppBuilder};

#[cfg(feature = "testing")]
pub mod trace;

#[cfg(feature = "testing")]
pub use trace::{CaptureExporter, SpanRecord, init_capture_tracing};

#[cfg(feature = "testing-postgres")]
pub mod postgres;

#[cfg(feature = "testing-postgres")]
pub use postgres::EphemeralPostgres;

#[cfg(feature = "testing")]
mod test_client {
    /// A thin wrapper around `reqwest::Client` pre-pointed at a local test server.
    pub struct TestClient {
        client: reqwest::Client,
        base_url: String,
    }

    impl TestClient {
        /// Create a test client pointed at `base_url`.
        pub fn new(base_url: impl Into<String>) -> Self {
            Self {
                client: reqwest::Client::new(),
                base_url: base_url.into(),
            }
        }

        /// Perform a GET request against `path`.
        pub async fn get(&self, path: &str) -> reqwest::Response {
            self.client
                .get(format!("{}{path}", self.base_url))
                .send()
                .await
                .expect("request failed")
        }

        /// Perform a POST request against `path` with a JSON body.
        pub async fn post<T: serde::Serialize>(&self, path: &str, body: &T) -> reqwest::Response {
            self.client
                .post(format!("{}{path}", self.base_url))
                .json(body)
                .send()
                .await
                .expect("request failed")
        }
    }
}
