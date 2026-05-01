//! OpenAPI 3.0.3 helpers for `axum` + `utoipa` + `progenitor` consumers.
//!
//! # Examples
//!
//! ```rust,no_run
//! # #[cfg(feature = "openapi")]
//! # mod example {
//! use socle::openapi::{BearerAuthAddon, merge_health_paths, to_3_0_pretty_json};
//! use utoipa::OpenApi as _;
//!
//! #[derive(utoipa::OpenApi)]
//! #[openapi(modifiers(&BearerAuthAddon))]
//! struct ApiDoc;
//!
//! fn export_spec() -> String {
//!     let mut doc = ApiDoc::openapi();
//!     merge_health_paths(&mut doc, "/health");
//!     to_3_0_pretty_json(&doc).expect("serialisation failed")
//! }
//! # }
//! ```

use utoipa::openapi::OpenApi;

/// Merge a standard health endpoint path into `doc`.
///
/// Adds `{health_path}/live` and `{health_path}/ready` operations and a
/// `health` tag if not already present (idempotent).
pub fn merge_health_paths(doc: &mut OpenApi, health_path: &str) {
    use utoipa::openapi::path::{HttpMethod, OperationBuilder, PathItem};
    use utoipa::openapi::response::ResponseBuilder;
    use utoipa::openapi::tag::Tag;

    let liveness_op = OperationBuilder::new()
        .operation_id(Some("liveness"))
        .tag("health")
        .summary(Some("Liveness probe"))
        .description(Some(
            "Returns `200 OK` when the process is alive. \
             Always succeeds as long as the server is running.",
        ))
        .response(
            "200",
            ResponseBuilder::new()
                .description("Service is alive")
                .build(),
        )
        .build();

    let readiness_op = OperationBuilder::new()
        .operation_id(Some("readiness"))
        .tag("health")
        .summary(Some("Readiness probe"))
        .description(Some(
            "Returns `200 OK` when all registered readiness checks pass, \
             `503 Service Unavailable` when any check fails.",
        ))
        .response(
            "200",
            ResponseBuilder::new()
                .description("Service is ready")
                .build(),
        )
        .response(
            "503",
            ResponseBuilder::new()
                .description("Service is not ready — one or more checks failed")
                .build(),
        )
        .build();

    let live_path = format!("{health_path}/live");
    let ready_path = format!("{health_path}/ready");

    doc.paths
        .paths
        .entry(live_path)
        .and_modify(|item| {
            item.merge_operations(PathItem::new(HttpMethod::Get, liveness_op.clone()));
        })
        .or_insert_with(|| PathItem::new(HttpMethod::Get, liveness_op));

    doc.paths
        .paths
        .entry(ready_path)
        .and_modify(|item| {
            item.merge_operations(PathItem::new(HttpMethod::Get, readiness_op.clone()));
        })
        .or_insert_with(|| PathItem::new(HttpMethod::Get, readiness_op));

    if !doc
        .tags
        .as_deref()
        .unwrap_or(&[])
        .iter()
        .any(|t| t.name == "health")
    {
        let tags = doc.tags.get_or_insert_with(Vec::new);
        tags.push(Tag::new("health"));
    }
}

/// Rewrite every `nullable: true` array schema node in `value` for progenitor
/// compatibility (`OpenAPI` 3.1 → 3.0 style).
pub fn rewrite_nullable_for_progenitor(value: &mut serde_json::Value) {
    rewrite_nullable_recursive(value);
}

/// Serialise `doc` as a pretty-printed `OpenAPI` **3.0.3** JSON string.
///
/// # Errors
///
/// Returns a serialization error if the document cannot be serialized to JSON.
pub fn to_3_0_pretty_json(doc: &OpenApi) -> serde_json::Result<String> {
    let json = serde_json::to_string_pretty(doc)?;
    let mut val: serde_json::Value = serde_json::from_str(&json)?;
    coerce_boolean_and_2020_schemas(&mut val);
    rewrite_nullable_recursive(&mut val);
    if let Some(obj) = val.as_object_mut() {
        obj.insert(
            "openapi".to_string(),
            serde_json::Value::String("3.0.3".to_string()),
        );
        if let Some(info) = obj.get_mut("info")
            && let Some(license) = info.as_object_mut().and_then(|o| o.get_mut("license"))
            && let Some(lic_obj) = license.as_object_mut()
        {
            lic_obj.remove("identifier");
        }
    }
    serde_json::to_string_pretty(&val)
}

/// utoipa [`utoipa::Modify`] plugin that registers a `BearerAuth` HTTP Bearer security scheme.
pub struct BearerAuthAddon;

impl utoipa::Modify for BearerAuthAddon {
    fn modify(&self, openapi: &mut OpenApi) {
        use utoipa::openapi::security::{Http, HttpAuthScheme, SecurityScheme};
        let components = openapi.components.get_or_insert_with(Default::default);
        components.add_security_scheme(
            "BearerAuth",
            SecurityScheme::Http(Http::new(HttpAuthScheme::Bearer)),
        );
    }
}

