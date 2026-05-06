// SPDX-License-Identifier: MIT
//! Cross-cutting platform context bundle.
//!
//! [`OrganizationContext`] carries the tenant, principal, request-id, roles,
//! and an optional opaque attestation in a single, cheap-to-clone bundle.
//! Downstream services consume this type instead of threading
//! `(org_id, principal)` pairs through every function.
//!
//! These types were previously exported by `api-bones` (removed in v5.0.0)
//! and now live here in `socle`.

use std::sync::Arc;

use api_bones::audit::Principal;
use api_bones::org_id::OrgId;
use api_bones::request_id::RequestId;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Role
// ---------------------------------------------------------------------------

/// Authorization role identifier.
///
/// A lightweight, cloneable wrapper around a role name string.
/// Roles are typically used in [`OrganizationContext`] to authorize
/// operations on behalf of a principal.
#[derive(Clone, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub struct Role(#[serde(with = "arc_str_serde")] Arc<str>);

impl Role {
    /// Construct a `Role` from a string reference.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use socle::org_context::Role;
    ///
    /// let admin = Role::from("admin");
    /// assert_eq!(admin.as_str(), "admin");
    /// ```
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for Role {
    fn from(s: &str) -> Self {
        Self(Arc::from(s))
    }
}

impl From<String> for Role {
    fn from(s: String) -> Self {
        Self(Arc::from(s.as_str()))
    }
}

impl std::fmt::Display for Role {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

mod arc_str_serde {
    use std::sync::Arc;

    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub(super) fn serialize<S: Serializer>(v: &Arc<str>, s: S) -> Result<S::Ok, S::Error> {
        v.as_ref().serialize(s)
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Arc<str>, D::Error> {
        let s = String::deserialize(d)?;
        Ok(Arc::from(s.as_str()))
    }
}

// ---------------------------------------------------------------------------
// RoleScope
// ---------------------------------------------------------------------------

/// Scope at which a role binding applies.
#[derive(Clone, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub enum RoleScope {
    /// Applies to exactly this org node only.
    Self_,
    /// Applies to this org and all its descendants.
    Subtree,
    /// Applies to exactly the named org (cross-org delegation).
    Specific(OrgId),
}

// ---------------------------------------------------------------------------
// RoleBinding
// ---------------------------------------------------------------------------

/// An authorization role paired with the scope at which it is valid.
#[derive(Clone, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub struct RoleBinding {
    /// The role being granted.
    pub role: Role,
    /// The org scope over which this binding applies.
    pub scope: RoleScope,
}

// ---------------------------------------------------------------------------
// AttestationKind
// ---------------------------------------------------------------------------

/// Kind of attestation token or credential.
///
/// Describes the format and origin of the raw bytes in [`Attestation::raw`].
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub enum AttestationKind {
    /// Biscuit capability token
    Biscuit,
    /// JWT token
    Jwt,
    /// API key
    ApiKey,
    /// mTLS certificate
    Mtls,
}

// ---------------------------------------------------------------------------
// Attestation
// ---------------------------------------------------------------------------

/// Opaque attestation / credential bundle.
///
/// Carries the raw bytes of a credential token (JWT, Biscuit, API key, etc.)
/// along with metadata about its kind.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Attestation {
    /// The kind of attestation
    pub kind: AttestationKind,
    /// The raw attestation bytes
    pub raw: Vec<u8>,
}

// ---------------------------------------------------------------------------
// OrganizationContext
// ---------------------------------------------------------------------------

