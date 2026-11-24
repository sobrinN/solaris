use std::sync::Arc;

use axum::Json;
use axum::extract::{Path as AxumPath, State};
use axum::http::StatusCode;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use solaris_agent_runtime::{
    ModelRecommendation, ModelRoleKind, SkillDoc, SolarisBackend, SwarmRunRequest, list_skills,
    run_qa_pass, run_swarm,
};
use solaris_core_ir::{EngineEvent, QaReport, SwarmRunState, SwarmRunStatus, TranscriptWord};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::state::{AppState, validate_safe_id};

#[derive(Serialize)]
pub struct AgentStatusResponse {
    pub openrouter_configured: bool,
    pub default_model: Option<String>,
    pub has_whisper_filter: bool,
    pub mcp_http_path: &'static str,
    pub skills_count: usize,
}

pub async fn get_agent_status(State(state): State<AppState>) -> Json<AgentStatusResponse> {
    Json(AgentStatusResponse {
        openrouter_configured: state.inner.openrouter.is_configured(),
        default_model: state.inner.openrouter.config().default_model,
        has_whisper_filter: state.inner.capabilities.has_whisper_filter,
        mcp_http_path: "/mcp",
        skills_count: list_skills().len(),
    })
}

#[derive(Deserialize, Default)]
pub struct SetAgentCredentialsRequest {
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub default_model: Option<String>,
    #[serde(default)]
    pub clear_default_model: bool,
}

pub async fn set_agent_credentials(
    State(state): State<AppState>,
    Json(req): Json<SetAgentCredentialsRequest>,
) -> Result<Json<AgentStatusResponse>, (StatusCode, String)> {
    if let Some(raw_key) = req.api_key {
        let trimmed = raw_key.trim();
        if trimmed.is_empty() {
            return Err((
                StatusCode::BAD_REQUEST,
                "api_key cannot be empty; use DELETE /api/v1/agent/credentials to clear"
                    .to_string(),
            ));
        }
        if trimmed.len() > 512 || trimmed.chars().any(char::is_control) {
            return Err((
                StatusCode::BAD_REQUEST,
                "invalid api_key format".to_string(),
            ));
        }
        state
            .inner
            .openrouter
            .set_api_key(Some(trimmed.to_string()));
        state.inner.model_advisor.clear_error_cooldown().await;
    }

    if req.clear_default_model {
        state.inner.openrouter.set_default_model(None);
    } else if let Some(model) = req.default_model {
        let trimmed = model.trim();
        if trimmed.is_empty() {
            state.inner.openrouter.set_default_model(None);
        } else if trimmed.len() > 160 || trimmed.chars().any(char::is_control) {
            return Err((
                StatusCode::BAD_REQUEST,
                "invalid default_model format".to_string(),
            ));
        } else {
            state
                .inner
                .openrouter
                .set_default_model(Some(trimmed.to_string()));
        }
    }

    Ok(get_agent_status(State(state)).await)
}

pub async fn clear_agent_credentials(State(state): State<AppState>) -> Json<AgentStatusResponse> {
    state.inner.openrouter.set_api_key(None);
    get_agent_status(State(state)).await
}

#[derive(Serialize)]
pub struct ModelAdvisorResponse {
    pub director: ModelRecommendation,
    pub worker: ModelRecommendation,
    pub stt: ModelRecommendation,
}

pub async fn get_recommended_models(State(state): State<AppState>) -> Json<ModelAdvisorResponse> {
    let advisor = &state.inner.model_advisor;
    let (director, worker, stt) = tokio::join!(
        advisor.recommend(ModelRoleKind::DirectorMultimodal),
        advisor.recommend(ModelRoleKind::SpecialistToolWorker),
        advisor.recommend(ModelRoleKind::TranscriptionStt),
    );
    Json(ModelAdvisorResponse {
        director,
        worker,
        stt,
    })
}

