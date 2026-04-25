use axum::http::Method;
use socle::AuditFilter;

#[test]
fn default_includes_mutating_methods() {
    let f = AuditFilter::default();
    assert!(f.matches(&Method::POST, "/orders"));
    assert!(f.matches(&Method::PUT, "/orders/1"));
    assert!(f.matches(&Method::PATCH, "/orders/1"));
    assert!(f.matches(&Method::DELETE, "/orders/1"));
}

#[test]
fn default_excludes_get() {
    let f = AuditFilter::default();
    assert!(!f.matches(&Method::GET, "/orders"));
}

#[test]
fn default_excludes_healthz() {
    let f = AuditFilter::default();
    assert!(!f.matches(&Method::POST, "/healthz"));
    assert!(!f.matches(&Method::POST, "/readyz"));
    assert!(!f.matches(&Method::POST, "/livez"));
}

#[test]
fn default_excludes_status_suffix() {
    let f = AuditFilter::default();
    assert!(!f.matches(&Method::POST, "/orders/status"));
    assert!(!f.matches(&Method::GET, "/orders/status"));
}

#[test]
fn custom_include_path_overrides_method_check() {
    let f = AuditFilter::new().include_path("/admin/reload");
    assert!(f.matches(&Method::GET, "/admin/reload"));
}

#[test]
fn custom_exclude_path_prefix() {
    let f = AuditFilter::new().exclude_path_prefix("/internal");
    assert!(!f.matches(&Method::POST, "/internal/sync"));
}