/// Platform context bundle — org, principal, request-id, roles, org-path, attestation.
///
/// Carries the cross-cutting request context (tenant ID, actor identity,
/// request tracing ID, authorization roles, org-path, and optional credential) in a
/// single, cheap-to-clone value. Avoids threading `(org_id, principal)`
/// pairs separately through every function and middleware layer.
///
/// # Examples
///
/// ```rust
/// use socle::org_context::{OrganizationContext, Role, RoleBinding, RoleScope, Attestation, AttestationKind};
/// use api_bones::{OrgId, Principal, RequestId};
///
/// let org_id = OrgId::generate();
/// let principal = Principal::system("test");
/// let request_id = RequestId::new();
///
/// let ctx = OrganizationContext::new(org_id, principal, request_id)
///     .with_roles(vec![RoleBinding { role: Role::from("admin"), scope: RoleScope::Self_ }])
///     .with_attestation(Attestation {
///         kind: AttestationKind::Jwt,
///         raw: vec![1, 2, 3],
///     });
///
/// assert_eq!(ctx.roles.len(), 1);
/// assert!(ctx.attestation.is_some());
/// ```
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct OrganizationContext {
    /// Tenant ID
    pub org_id: OrgId,
    /// Actor identity
    pub principal: Principal,
    /// Request tracing ID
    pub request_id: RequestId,
    /// Authorization roles
    pub roles: Vec<RoleBinding>,
    /// Org path from root to the acting org (inclusive). Empty = platform scope.
    #[serde(default)]
    pub org_path: Vec<OrgId>,
    /// Optional credential/attestation
    pub attestation: Option<Attestation>,
}

impl OrganizationContext {
    /// Construct a new context with org, principal, and request-id.
    ///
    /// Roles default to an empty vec, `org_path` to empty, attestation to `None`.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use socle::org_context::OrganizationContext;
    /// use api_bones::{OrgId, Principal, RequestId};
    ///
    /// let ctx = OrganizationContext::new(
    ///     OrgId::generate(),
    ///     Principal::system("test"),
    ///     RequestId::new(),
    /// );
    ///
    /// assert!(ctx.roles.is_empty());
    /// assert!(ctx.org_path.is_empty());
    /// assert!(ctx.attestation.is_none());
    /// ```
    #[must_use]
    pub fn new(org_id: OrgId, principal: Principal, request_id: RequestId) -> Self {
        Self {
            org_id,
            principal,
            request_id,
            roles: Vec::new(),
            org_path: Vec::new(),
            attestation: None,
        }
    }

    /// Set the roles on this context (builder-style).
    ///
    /// # Examples
    ///
    /// ```rust
    /// use socle::org_context::{OrganizationContext, Role, RoleBinding, RoleScope};
    /// use api_bones::{OrgId, Principal, RequestId};
    ///
    /// let ctx = OrganizationContext::new(
    ///     OrgId::generate(),
    ///     Principal::system("test"),
    ///     RequestId::new(),
    /// ).with_roles(vec![RoleBinding { role: Role::from("editor"), scope: RoleScope::Self_ }]);
    ///
    /// assert_eq!(ctx.roles.len(), 1);
    /// ```
    #[must_use]
    pub fn with_roles(mut self, roles: Vec<RoleBinding>) -> Self {
        self.roles = roles;
        self
    }

    /// Set the org path on this context (builder-style).
    ///
    /// # Examples
    ///
    /// ```rust
    /// use socle::org_context::OrganizationContext;
    /// use api_bones::{OrgId, Principal, RequestId};
    ///
    /// let org_id = OrgId::generate();
    /// let ctx = OrganizationContext::new(
    ///     org_id,
    ///     Principal::system("test"),
    ///     RequestId::new(),
    /// ).with_org_path(vec![org_id]);
    ///
    /// assert!(!ctx.org_path.is_empty());
    /// ```
    #[must_use]
    pub fn with_org_path(mut self, org_path: Vec<OrgId>) -> Self {
        self.org_path = org_path;
        self
    }

    /// Set the attestation on this context (builder-style).
    ///
    /// # Examples
    ///
    /// ```rust
    /// use socle::org_context::{OrganizationContext, Attestation, AttestationKind};
    /// use api_bones::{OrgId, Principal, RequestId};
    ///
    /// let ctx = OrganizationContext::new(
    ///     OrgId::generate(),
    ///     Principal::system("test"),
    ///     RequestId::new(),
    /// ).with_attestation(Attestation {
    ///     kind: AttestationKind::ApiKey,
    ///     raw: vec![42],
    /// });
    ///
    /// assert!(ctx.attestation.is_some());
    /// ```
    #[must_use]
    pub fn with_attestation(mut self, attestation: Attestation) -> Self {
        self.attestation = Some(attestation);
        self
    }
}
