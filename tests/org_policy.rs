use api_bones::audit::Principal;
use api_bones::org_context::OrganizationContext;
use api_bones::org_id::OrgId;
use api_bones::request_id::RequestId;

use socle::org_policy::{AncestryOrgPolicy, OrgPolicy};

fn ctx_with_path(org_id: OrgId, org_path: Vec<OrgId>) -> OrganizationContext {
    OrganizationContext::new(org_id, Principal::system("test"), RequestId::new())
        .with_org_path(org_path)
}

#[test]
fn ancestry_allows_self() {
    let org_id = OrgId::generate();
    let ctx = ctx_with_path(org_id, vec![org_id]);
    assert!(AncestryOrgPolicy.allows(&ctx, &org_id).is_ok());
}

#[test]
fn ancestry_allows_descendant() {
    let root = OrgId::generate();
    let caller_org = OrgId::generate();
    let child_org = OrgId::generate();
    // org_path extends beyond caller_org to represent a subtree grant.
    let ctx = ctx_with_path(caller_org, vec![root, caller_org, child_org]);
    assert!(AncestryOrgPolicy.allows(&ctx, &child_org).is_ok());
}

#[test]
fn ancestry_rejects_sibling() {
    let root = OrgId::generate();
    let caller_org = OrgId::generate();
    let sibling_org = OrgId::generate();
    let ctx = ctx_with_path(caller_org, vec![root, caller_org]);
    assert!(AncestryOrgPolicy.allows(&ctx, &sibling_org).is_err());
}

#[test]
fn ancestry_rejects_ancestor() {
    let root = OrgId::generate();
    let parent = OrgId::generate();
    let caller_org = OrgId::generate();
    let ctx = ctx_with_path(caller_org, vec![root, parent, caller_org]);
    // parent is an ancestor of caller — should be denied
    assert!(AncestryOrgPolicy.allows(&ctx, &parent).is_err());
    assert!(AncestryOrgPolicy.allows(&ctx, &root).is_err());
}

#[test]
fn ancestry_rejects_unrelated_org() {
    let caller_org = OrgId::generate();
    let unrelated = OrgId::generate();
    let ctx = ctx_with_path(caller_org, vec![caller_org]);
    assert!(AncestryOrgPolicy.allows(&ctx, &unrelated).is_err());
}

#[test]
fn check_target_returns_forbidden_with_detail() {
    let caller_org = OrgId::generate();
    let other_org = OrgId::generate();
    let ctx = ctx_with_path(caller_org, vec![caller_org]);
    let err = AncestryOrgPolicy
        .check_target(&ctx, &other_org)
        .unwrap_err();
    assert_eq!(err.status_code(), 403);
    assert_eq!(err.detail, "cross-org access denied");
    assert!(err.request_id.is_some());
}

#[test]
fn check_target_ok_on_self() {
    let org_id = OrgId::generate();
    let ctx = ctx_with_path(org_id, vec![org_id]);
    assert!(AncestryOrgPolicy.check_target(&ctx, &org_id).is_ok());
}