#[derive(Serialize)]
pub struct SkillSummary {
    pub name: String,
    pub description: String,
    pub uri: String,
}

pub async fn get_skills() -> Json<Vec<SkillSummary>> {
    let items = list_skills()
        .into_iter()
        .map(|s: SkillDoc| SkillSummary {
            name: s.name,
            description: s.description,
            uri: s.uri,
        })
        .collect();
    Json(items)
}

pub async fn start_swarm_run(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
    Json(req): Json<SwarmRunRequest>,
) -> Result<(StatusCode, Json<SwarmRunStatus>), (StatusCode, String)> {
    validate_safe_id(&id)?;
    {
        let guard = state.inner.projects.read().await;
        if !guard.contains_key(&id) {
            return Err((StatusCode::NOT_FOUND, format!("project '{id}' not found")));
        }
    }

    let run_id = Uuid::new_v4().to_string();
    let now = Utc::now();
    let initial = SwarmRunStatus {
        run_id: run_id.clone(),
        project_id: id.clone(),
        state: SwarmRunState::Queued,
        phase_label: "Queued for Solaris Swarm".to_string(),
        model_used: None,
        used_local_fallback: !state.inner.openrouter.is_configured(),
        active_roles: Vec::new(),
        ops_committed: 0,
        summary: None,
        error: None,
        qa_report: None,
        started_at: now,
        updated_at: now,
    };

    let cancel = CancellationToken::new();
    {
        let mut runs = state.inner.swarm_runs.write().await;
        let has_active_run = runs.values().any(|r| {
            r.project_id == id
                && !matches!(
                    r.state,
                    SwarmRunState::Completed | SwarmRunState::Failed | SwarmRunState::Cancelled
                )
        });
        if has_active_run {
            return Err((
                StatusCode::CONFLICT,
                "A swarm run is already active for this project".to_string(),
            ));
        }
        if runs.len() >= 64
            && let Some(oldest_id) = runs
                .iter()
                .filter(|(_, r)| {
                    matches!(
                        r.state,
                        SwarmRunState::Completed | SwarmRunState::Failed | SwarmRunState::Cancelled
                    )
                })
                .min_by_key(|(_, r)| r.updated_at)
                .map(|(k, _)| k.clone())
        {
            runs.remove(&oldest_id);
            state.inner.swarm_cancels.write().await.remove(&oldest_id);
        }
        runs.insert(run_id.clone(), initial.clone());
    }
    state
        .inner
        .swarm_cancels
        .write()
        .await
        .insert(run_id.clone(), cancel.clone());

    state.emit(EngineEvent::SwarmProgress {
        project_id: id.clone(),
        run: initial.clone(),
    });

    let state_bg = state.clone();
    let run_id_bg = run_id.clone();
    let proj_id_bg = id.clone();

    tokio::spawn(async move {
        let backend: Arc<dyn SolarisBackend> = Arc::new(state_bg.clone());
        let openrouter = state_bg.inner.openrouter.clone();
        let advisor = state_bg.inner.model_advisor.clone();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<SwarmRunStatus>();

        let writer_state = state_bg.clone();
        let writer_rid = run_id_bg.clone();
        let writer_pid = proj_id_bg.clone();
        let writer_task = tokio::spawn(async move {
            while let Some(next_status) = rx.recv().await {
                writer_state
                    .inner
                    .swarm_runs
                    .write()
                    .await
                    .insert(writer_rid.clone(), next_status.clone());
                writer_state.emit(EngineEvent::SwarmProgress {
                    project_id: writer_pid.clone(),
                    run: next_status,
                });
            }
        });

        let final_status = run_swarm(
            backend,
            openrouter,
            advisor,
            proj_id_bg.clone(),
            run_id_bg.clone(),
            req,
            cancel,
            move |next_status| {
                let _ = tx.send(next_status);
            },
        )
        .await;

        let _ = writer_task.await;

        state_bg
            .inner
            .swarm_runs
            .write()
            .await
            .insert(run_id_bg.clone(), final_status.clone());
        state_bg
            .inner
            .swarm_cancels
            .write()
            .await
            .remove(&run_id_bg);
        state_bg.emit(EngineEvent::SwarmProgress {
            project_id: proj_id_bg,
            run: final_status,
        });
    });

    Ok((StatusCode::ACCEPTED, Json(initial)))
}

