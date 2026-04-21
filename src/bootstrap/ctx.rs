//! Bootstrap dependency-injection context passed to the user's router builder.

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::Arc;

/// Context handed to the user's router builder closure.
///
/// In addition to the typed fields, an arbitrary type map is available via
/// [`BootstrapCtx::insert`] / [`BootstrapCtx::get`]. Service-kit and other
/// Brefwiz layers use this to inject internal resources (e.g. a
/// `ConnectionSource`) without requiring groundwork to know about them.
#[derive(Clone)]
pub struct BootstrapCtx {
    pub(crate) service_name: Arc<str>,
    #[cfg(feature = "database")]
    pub(crate) db: Option<sqlx::PgPool>,
    pub(crate) extensions: HashMap<TypeId, Arc<dyn Any + Send + Sync>>,
}

impl BootstrapCtx {
    /// The service name passed to [`crate::bootstrap::ServiceBootstrap::new`].
    pub fn service_name(&self) -> &str {
        &self.service_name
    }

    /// The database pool, if `with_database` or `with_db_pool` was called.
    ///
    /// Panics if called without either — that's intentional: a missing pool is
    /// a wiring bug, not a runtime condition.
    #[cfg(feature = "database")]
    pub fn db(&self) -> &sqlx::PgPool {
        self.db
            .as_ref()
            .expect("BootstrapCtx::db called but with_database() was never invoked")
    }

    /// Store an arbitrary value in the extension map.
    ///
    /// Used by wrapper crates (e.g. service-kit) to inject types that groundwork
    /// doesn't know about. Overwrites any previously stored value of the same type.
    pub fn insert<T: Send + Sync + 'static>(&mut self, val: T) {
        self.extensions.insert(TypeId::of::<T>(), Arc::new(val));
    }

    /// Retrieve a value from the extension map by type.
    ///
    /// Returns `None` if no value of type `T` was inserted.
    pub fn get<T: Send + Sync + 'static>(&self) -> Option<Arc<T>> {
        self.extensions
            .get(&TypeId::of::<T>())
            .and_then(|arc| arc.clone().downcast::<T>().ok())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_ctx() -> BootstrapCtx {
        BootstrapCtx {
            service_name: Arc::from("svc"),
            #[cfg(feature = "database")]
            db: None,
            extensions: HashMap::new(),
        }
    }

    #[test]
    fn service_name_is_accessible() {
        assert_eq!(make_ctx().service_name(), "svc");
    }

    #[cfg(feature = "database")]
    #[test]
    #[should_panic(expected = "BootstrapCtx::db called")]
    fn db_panics_when_missing() {
        let _ = make_ctx().db();
    }

    #[test]
    fn clone_preserves_service_name() {
        assert_eq!(make_ctx().clone().service_name(), "svc");
    }

    #[test]
    fn extensions_insert_and_get() {
        let mut ctx = make_ctx();
        ctx.insert(42u32);
        ctx.insert("hello");
        assert_eq!(*ctx.get::<u32>().unwrap(), 42);
        assert_eq!(*ctx.get::<&str>().unwrap(), "hello");
        assert!(ctx.get::<u64>().is_none());
    }

    #[test]
    fn extensions_survive_clone() {
        let mut ctx = make_ctx();
        ctx.insert(99u32);
        let cloned = ctx.clone();
        assert_eq!(*cloned.get::<u32>().unwrap(), 99);
    }
}
