use std::time::Duration;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::response::IntoResponse;
use serde::Deserialize;
use tokio::sync::broadcast::error::RecvError;

use solaris_core_ir::EngineEvent;

use crate::state::AppState;

#[derive(Deserialize)]
pub struct WsQuery {
    pub project_id: Option<String>,
}

pub async fn ws_handler(
    ws: WebSocketUpgrade,
    Query(query): Query<WsQuery>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    ws.protocols(["solaris.v1"])
        .on_upgrade(move |socket| handle_ws(socket, state, query.project_id))
}

async fn handle_ws(mut socket: WebSocket, state: AppState, project_filter: Option<String>) {
    let mut rx = state.inner.events_tx.subscribe();
    let mut heartbeat = tokio::time::interval(Duration::from_secs(30));
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    heartbeat.tick().await; // consume immediate initial tick

    loop {
        tokio::select! {
            recv_res = rx.recv() => {
                match recv_res {
                    Ok(envelope) => {
                        if let Some(filter) = &project_filter {
                            let matches = match &envelope.payload {
                                EngineEvent::OpCommitted { project_id, .. }
                                | EngineEvent::ProjectReset { project_id, .. }
                                | EngineEvent::MediaAnalyzed { project_id, .. }
                                | EngineEvent::TrackLockChanged { project_id, .. }
                                | EngineEvent::CursorPulse { project_id, .. }
                                | EngineEvent::ExportProgress { project_id, .. }
                                | EngineEvent::SwarmProgress { project_id, .. }
                                | EngineEvent::PlayheadSeek { project_id, .. } => project_id == filter,
                                EngineEvent::LogEmitted { project_id, .. } => {
                                    project_id.as_deref().is_none_or(|pid| pid == filter)
                                }
                                // Workspace-global GFX library changes reach every Studio client.
                                EngineEvent::AssetsChanged { .. } => true,
                            };
                            if !matches {
                                continue;
                            }
                        }
                        if let Ok(text) = serde_json::to_string(&envelope)
                            && socket.send(Message::Text(text.into())).await.is_err()
                        {
                            break;
                        }
                    }
                    Err(RecvError::Lagged(skipped)) => {
                        tracing::warn!(
                            skipped,
                            project_filter = ?project_filter,
                            "websocket event stream lagged; closing socket for client resync"
                        );
                        let _ = socket.send(Message::Close(None)).await;
                        break;
                    }
                    Err(RecvError::Closed) => break,
                }
            }
            incoming = socket.recv() => {
                match incoming {
                    Some(Ok(Message::Ping(payload))) => {
                        if socket.send(Message::Pong(payload)).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(Message::Pong(_))) => {}
                    Some(Ok(Message::Close(_))) | Some(Err(_)) | None => {
                        break;
                    }
                    Some(Ok(_)) => {}
                }
            }
            _ = heartbeat.tick() => {
                if socket.send(Message::Ping(Vec::new().into())).await.is_err() {
                    break;
                }
            }
            _ = state.inner.shutdown.cancelled() => {
                let _ = socket.send(Message::Close(None)).await;
                break;
            }
        }
    }
}
