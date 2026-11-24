use std::path::PathBuf;

use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tracing_subscriber::EnvFilter;

use solaris_media_engine::S3StorageConfig;
use solaris_server::{AppState, ServerConfig, build_router, is_loopback_host, validate_bind_auth};

async fn shutdown_signal(shutdown: CancellationToken) {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut sig) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            sig.recv().await;
        } else {
            std::future::pending::<()>().await;
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {}
        _ = terminate => {}
    }
    tracing::info!("shutdown signal received; draining solaris-server connections");
    shutdown.cancel();
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let host = std::env::var("SOLARIS_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    let port = std::env::var("SOLARIS_PORT")
        .ok()
        .and_then(|p| p.parse::<u16>().ok())
        .unwrap_or(41731);
    let workspace_dir = std::env::var("SOLARIS_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(".solaris-data"));
    let auth_token = std::env::var("SOLARIS_AUTH_TOKEN")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    validate_bind_auth(&host, auth_token.as_deref())
        .expect("SOLARIS_AUTH_TOKEN is required when binding outside loopback");

    let s3_config = S3StorageConfig::from_env().expect("invalid SOLARIS_S3_* configuration");
    let allow_external_media_reads = std::env::var("SOLARIS_ALLOW_EXTERNAL_READS")
        .ok()
        .map(|v| v.trim().to_ascii_lowercase())
        .filter(|v| !v.is_empty())
        .map(|v| matches!(v.as_str(), "1" | "true" | "yes"))
        .unwrap_or_else(|| is_loopback_host(&host));
    let static_dir = std::env::var("SOLARIS_STATIC_DIR")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .map(PathBuf::from);
    let cors_origins = std::env::var("SOLARIS_CORS_ORIGINS")
        .ok()
        .map(|raw| {
            raw.split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let config = ServerConfig {
        host: host.clone(),
        port,
        workspace_dir,
        allow_external_media_reads,
        auth_token,
        openrouter_api_key: None,
        s3_config,
        static_dir,
        cors_origins,
    };

    let state = AppState::initialize(&config).expect("failed to initialize solaris server state");
    let shutdown = state.inner.shutdown.clone();
    let app = build_router(state);

    let listener = TcpListener::bind((host.as_str(), port))
        .await
        .expect("failed to bind solaris-server listener");
    let bound = listener
        .local_addr()
        .expect("failed to read bound socket address");

    tracing::info!("solaris-server listening on http://{bound}");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal(shutdown))
        .await
        .expect("solaris-server exited unexpectedly");
}
