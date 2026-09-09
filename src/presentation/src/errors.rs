use application::ApplicationError;
use axum::extract::rejection::{JsonRejection, QueryRejection};
use axum::response::{IntoResponse, Response};
use http::StatusCode as HttpStatusCode;
use problem_details::ProblemDetails;
use serde_json::Error as SerdeError;
use std::fmt::{Display, Formatter};
use tracing::warn;
use validator::ValidationErrors;

#[derive(Debug)]
pub enum HttpError {
    Problem(ProblemDetails),
    /// A bare status code with a plain-text body.
    ///
    /// Needed because not every failure in the original was rendered as problem
    /// details: Actix reported an unparseable path segment as a 404 whose body was
    /// the raw deserialiser message, with no JSON envelope. Reproducing that shape
    /// requires a response that bypasses ProblemDetails entirely.
    PlainText(HttpStatusCode, String),
}

impl HttpError {
    pub fn bad_request(detail: impl Into<String>) -> Self {
        HttpError::Problem(
            ProblemDetails::new()
                .with_status(HttpStatusCode::BAD_REQUEST)
                .with_detail(detail.into()),
        )
    }

    pub fn internal_server_error(detail: impl Into<String>) -> Self {
        HttpError::Problem(
            ProblemDetails::new()
                .with_status(HttpStatusCode::INTERNAL_SERVER_ERROR)
                .with_title("Internal Server Error")
                .with_detail(detail.into()),
        )
    }

    pub fn precondition_failed(detail: impl Into<String>) -> Self {
        HttpError::Problem(
            ProblemDetails::new()
                .with_status(HttpStatusCode::PRECONDITION_FAILED)
                .with_title("Precondition Failed")
                .with_detail(detail.into()),
        )
    }

    pub fn precondition_required(detail: impl Into<String>) -> Self {
        HttpError::Problem(
            ProblemDetails::new()
                .with_status(HttpStatusCode::PRECONDITION_REQUIRED)
                .with_title("Precondition Required")
                .with_detail(detail.into()),
        )
    }

    /// A 404 carrying the raw path-extraction message as plain text.
    ///
    /// Matches the original, where a malformed path parameter failed extraction
    /// and surfaced as 404 Not Found rather than as a 400 with a JSON body.
    pub fn path_not_found(detail: impl Into<String>) -> Self {
        HttpError::PlainText(HttpStatusCode::NOT_FOUND, detail.into())
    }

    pub fn unauthorized(detail: impl Into<String>) -> Self {
        HttpError::Problem(
            ProblemDetails::new()
                .with_status(HttpStatusCode::UNAUTHORIZED)
                .with_title("Unauthorized")
                .with_detail(detail.into()),
        )
    }
}

impl Display for HttpError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl IntoResponse for HttpError {
    fn into_response(self) -> Response {
        warn!("Error encountered while processing the incoming HTTP request: {self}");

        match self {
            HttpError::Problem(problem) => problem.into_response(),
            HttpError::PlainText(status, body) => (status, body).into_response(),
        }
    }
}

impl From<SerdeError> for HttpError {
    fn from(err: SerdeError) -> Self {
        HttpError::bad_request(err.to_string())
    }
}

impl From<ValidationErrors> for HttpError {
    fn from(err: ValidationErrors) -> Self {
        HttpError::bad_request(err.to_string())
    }
}

impl From<JsonRejection> for HttpError {
    fn from(err: JsonRejection) -> Self {
        HttpError::bad_request(err.to_string())
    }
}

impl From<QueryRejection> for HttpError {
    fn from(err: QueryRejection) -> Self {
        HttpError::bad_request(err.to_string())
    }
}

impl From<ApplicationError> for HttpError {
    fn from(err: ApplicationError) -> Self {
        match err {
            ApplicationError::NotFound { .. } => HttpError::Problem(
                ProblemDetails::new()
                    .with_status(HttpStatusCode::NOT_FOUND)
                    .with_title("Not Found")
                    .with_detail(err.to_string()),
            ),
            ApplicationError::Conflict { .. } => HttpError::precondition_failed(err.to_string()),
            ApplicationError::Internal { .. } => {
                HttpError::internal_server_error("an internal error occurred")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use axum::response::IntoResponse;
    use uuid::Uuid;

    #[tokio::test]
    async fn maps_not_found_application_errors_to_404_problem_details() {
        let error = HttpError::from(ApplicationError::NotFound { id: Uuid::nil() });

        let response = error.into_response();

        assert_eq!(
            response.status().as_u16(),
            HttpStatusCode::NOT_FOUND.as_u16()
        );
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body = String::from_utf8(body.to_vec()).unwrap();
        assert!(body.contains("\"status\":404"));
        assert!(body.contains("todo item with id"));
    }

    #[tokio::test]
    async fn sanitizes_internal_application_errors() {
        let error = HttpError::from(ApplicationError::internal("db exploded"));

        let response = error.into_response();

        assert_eq!(
            response.status().as_u16(),
            HttpStatusCode::INTERNAL_SERVER_ERROR.as_u16()
        );
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body = String::from_utf8(body.to_vec()).unwrap();
        assert!(body.contains("an internal error occurred"));
        assert!(!body.contains("db exploded"));
    }
}
