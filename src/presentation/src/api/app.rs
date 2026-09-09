use application::{
    Audit, DeleteToDoItemCommand, GetAllToDoItemsQuery, GetDeletedToDoItemForAuditQuery,
    GetToDoItemQuery, ToDoItemService,
};
use axum::http::header::{ETAG, IF_MATCH};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
// Load-bearing despite looking unused: the `#[utoipa::path]` attributes below refer
// to `Uuid` when describing the `id` path parameter, and utoipa resolves it lazily,
// so rustc reports the import as unused. Removing it does not fail the build - it
// silently drops `schema: {format: uuid, type: string}` from those parameters in the
// generated OpenAPI document, which was caught by comparing the served document
// before and after.
#[allow(unused_imports)]
use uuid::Uuid;
use validator::Validate;

use crate::config::{JsonPayload, PathUuid, QueryPayload};
use crate::errors::HttpError;
use crate::requests::{
    parse_audit_token_header, parse_optional_delete_actor_id, CreateToDoItemRequest,
    GetAllToDoItemsQueryRequest, UpdateToDoItemRequest,
};
use crate::responses::{
    AuditToDoItemResponse, ProblemDetailsResponse, ToDoItemResponse, ToDoItemsPageResponse,
};

const TODO: &str = "todo";

/// Retrieves a paginated list of active to-do items with optional text search.
#[utoipa::path(
    get,
    path = "",
    context_path = "/api/v1/to-do-items",
    tag = TODO,
    responses(
        (status = 200, description = "List active to-do items filtered by the optional search term. Responses include X-Request-Id.", body = ToDoItemsPageResponse),
        (status = 400, description = "Validation error for blank or malformed query parameters. Responses include X-Request-Id.", body = ProblemDetailsResponse)
    ),
    params(GetAllToDoItemsQueryRequest)
)]
pub async fn get_all(
    Extension(service): Extension<ToDoItemService>,
    QueryPayload(query): QueryPayload<GetAllToDoItemsQueryRequest>,
) -> Result<Response, HttpError> {
    query.validate()?;
    query.validate_search().map_err(HttpError::bad_request)?;
    query.validate_sort().map_err(HttpError::bad_request)?;
    let handler = service.get_all_query_handler();
    let query: GetAllToDoItemsQuery = query.to_query().map_err(HttpError::bad_request)?;
    let data = ToDoItemsPageResponse::from(handler.execute(query).await?);

    Ok(Json(data).into_response())
}

/// Retrieves a to-do item by Id.
#[utoipa::path(
    get,
    path = "/{id}",
    context_path = "/api/v1/to-do-items",
    tag = TODO,
    responses(
        (status = 200, description = "Get todo item by id. Responses include X-Request-Id.", body = ToDoItemResponse),
        (status = 404, description = "Todo item not found. Responses include X-Request-Id.", body = ProblemDetailsResponse),
        (status = 500, description = "Unexpected internal error. Responses include X-Request-Id.", body = ProblemDetailsResponse)
    ),
    params(
        ("id" = Uuid, Path, description = "Id of the to-do item")
    ),
)]
pub async fn get_by_id(
    Extension(service): Extension<ToDoItemService>,
    PathUuid(id): PathUuid,
) -> Result<Response, HttpError> {
    let handler = service.get_query_handler();
    let item = handler.execute(GetToDoItemQuery::new(id)).await?;
    let etag = format_etag(item.version);
    let data = ToDoItemResponse::from(item);

    Ok(([(ETAG, etag)], Json(data)).into_response())
}

/// Creates a new to-do item.
#[utoipa::path(
    post,
    path = "",
    context_path = "/api/v1/to-do-items",
    tag = TODO,
    responses(
        (status = 201, description = "Create todo item. Responses include X-Request-Id.", body = Uuid),
        (status = 400, description = "Validation error. Responses include X-Request-Id.", body = ProblemDetailsResponse)
    ),
    request_body = CreateToDoItemRequest,
)]
pub async fn create(
    Extension(service): Extension<ToDoItemService>,
    JsonPayload(item): JsonPayload<CreateToDoItemRequest>,
) -> Result<Response, HttpError> {
    item.validate()?;
    let handler = service.create_command_handler();
    let data = handler.execute(item.to_command()).await?;

    Ok((StatusCode::CREATED, Json(data)).into_response())
}

