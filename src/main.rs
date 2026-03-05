mod api;
mod config;
mod db;
mod executor;
mod models;
mod scheduler;

use std::net::SocketAddr;
use std::sync::Arc;

use tokio::signal;
use tokio::sync::watch;
use tower_http::trace::TraceLayer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use api::{create_router, AppState};
use config::Config;
use db::{create_pool, Repository};
use executor::Executor;
use scheduler::Scheduler;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "scheduler=info,tower_http=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Load configuration
    let config = Config::from_env();
    tracing::info!("Starting scheduler with config: {:?}", config);

    // Ensure data directory exists
    let data_dir = std::path::Path::new(&config.database_url)
        .parent()
        .and_then(|p| p.to_str())
        .map(|s| s.trim_start_matches("sqlite:"))
        .unwrap_or("./data");
    tokio::fs::create_dir_all(data_dir).await?;

    // Create database pool
    let pool = create_pool(&config.database_url).await?;
    tracing::info!("Database initialized");

    // Create repository
    let repo = Repository::new(pool);

    // Create executor
    let executor = Arc::new(
        Executor::new(config.logs_dir.clone(), config.work_dir.clone())
            .await
            .expect("Failed to create executor - is Docker running?"),
    );
    tracing::info!("Executor initialized");

    // Create and initialize scheduler
    let scheduler = Arc::new(Scheduler::new(repo.clone(), executor.clone()));
    scheduler
        .initialize()
        .await
        .expect("Failed to initialize scheduler");

    // Create shutdown channel
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    // Spawn scheduler
    let scheduler_for_task = scheduler.clone();
    let scheduler_handle = tokio::spawn(async move {
        scheduler_for_task.run(shutdown_rx).await;
    });

    // Create API state and router
    let state = AppState {
        repo,
        executor,
        scheduler,
    };
    let app = create_router(state).layer(TraceLayer::new_for_http());

    // Start server
    let addr: SocketAddr = format!("{}:{}", config.api_host, config.api_port).parse()?;
    tracing::info!("Starting server on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;

    // Run server with graceful shutdown
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal(shutdown_tx))
        .await?;

    // Wait for scheduler to finish
    let _ = scheduler_handle.await;

    tracing::info!("Shutdown complete");
    Ok(())
}

async fn shutdown_signal(shutdown_tx: watch::Sender<bool>) {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    tracing::info!("Shutdown signal received");
    let _ = shutdown_tx.send(true);
}
