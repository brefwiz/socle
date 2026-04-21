//! Adapters — concrete implementations of the ports defined in `crate::ports`.

pub(crate) mod health;
pub(crate) mod observability;
pub(crate) mod security;

#[cfg(feature = "openapi")]
pub(crate) mod openapi {
    pub(crate) use super::observability::openapi::*;
}

pub(crate) mod cors {
    pub(crate) use super::security::cors::*;
}
