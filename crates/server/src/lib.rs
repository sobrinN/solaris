use std::net::SocketAddr;
use std::time::Duration;

use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

pub mod backend;
pub mod routes;
pub mod state;

pub use routes::build_router;
pub use state::{AppState, ServerConfig, is_loopback_host, validate_bind_auth};

pub struct EmbeddedServerHandle {
    shutdown_token: CancellationToken,
    shutdown_tx: Option<oneshot::Sender<()>>,
    handle: tokio::task::JoinHandle<()>,
}

impl EmbeddedServerHandle {
    pub fn trigger_shutdown(&mut self) {
        self.shutdown_token.cancel();
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }

    pub async fn shutdown(mut self) {
        self.trigger_shutdown();
        let _ = self.handle.await;
    }

    pub async fn shutdown_with_timeout(mut self, timeout: Duration) {
        self.trigger_shutdown();
        if tokio::time::timeout(timeout, &mut self.handle)
            .await
            .is_err()
        {
            tracing::warn!(
                "embedded solaris-server did not finish shutdown within {timeout:?}; aborting task"
            );
            self.handle.abort();
        }
    }
}

pub async fn spawn_embedded(
    config: ServerConfig,
) -> Result<(SocketAddr, tokio::task::JoinHandle<()>), std::io::Error> {
    let (addr, _shutdown_tx, handle) = spawn_embedded_with_shutdown(config).await?;
    Ok((addr, handle))
}

pub async fn spawn_embedded_handle(
    config: ServerConfig,
) -> Result<(SocketAddr, EmbeddedServerHandle), std::io::Error> {
    validate_bind_auth(&config.host, config.auth_token.as_deref())?;
    let state = AppState::initialize(&config)?;
    let shutdown_token = state.inner.shutdown.clone();
    let app = build_router(state);
    let listener = TcpListener::bind((config.host.as_str(), config.port)).await?;
    let bound_addr = listener.local_addr()?;
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let token_bg = shutdown_token.clone();
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                let _ = shutdown_rx.await;
                token_bg.cancel();
            })
            .await;
    });
    Ok((
        bound_addr,
        EmbeddedServerHandle {
            shutdown_token,
            shutdown_tx: Some(shutdown_tx),
            handle,
        },
    ))
}

pub async fn spawn_embedded_with_shutdown(
    config: ServerConfig,
) -> Result<(SocketAddr, oneshot::Sender<()>, tokio::task::JoinHandle<()>), std::io::Error> {
    validate_bind_auth(&config.host, config.auth_token.as_deref())?;
    let state = AppState::initialize(&config)?;
    let shutdown_token = state.inner.shutdown.clone();
    let app = build_router(state);
    let listener = TcpListener::bind((config.host.as_str(), config.port)).await?;
    let bound_addr = listener.local_addr()?;
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                let _ = shutdown_rx.await;
                shutdown_token.cancel();
            })
            .await;
    });
    Ok((bound_addr, shutdown_tx, handle))
}
