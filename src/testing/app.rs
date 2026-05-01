// SPDX-License-Identifier: LicenseRef-Proprietary
//! [`TestApp`] — spawn a real Axum server on an ephemeral port.

use std::net::SocketAddr;

use axum::Router;
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use super::TestClient;

/// A running test server.  Drop or call [`TestApp::shutdown`] when done.
///
/// # Examples
///
/// ```rust,no_run
/// # #[cfg(feature = "testing")]
/// # mod example {
/// use axum::{Router, routing::get};
/// use socle::testing::TestApp;
///
/// #[tokio::test]
/// async fn health_check() {
///     let router = Router::new().route("/health", get(|| async { "ok" }));
///     let app = TestApp::builder().router(router).build().await;
///     let status = app.client().get("/health").send().await.unwrap().status();
///     assert_eq!(status, 200);
/// }
/// # }
/// ```
pub struct TestApp {
    /// The address the server is listening on.
    pub addr: SocketAddr,
    shutdown_tx: Option<oneshot::Sender<()>>,
    handle: Option<JoinHandle<()>>,
}

impl TestApp {
    /// Create a [`TestAppBuilder`].
    #[must_use]
    pub fn builder() -> TestAppBuilder {
        TestAppBuilder::default()
    }

    /// Return a [`TestClient`] pre-configured with this server's base URL.
    #[must_use]
    pub fn client(&self) -> TestClient {
        TestClient::new(format!("http://{}", self.addr))
    }

    /// Send the shutdown signal and wait for the server task to finish.
    pub async fn shutdown(mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        if let Some(handle) = self.handle.take() {
            let _ = handle.await;
        }
    }
}

impl Drop for TestApp {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        if let Some(handle) = self.handle.take() {
            handle.abort();
        }
    }
}

/// Builder for [`TestApp`].
#[derive(Default)]
pub struct TestAppBuilder {
    router: Option<Router>,
}

impl TestAppBuilder {
    /// Set the Axum router the test server will serve.
    #[must_use]
    pub fn router(mut self, router: Router) -> Self {
        self.router = Some(router);
        self
    }

    /// Bind to an ephemeral port and start the server.
    ///
    /// # Panics
    ///
    /// Panics if binding the ephemeral port fails or the server encounters an error.
    pub async fn build(self) -> TestApp {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("failed to bind ephemeral port");
        let addr = listener.local_addr().expect("no local addr");

        let router = self.router.unwrap_or_default();
        let (tx, rx) = oneshot::channel::<()>();

        let handle = tokio::spawn(async move {
            axum::serve(listener, router)
                .with_graceful_shutdown(async {
                    let _ = rx.await;
                })
                .await
                .expect("test server failed");
        });

        TestApp {
            addr,
            shutdown_tx: Some(tx),
            handle: Some(handle),
        }
    }
}
