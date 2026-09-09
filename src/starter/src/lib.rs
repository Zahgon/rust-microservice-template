mod observability;

use anyhow::Result;
use application::{Settings, ToDoItemService};
use axum::{middleware::from_fn, Extension, Router};
use infrastructure::PostgresToDoItemRepository;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::net::TcpListener;
use tower_http::trace::TraceLayer;
use tracing::{debug, info, warn};
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

/// The bound HTTP server, awaited to run it to completion.
pub type Server = Pin<Box<dyn Future<Output = std::io::Result<()>> + Send>>;

pub async fn run() -> Result<Server> {
    let settings = Settings::default().load()?;
    run_internal(&settings).await
}

pub async fn run_with_config(path: &str) -> Result<Server> {
    let settings = Settings::with_path(path).load()?;
    run_internal(&settings).await
}

async fn run_internal(settings: &Settings) -> Result<Server> {
    observability::init_tracing(settings)?;
    let observability_config = observability::ObservabilityConfig::from_settings(settings)?;
    let prometheus_handle = observability::init_prometheus_recorder()?;

    info!("Starting HTTP server at {}", &settings.service.http_url);
    debug!("with configuration: {:?}", &settings);

    let pool = infrastructure::configure(settings).await?;

    // Create repository with Arc for thread safety
    let repository = Arc::new(PostgresToDoItemRepository::new(&Arc::new(pool.clone())));

    // Create service with explicit command/query dependencies.
    let todo_service = ToDoItemService::new(repository.clone(), repository);
    let audit_settings = settings.audit.clone();
    let observability_settings = observability_config.clone();
    let metrics_handle = prometheus_handle.clone();

    let app = presentation::configure()
        .merge(Router::from(SwaggerUi::new("/api/v1/swagger-ui").url(
            "/api/v1/api-docs/openapi.json",
            presentation::ApiDoc::openapi(),
        )))
        .layer(TraceLayer::new_for_http())
        .layer(from_fn(observability::observability_middleware))
        .layer(Extension(observability_settings))
        .layer(Extension(metrics_handle))
        .layer(Extension(Arc::new(pool)))
        .layer(Extension(todo_service))
        .layer(Extension(audit_settings));

    let listener = TcpListener::bind(&settings.service.http_url).await?;
    let server = Box::pin(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(shutdown_signal())
            .await
    });

    Ok(server)
}

/// Resolves once the process is asked to terminate, so that in-flight requests
/// are drained rather than cut, as `HttpServer` did on `SIGTERM`.
async fn shutdown_signal() {
    #[cfg(unix)]
    let signal = async {
        use tokio::signal::unix::{signal, SignalKind};

        match signal(SignalKind::terminate()) {
            Ok(mut terminate) => {
                terminate.recv().await;
                info!("SIGTERM received; starting graceful shutdown");
            }
            Err(err) => {
                warn!("failed to install SIGTERM handler: {err}");
                std::future::pending::<()>().await;
            }
        }
    };

    #[cfg(not(unix))]
    let signal = async {
        match tokio::signal::ctrl_c().await {
            Ok(()) => info!("shutdown signal received; starting graceful shutdown"),
            Err(err) => {
                warn!("failed to install shutdown handler: {err}");
                std::future::pending::<()>().await;
            }
        }
    };

    signal.await;
}
