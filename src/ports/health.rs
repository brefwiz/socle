//! Health port — readiness check abstraction.

use std::pin::Pin;
use std::sync::Arc;

/// Readiness check closure: called on every `GET /health/ready`.
pub type ReadinessCheckFn = Arc<
    dyn Fn() -> Pin<Box<dyn std::future::Future<Output = api_bones::health::HealthCheck> + Send>>
        + Send
        + Sync,
>;

/// Port: any component that can answer a readiness probe.
pub trait HealthProbe: Send + Sync {
    /// Name of this probe.
    fn name(&self) -> &'static str;
    /// Run the probe and return its result.
    fn check(
        &self,
    ) -> Pin<Box<dyn std::future::Future<Output = api_bones::health::HealthCheck> + Send>>;
}

/// Convert a [`HealthProbe`] into the internal `(name, ReadinessCheckFn)` pair.
pub(crate) fn probe_to_check_fn(probe: impl HealthProbe + 'static) -> (String, ReadinessCheckFn) {
    let probe = Arc::new(probe);
    let name = probe.name().to_owned();
    let f: ReadinessCheckFn = Arc::new(move || probe.check());
    (name, f)
}

#[cfg(test)]
mod tests {
    use super::*;
    use api_bones::health::{HealthCheck, HealthStatus};

    struct AlwaysPass;
    struct AlwaysFail;

    impl HealthProbe for AlwaysPass {
        fn name(&self) -> &'static str {
            "pass"
        }
        fn check(&self) -> Pin<Box<dyn std::future::Future<Output = HealthCheck> + Send>> {
            Box::pin(async { HealthCheck::pass("ok") })
        }
    }

    impl HealthProbe for AlwaysFail {
        fn name(&self) -> &'static str {
            "fail"
        }
        fn check(&self) -> Pin<Box<dyn std::future::Future<Output = HealthCheck> + Send>> {
            Box::pin(async { HealthCheck::fail("fail", "always fails") })
        }
    }

    #[tokio::test]
    async fn probe_to_check_fn_preserves_name() {
        let (name, _) = probe_to_check_fn(AlwaysPass);
        assert_eq!(name, "pass");
    }

    #[tokio::test]
    async fn probe_to_check_fn_pass_returns_pass_status() {
        let (_, check) = probe_to_check_fn(AlwaysPass);
        let result = check().await;
        assert_eq!(result.status, HealthStatus::Pass);
    }

    #[tokio::test]
    async fn probe_to_check_fn_fail_returns_fail_status() {
        let (_, check) = probe_to_check_fn(AlwaysFail);
        let result = check().await;
        assert_eq!(result.status, HealthStatus::Fail);
    }

    #[tokio::test]
    async fn readiness_check_fn_callable_multiple_times() {
        let (_, check) = probe_to_check_fn(AlwaysPass);
        assert_eq!(check().await.status, HealthStatus::Pass);
        assert_eq!(check().await.status, HealthStatus::Pass);
    }
}
