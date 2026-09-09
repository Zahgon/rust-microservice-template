use axum::http::header::CONTENT_TYPE;
use axum::response::{IntoResponse, Response};
use axum::Extension;
use metrics_exporter_prometheus::PrometheusHandle;

/// Exposes Prometheus-compatible runtime metrics.
#[utoipa::path(
    get,
    path = "/metrics",
    context_path = "",
    tag = "todo",
    responses(
        (status = 200, description = "Prometheus metrics output")
    )
)]
pub async fn metrics(Extension(handle): Extension<PrometheusHandle>) -> Response {
    (
        [(CONTENT_TYPE, "text/plain; version=0.0.4")],
        handle.render(),
    )
        .into_response()
}