/// Updates a to-do item by Id.
#[utoipa::path(
    put,
    path = "/{id}",
    context_path = "/api/v1/to-do-items",
    tag = TODO,
    responses(
        (status = 200, description = "Update todo item. Responses include X-Request-Id."),
        (status = 400, description = "Validation error. Responses include X-Request-Id.", body = ProblemDetailsResponse),
        (status = 404, description = "Todo item not found. Responses include X-Request-Id.", body = ProblemDetailsResponse),
        (status = 412, description = "Stale If-Match precondition. Responses include X-Request-Id.", body = ProblemDetailsResponse),
        (status = 428, description = "Missing If-Match precondition. Responses include X-Request-Id.", body = ProblemDetailsResponse),
        (status = 500, description = "Unexpected internal error. Responses include X-Request-Id.", body = ProblemDetailsResponse)
    ),
    params(
        ("id", description = "Id of the to-do item to update")
    ),
    request_body = UpdateToDoItemRequest,
)]
pub async fn update(
    Extension(service): Extension<ToDoItemService>,
    PathUuid(id): PathUuid,
    headers: HeaderMap,
    JsonPayload(item): JsonPayload<UpdateToDoItemRequest>,
) -> Result<Response, HttpError> {
    item.validate()?;
    let handler = service.update_command_handler();
    let version = parse_if_match(&headers)?;

    handler.execute(item.to_command(id, version)).await?;

    Ok(([(ETAG, format_etag(version + 1))], StatusCode::OK).into_response())
}

/// Deletes a to-do item by Id.
#[utoipa::path(
    delete,
    path = "/{id}",
    context_path = "/api/v1/to-do-items",
    tag = TODO,
    responses(
        (status = 200, description = "Delete todo item. Responses include X-Request-Id."),
        (status = 500, description = "Unexpected internal error. Responses include X-Request-Id.", body = ProblemDetailsResponse)
    ),
    params(
        ("id", description = "Id of the to-do item to delete")
    )
)]
pub async fn delete(
    Extension(service): Extension<ToDoItemService>,
    PathUuid(id): PathUuid,
    headers: HeaderMap,
) -> Result<Response, HttpError> {
    let deleted_by = parse_optional_delete_actor_id(&headers).map_err(HttpError::bad_request)?;
    let handler = service.delete_command_handler();

    handler
        .execute(DeleteToDoItemCommand::new(id, deleted_by))
        .await?;

    Ok(StatusCode::OK.into_response())
}

/// Retrieves a deleted to-do item by Id for audit purposes.
#[utoipa::path(
    get,
    path = "/{id}",
    context_path = "/api/v1/audit/to-do-items",
    tag = TODO,
    responses(
        (status = 200, description = "Get deleted todo item by id for audit. Responses include X-Request-Id.", body = AuditToDoItemResponse),
        (status = 401, description = "Missing or invalid audit token. Responses include X-Request-Id.", body = ProblemDetailsResponse),
        (status = 404, description = "Deleted todo item not found. Responses include X-Request-Id.", body = ProblemDetailsResponse),
        (status = 500, description = "Unexpected internal error. Responses include X-Request-Id.", body = ProblemDetailsResponse)
    ),
    params(
        ("id" = Uuid, Path, description = "Id of the deleted to-do item"),
        ("X-Audit-Token" = String, Header, description = "Audit access token")
    ),
)]
pub async fn get_deleted_by_id_for_audit(
    Extension(service): Extension<ToDoItemService>,
    Extension(audit): Extension<Audit>,
    PathUuid(id): PathUuid,
    headers: HeaderMap,
) -> Result<Response, HttpError> {
    let provided_token = parse_audit_token_header(&headers)
        .ok_or_else(|| HttpError::unauthorized("missing X-Audit-Token header"))?;
    let configured_token = audit
        .token
        .as_ref()
        .filter(|token| !token.trim().is_empty())
        .ok_or_else(|| HttpError::unauthorized("audit endpoint is not configured"))?;
    if configured_token != &provided_token {
        return Err(HttpError::unauthorized("invalid audit token"));
    }

    let handler = service.get_deleted_for_audit_query_handler();
    let item = handler
        .execute(GetDeletedToDoItemForAuditQuery::new(id))
        .await?;
    let data = AuditToDoItemResponse::from(item);

    Ok(Json(data).into_response())
}

fn format_etag(version: i32) -> String {
    format!("\"{version}\"")
}

#[allow(clippy::result_large_err)]
fn parse_if_match(headers: &HeaderMap) -> Result<i32, HttpError> {
    let raw = headers
        .get(IF_MATCH)
        .ok_or_else(|| HttpError::precondition_required("missing If-Match header"))?
        .to_str()
        .map_err(|_| HttpError::bad_request("If-Match header must be valid ASCII"))?;

    let normalized = raw.trim();
    if normalized == "*" {
        return Err(HttpError::bad_request(
            "If-Match '*' is not supported for optimistic locking",
        ));
    }

    let normalized = normalized
        .strip_prefix("W/")
        .unwrap_or(normalized)
        .trim()
        .trim_matches('"');

    normalized
        .parse::<i32>()
        .map_err(|_| HttpError::bad_request("If-Match header must contain an integer ETag"))
}
