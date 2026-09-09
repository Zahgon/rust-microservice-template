use crate::api;
use crate::errors::HttpError;
use axum::extract::{DefaultBodyLimit, FromRequest, FromRequestParts, Path, Query, Request};
use axum::http::request::Parts;
use axum::routing::get;
use axum::{Json, Router};
use axum::http::StatusCode;
use serde::de::{DeserializeOwned, IntoDeserializer};
use serde::Deserialize;
use uuid::Uuid;

extern crate application;

const JSON_PAYLOAD_LIMIT: usize = 8 * 1024;

pub fn configure() -> Router {
    Router::new()
        .route("/metrics", get(api::metrics))
        .route("/api/v1/to-do-items", get(api::get_all).post(api::create))
        .route(
            "/api/v1/to-do-items/{id}",
            get(api::get_by_id).put(api::update).delete(api::delete),
        )
        .route(
            "/api/v1/audit/to-do-items/{id}",
            get(api::get_deleted_by_id_for_audit),
        )
        .route("/api/v1/healthz/startup", get(api::startup))
        .route("/api/v1/healthz/ready", get(api::ready))
        .route("/api/v1/healthz/live", get(api::live))
        // Axum answers an unsupported method on a known path with 405 and an Allow
        // header. The original answered 404 with an empty body and no Allow, not
        // distinguishing an unknown path from an unsupported method, so clients
        // branching on the status code see what they saw before.
        .method_not_allowed_fallback(method_not_allowed)
        .layer(json_config())
}

/// Answers an unsupported method the way the original did: 404 with an empty body.
///
/// The status code is restored, but axum still attaches its own
/// `Allow` header to this response. That header is added by the method router, which
/// sits outside the layer stack of the router owning the routes, so no `.layer` here
/// can strip it; `MethodRouter::skip_allow_header` is private in axum 0.8; and the
/// remaining options (wrapping the whole application in an extra service, or
/// promoting `tower` from a dev-dependency to a production dependency) are out of
/// proportion to deleting one header from a 404. The residual header is therefore
/// recorded as a documented minor rather than papered over.
async fn method_not_allowed() -> StatusCode {
    StatusCode::NOT_FOUND
}

/// Path extractor for a Uuid segment that fails the way the original failed.
///
/// Axum's own `Path<Uuid>` rejects an unparseable segment with 400 Bad Request and
/// a message that wraps the parse error in its own prose. The original surfaced the
/// same input as 404 Not Found carrying only the deserialiser's message as plain
/// text. The segment is therefore taken as a String and run through the same serde
/// path both frameworks use, so the message is produced by uuid's own Deserialize
/// impl rather than reconstructed by hand.
pub struct PathUuid(pub Uuid);

impl<S> FromRequestParts<S> for PathUuid
where
    S: Send + Sync,
{
    type Rejection = HttpError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let Path(raw) = Path::<String>::from_request_parts(parts, state)
            .await
            .map_err(|rejection| HttpError::path_not_found(rejection.body_text()))?;

        let id = Uuid::deserialize(raw.as_str().into_deserializer())
            .map_err(|err: serde::de::value::Error| HttpError::path_not_found(err.to_string()))?;

        Ok(Self(id))
    }
}

fn json_config() -> DefaultBodyLimit {
    DefaultBodyLimit::max(JSON_PAYLOAD_LIMIT)
}

/// JSON body extractor reporting extraction failures as problem details.
pub struct JsonPayload<T>(pub T);

impl<T, S> FromRequest<S> for JsonPayload<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = HttpError;

    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        let Json(payload) = Json::<T>::from_request(request, state)
            .await
            .map_err(HttpError::from)?;

        Ok(Self(payload))
    }
}

/// Query string extractor reporting extraction failures as problem details.
pub struct QueryPayload<T>(pub T);

impl<T, S> FromRequestParts<S> for QueryPayload<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = HttpError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let Query(payload) = Query::<T>::from_request_parts(parts, state)
            .await
            .map_err(HttpError::from)?;

        Ok(Self(payload))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::requests::{CreateToDoItemRequest, GetAllToDoItemsQueryRequest};
    use axum::body::Body;
    use axum::http::{Request as HttpRequest, StatusCode};
    use axum::response::{IntoResponse, Response};
    use axum::routing::post;
    use tower::ServiceExt;
    use validator::Validate;

    async fn json_echo(
        JsonPayload(item): JsonPayload<CreateToDoItemRequest>,
    ) -> Result<Response, HttpError> {
        item.validate()?;
        Ok(StatusCode::OK.into_response())
    }

    async fn query_echo(
        QueryPayload(query): QueryPayload<GetAllToDoItemsQueryRequest>,
    ) -> Result<Response, HttpError> {
        query.validate()?;
        query.validate_search().map_err(HttpError::bad_request)?;
        query.validate_sort().map_err(HttpError::bad_request)?;
        Ok(StatusCode::OK.into_response())
    }

    #[tokio::test]
    async fn invalid_json_payload_returns_problem_details_400() {
        let app = Router::new()
            .route("/json", post(json_echo))
            .layer(json_config());

        let request = HttpRequest::post("/json")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({
                    "title": "   ",
                    "note": "note"
                })
                .to_string(),
            ))
            .unwrap();
        let response = app.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn oversized_json_payload_returns_problem_details_400() {
        let app = Router::new()
            .route("/json", post(json_echo))
            .layer(json_config());

        let request = HttpRequest::post("/json")
            .header("content-type", "application/json")
            .body(Body::from(format!(
                "{{\"title\":\"{}\",\"note\":\"ok\"}}",
                "a".repeat(9000)
            )))
            .unwrap();
        let response = app.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn invalid_query_payload_returns_problem_details_400() {
        let app = Router::new().route("/query", get(query_echo));

        let request = HttpRequest::get("/query?page=0&page_size=20&search=test")
            .body(Body::empty())
            .unwrap();
        let response = app.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn valid_query_payload_returns_ok() {
        let app = Router::new().route("/query", get(query_echo));

        let request = HttpRequest::get("/query?page=1&page_size=10&search=todo&sort=title:desc")
            .body(Body::empty())
            .unwrap();
        let response = app.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }
}