pub async fn get_swarm_run(
    State(state): State<AppState>,
    AxumPath((id, run_id)): AxumPath<(String, String)>,
) -> Result<Json<SwarmRunStatus>, (StatusCode, String)> {
    validate_safe_id(&id)?;
    let guard = state.inner.swarm_runs.read().await;
    let run = guard
        .get(&run_id)
        .filter(|r| r.project_id == id)
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                format!("swarm run '{run_id}' not found"),
            )
        })?;
    Ok(Json(run.clone()))
}

pub async fn cancel_swarm_run(
    State(state): State<AppState>,
    AxumPath((id, run_id)): AxumPath<(String, String)>,
) -> Result<StatusCode, (StatusCode, String)> {
    validate_safe_id(&id)?;
    {
        let runs = state.inner.swarm_runs.read().await;
        if !runs.get(&run_id).is_some_and(|r| r.project_id == id) {
            return Err((
                StatusCode::NOT_FOUND,
                format!("active swarm run '{run_id}' not found for project '{id}'"),
            ));
        }
    }
    let guard = state.inner.swarm_cancels.read().await;
    let Some(tok) = guard.get(&run_id) else {
        return Err((
            StatusCode::NOT_FOUND,
            format!("active swarm run '{run_id}' not found"),
        ));
    };
    tok.cancel();
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize, Default)]
pub struct TranscribeRequest {
    #[serde(default)]
    pub language: Option<String>,
}

#[derive(Serialize)]
pub struct TranscribeResponse {
    pub media_id: String,
    pub word_count: usize,
    pub words: Vec<TranscriptWord>,
}

pub async fn transcribe_media_route(
    State(state): State<AppState>,
    AxumPath((id, media_id)): AxumPath<(String, String)>,
    body: Option<Json<TranscribeRequest>>,
) -> Result<Json<TranscribeResponse>, (StatusCode, String)> {
    validate_safe_id(&id)?;
    validate_safe_id(&media_id)?;
    let lang = body.and_then(|b| b.0.language);
    let words = state
        .transcribe_media(&id, &media_id, lang)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    Ok(Json(TranscribeResponse {
        media_id,
        word_count: words.len(),
        words,
    }))
}

#[derive(Deserialize, Default)]
pub struct QaRequest {
    #[serde(default = "default_auto_fix")]
    pub auto_fix: bool,
}

fn default_auto_fix() -> bool {
    true
}

pub async fn run_project_qa(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
    body: Option<Json<QaRequest>>,
) -> Result<Json<QaReport>, (StatusCode, String)> {
    validate_safe_id(&id)?;
    {
        let guard = state.inner.projects.read().await;
        if !guard.contains_key(&id) {
            return Err((StatusCode::NOT_FOUND, format!("project '{id}' not found")));
        }
    }
    let auto_fix = body.map(|b| b.0.auto_fix).unwrap_or(true);
    let cancel = CancellationToken::new();
    let (openrouter, vision_rec) = if state.inner.openrouter.is_configured() {
        let rec = state
            .inner
            .model_advisor
            .recommend(ModelRoleKind::DirectorMultimodal)
            .await;
        (Some(&state.inner.openrouter), Some(rec))
    } else {
        (None, None)
    };
    let vision_ref = vision_rec
        .as_ref()
        .map(|r| (r.primary.as_str(), r.fallbacks.as_slice()));
    let report = run_qa_pass(&state, openrouter, vision_ref, &id, auto_fix, &cancel, None).await;
    Ok(Json(report))
}
