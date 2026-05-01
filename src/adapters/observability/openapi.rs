//! `OpenAPI` adapter — merge health paths and mount Swagger UI.

use axum::Router;

#[cfg(feature = "openapi")]
pub(crate) fn mount_openapi(
    router: Router,
    api: utoipa::openapi::OpenApi,
    spec_path: &str,
    ui_path: &str,
) -> Router {
    use utoipa_swagger_ui::SwaggerUi;
    router.merge(SwaggerUi::new(ui_path.to_string()).url(spec_path.to_string(), api))
}

#[cfg(feature = "openapi")]
pub(crate) fn merge_health_paths(
    mut api: utoipa::openapi::OpenApi,
    health_path: &str,
) -> utoipa::openapi::OpenApi {
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

    api.paths
        .paths
        .entry(live_path)
        .and_modify(|item| {
            item.merge_operations(PathItem::new(HttpMethod::Get, liveness_op.clone()));
        })
        .or_insert_with(|| PathItem::new(HttpMethod::Get, liveness_op));

    api.paths
        .paths
        .entry(ready_path)
        .and_modify(|item| {
            item.merge_operations(PathItem::new(HttpMethod::Get, readiness_op.clone()));
        })
        .or_insert_with(|| PathItem::new(HttpMethod::Get, readiness_op));

    if !api
        .tags
        .as_deref()
        .unwrap_or(&[])
        .iter()
        .any(|t| t.name == "health")
    {
        let tags = api.tags.get_or_insert_with(Vec::new);
        tags.push(Tag::new("health"));
    }

    api
}
