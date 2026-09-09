use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Extension;
use diesel::connection::SimpleConnection;
use infrastructure::DbPool;
use std::sync::Arc;
use tokio::task;
use tracing::warn;

const OK_STATUS: &str = "Ok";

/// Handles the health check for the application startup.
///
/// This endpoint is used to check if the application is up and running.
/// It returns a response with the current status of the application.
pub async fn startup() -> Result<Response, ServiceUnavailable> {
    Ok(OK_STATUS.into_response())
}

/// Handles the health check for the application's live status.
///
/// This endpoint is used to check if the application is currently live and accepting requests.
/// It returns a response with the current status of the application.
pub async fn live() -> Result<Response, ServiceUnavailable> {
    Ok(OK_STATUS.into_response())
}

/// Handles the health check for the application's ready status.
///
/// This endpoint is used to check if the application is currently ready to handle requests.
/// It returns a response with the current status of the application.
pub async fn ready(
    Extension(pool): Extension<Arc<DbPool>>,
) -> Result<Response, ServiceUnavailable> {
    let pool = pool.clone();

    task::spawn_blocking(move || -> Result<(), String> {
        let mut connection = pool
            .get()
            .map_err(|err| format!("failed to acquire database connection: {err}"))?;
        connection
            .batch_execute("SELECT 1;")
            .map_err(|err| format!("failed to execute readiness query: {err}"))?;
        Ok(())
    })
    .await
    .map_err(|err| service_unavailable(format!("readiness task join failure: {err}")))?
    .map_err(service_unavailable)?;

    Ok(OK_STATUS.into_response())
}

/// Error response returned when a health check cannot be satisfied.
pub struct ServiceUnavailable(String);

impl IntoResponse for ServiceUnavailable {
    fn into_response(self) -> Response {
        warn!(
            "Error encountered while processing the incoming HTTP request: {}",
            self.0
        );

        (StatusCode::SERVICE_UNAVAILABLE, self.0).into_response()
    }
}

fn service_unavailable(detail: String) -> ServiceUnavailable {
    ServiceUnavailable(detail)
}
