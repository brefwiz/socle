//! Org-scoped access policy trait and default ancestry implementation.

use api_bones::error::{ApiError, ErrorCode};
use api_bones::org_context::OrganizationContext;
use api_bones::org_id::OrgId;

// ── OrgPolicy ────────────────────────────────────────────────────────────────

/// Policy that determines whether a caller may access resources belonging to
/// a target org.
pub trait OrgPolicy: Send + Sync {
    /// Return `Ok(())` if `caller` is allowed to access `target`, or an
    /// [`ApiError`] describing the denial.
    ///
    /// # Errors
    ///
    /// Returns an [`ApiError`] if access is denied.
    fn allows(&self, caller: &OrganizationContext, target: &OrgId) -> Result<(), ApiError>;

    /// Convenience wrapper — returns a `403 Forbidden` with
    /// `detail: "cross-org access denied"` on rejection, preserving
    /// `caller.request_id` as the RFC 9457 `instance`.
    ///
    /// # Errors
    ///
    /// Returns a `403 Forbidden` [`ApiError`] if access is denied.
    fn check_target(&self, caller: &OrganizationContext, target: &OrgId) -> Result<(), ApiError> {
        self.allows(caller, target).map_err(|_| {
            ApiError::new(ErrorCode::Forbidden, "cross-org access denied")
                .with_request_id(caller.request_id.as_uuid())
        })
    }
}

// ── AncestryOrgPolicy ────────────────────────────────────────────────────────

/// Default [`OrgPolicy`] implementation.
///
/// Allows access when `target` is `caller.org_id` (self) or appears in the
/// descendant suffix of `caller.org_path` (i.e. any `OrgId` in `org_path`
/// that comes after `caller.org_id`).
///
/// All other cases (sibling, ancestor, unrelated) are denied.
pub struct AncestryOrgPolicy;

impl OrgPolicy for AncestryOrgPolicy {
    fn allows(&self, caller: &OrganizationContext, target: &OrgId) -> Result<(), ApiError> {
        if &caller.org_id == target {
            return Ok(());
        }
        // Allow access to a descendant node when org_path is extended beyond
        // org_id (i.e. the auth layer granted subtree scope).
        let self_pos = caller.org_path.iter().position(|id| id == &caller.org_id);
        if let Some(pos) = self_pos {
            if caller.org_path[pos + 1..].contains(target) {
                return Ok(());
            }
        }
        Err(ApiError::new(
            ErrorCode::Forbidden,
            "cross-org access denied",
        ))
    }
}