fn rewrite_nullable_recursive(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            let rewrite_type = if let Some(type_val) = map.get("type") {
                if let Some(arr) = type_val.as_array() {
                    let non_null: Vec<&str> = arr
                        .iter()
                        .filter_map(|v| v.as_str())
                        .filter(|s| *s != "null")
                        .collect();
                    let has_null = arr.iter().any(|v| v.as_str() == Some("null"));
                    if non_null.len() == 1 {
                        Some((non_null[0].to_string(), has_null))
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            };
            if let Some((t, has_null)) = rewrite_type {
                map.insert("type".to_string(), serde_json::Value::String(t));
                if has_null {
                    map.insert("nullable".to_string(), serde_json::Value::Bool(true));
                }
            }

            for of_key in &["oneOf", "anyOf"] {
                let rewrite = if let Some(variants) = map.get(*of_key) {
                    if let Some(arr) = variants.as_array() {
                        let has_null_arm = arr.iter().any(|v| {
                            v.as_object()
                                .and_then(|o| o.get("type"))
                                .and_then(|t| t.as_str())
                                == Some("null")
                        });
                        if has_null_arm {
                            let remaining: Vec<serde_json::Value> = arr
                                .iter()
                                .filter(|v| {
                                    v.as_object()
                                        .and_then(|o| o.get("type"))
                                        .and_then(|t| t.as_str())
                                        != Some("null")
                                })
                                .cloned()
                                .collect();
                            Some(remaining)
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    None
                };
                if let Some(remaining) = rewrite {
                    map.insert("nullable".to_string(), serde_json::Value::Bool(true));
                    if remaining.len() == 1 {
                        let arm = remaining.into_iter().next().unwrap();
                        map.remove(*of_key);
                        if let serde_json::Value::Object(arm_map) = arm {
                            for (k, v) in arm_map {
                                map.entry(k).or_insert(v);
                            }
                        }
                    } else {
                        map.insert(of_key.to_string(), serde_json::Value::Array(remaining));
                    }
                    break;
                }
            }

            map.remove("propertyNames");

            if let Some(examples) = map.remove("examples")
                && let Some(arr) = examples.as_array()
                && let Some(first) = arr.first()
            {
                map.entry("example".to_string())
                    .or_insert_with(|| first.clone());
            }

            for v in map.values_mut() {
                rewrite_nullable_recursive(v);
            }
        }
        serde_json::Value::Array(arr) => {
            for v in arr.iter_mut() {
                rewrite_nullable_recursive(v);
            }
        }
        _ => {}
    }
}

fn coerce_boolean_and_2020_schemas(val: &mut serde_json::Value) {
    match val {
        serde_json::Value::Object(map) => {
            let schema_keys = [
                "additionalProperties",
                "additionalItems",
                "items",
                "not",
                "if",
                "then",
                "else",
                "contains",
                "propertyNames",
            ];

            for key in &schema_keys {
                if let Some(&serde_json::Value::Bool(b)) = map.get(*key) {
                    if b {
                        map.insert(
                            key.to_string(),
                            serde_json::Value::Object(serde_json::Map::default()),
                        );
                    } else {
                        map.remove(*key);
                    }
                }
            }

            map.remove("unevaluatedProperties");
            map.remove("unevaluatedItems");
            map.remove("$schema");

            for v in map.values_mut() {
                coerce_boolean_and_2020_schemas(v);
            }
        }
        serde_json::Value::Array(arr) => {
            for v in arr.iter_mut() {
                coerce_boolean_and_2020_schemas(v);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use utoipa::openapi::OpenApiBuilder;

    fn empty_api() -> OpenApi {
        OpenApiBuilder::new()
            .info(
                utoipa::openapi::InfoBuilder::new()
                    .title("test")
                    .version("0.1.0")
                    .build(),
            )
            .build()
    }

    #[test]
    fn to_3_0_pretty_json_version_field() {
        let api = empty_api();
        let result = to_3_0_pretty_json(&api).unwrap();
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(v["openapi"], "3.0.3");
    }

    #[test]
    fn rewrite_nullable_for_progenitor_round_trip() {
        let mut val = json!({ "type": ["string", "null"] });
        rewrite_nullable_for_progenitor(&mut val);
        assert_eq!(val["type"], "string");
        assert_eq!(val["nullable"], true);
    }

    #[test]
    fn merge_health_paths_idempotency() {
        let mut api = empty_api();
        merge_health_paths(&mut api, "/health");
        merge_health_paths(&mut api, "/health");
        let count = api
            .tags
            .as_deref()
            .unwrap_or(&[])
            .iter()
            .filter(|t| t.name == "health")
            .count();
        assert_eq!(count, 1);
        assert!(api.paths.paths.contains_key("/health/live"));
        assert!(api.paths.paths.contains_key("/health/ready"));
    }

    #[test]
    fn bearer_auth_addon_modify_presence() {
        use utoipa::Modify as _;
        let mut api = empty_api();
        BearerAuthAddon.modify(&mut api);
        let components = api.components.unwrap();
        assert!(components.security_schemes.contains_key("BearerAuth"));
    }
}
