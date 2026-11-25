use std::sync::Arc;

use axum::Json;
use axum::extract::{Path as AxumPath, Query, State};
use axum::http::StatusCode;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use solaris_agent_runtime::{
    CreativePlan, CreativePlanInput, ModelRecommendation, ModelRoleKind, SkillDoc, SolarisBackend,
    SwarmRunRequest, build_local_creative_plan, generate_creative_plan, list_skills,
    prepare_swarm_request, run_qa_pass, run_swarm,
};
use solaris_core_ir::{
    AspectPreset, BrandPlaybook, EngineEvent, LogEntry, LogLevel, MediaAsset, QaReport,
    SavedCodeAsset, SwarmRunState, SwarmRunStatus, TranscriptWord,
};
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
    let mut api_key_update: Option<Option<String>> = None;
    let mut default_model_update: Option<Option<String>> = None;

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
        api_key_update = Some(Some(trimmed.to_string()));
    }

    if req.clear_default_model {
        default_model_update = Some(None);
    } else if let Some(model) = req.default_model {
        let trimmed = model.trim();
        if trimmed.is_empty() {
            default_model_update = Some(None);
        } else if trimmed.len() > 160 || trimmed.chars().any(char::is_control) {
            return Err((
                StatusCode::BAD_REQUEST,
                "invalid default_model format".to_string(),
            ));
        } else {
            default_model_update = Some(Some(trimmed.to_string()));
        }
    }

    state
        .persist_agent_credentials(api_key_update.clone(), default_model_update.clone())
        .await?;

    if let Some(next_key) = api_key_update {
        state.inner.openrouter.set_api_key(next_key);
        state.inner.model_advisor.clear_error_cooldown().await;
    }
    if let Some(next_model) = default_model_update {
        state.inner.openrouter.set_default_model(next_model);
    }

    Ok(get_agent_status(State(state)).await)
}

pub async fn clear_agent_credentials(
    State(state): State<AppState>,
) -> Result<Json<AgentStatusResponse>, (StatusCode, String)> {
    state.persist_agent_credentials(Some(None), None).await?;
    state.inner.openrouter.set_api_key(None);
    Ok(get_agent_status(State(state)).await)
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

fn default_plan_aspect() -> AspectPreset {
    AspectPreset::Vertical9x16
}

#[derive(Deserialize)]
pub struct GeneratePlanRequest {
    pub prompt: String,
    #[serde(default = "default_plan_aspect")]
    pub aspect_preset: AspectPreset,
    #[serde(default)]
    pub brand_playbook_id: Option<String>,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub model_override: Option<String>,
    #[serde(default)]
    pub refinement_note: Option<String>,
    #[serde(default)]
    pub previous_plan: Option<CreativePlan>,
}

pub async fn generate_plan(
    State(state): State<AppState>,
    Json(req): Json<GeneratePlanRequest>,
) -> Result<Json<CreativePlan>, (StatusCode, String)> {
    let prompt = req.prompt.trim();
    if prompt.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "prompt cannot be empty when generating a creative plan".to_string(),
        ));
    }
    if prompt.chars().count() > 8000 {
        return Err((
            StatusCode::BAD_REQUEST,
            "prompt exceeds maximum length of 8000 characters".to_string(),
        ));
    }

    if let Some(ref pid) = req.project_id {
        validate_safe_id(pid)?;
    }
    if let Some(ref bid) = req.brand_playbook_id
        && !bid.trim().is_empty()
    {
        validate_safe_id(bid.trim())?;
    }
    let model_override = req
        .model_override
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    if let Some(m) = model_override
        && (m.len() > 160 || m.chars().any(char::is_control))
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "invalid model_override format".to_string(),
        ));
    }
    let refinement_note = req
        .refinement_note
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    if let Some(note) = refinement_note
        && note.chars().count() > 2000
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "refinement_note exceeds maximum length of 2000 characters".to_string(),
        ));
    }

    let brand: BrandPlaybook = {
        let guard = state.inner.brands.read().await;
        req.brand_playbook_id
            .as_deref()
            .and_then(|id| guard.get(id).cloned())
            .or_else(|| guard.get("solaris-default").cloned())
            .unwrap_or_else(BrandPlaybook::solaris_default)
    };
    let saved_assets: Vec<SavedCodeAsset> = {
        let guard = state.inner.assets.read().await;
        guard.values().cloned().collect()
    };
    let media_pool: Vec<MediaAsset> = if let Some(ref pid) = req.project_id {
        let guard = state.inner.projects.read().await;
        guard
            .get(pid)
            .map(|e| e.project().media_pool.clone())
            .unwrap_or_default()
    } else {
        Vec::new()
    };

    let input = CreativePlanInput {
        prompt,
        aspect_preset: req.aspect_preset,
        brand: &brand,
        media_pool: &media_pool,
        saved_assets: &saved_assets,
        model_override,
        refinement_note,
        previous_plan: req.previous_plan.as_ref(),
    };

    let cancel = CancellationToken::new();
    let plan = match generate_creative_plan(
        &state.inner.openrouter,
        &state.inner.model_advisor,
        &input,
        &cancel,
    )
    .await
    {
        Ok(p) => p,
        Err(err) => {
            state.push_log(LogEntry {
                id: Uuid::new_v4().to_string(),
                timestamp: Utc::now(),
                level: LogLevel::Warn,
                source: "planner".to_string(),
                project_id: req.project_id.clone(),
                run_id: None,
                agent_id: Some("director-plan".to_string()),
                message: format!("Creative planner fell back to local draft: {err}"),
                details: None,
            });
            build_local_creative_plan(&input)
        }
    };

    state.push_log(LogEntry {
        id: Uuid::new_v4().to_string(),
        timestamp: Utc::now(),
        level: LogLevel::Info,
        source: "planner".to_string(),
        project_id: req.project_id,
        run_id: None,
        agent_id: Some("director-plan".to_string()),
        message: format!(
            "Generated creative plan '{}' ({}, {:.1}s, {} beats via {})",
            plan.title,
            plan.approach,
            plan.target_duration_sec,
            plan.beats.len(),
            plan.model_used
        ),
        details: None,
    });

    Ok(Json(plan))
}

pub async fn start_swarm_run(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
    Json(mut req): Json<SwarmRunRequest>,
) -> Result<(StatusCode, Json<SwarmRunStatus>), (StatusCode, String)> {
    validate_safe_id(&id)?;
    for clip_id in &req.selected_clip_ids {
        validate_safe_id(clip_id)?;
    }
    {
        let guard = state.inner.projects.read().await;
        let Some(engine) = guard.get(&id) else {
            return Err((StatusCode::NOT_FOUND, format!("project '{id}' not found")));
        };
        prepare_swarm_request(&mut req, engine.project())
            .map_err(|message| (StatusCode::BAD_REQUEST, message))?;
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
        refinement_summary: None,
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

#[derive(Deserialize, Default)]
pub struct LogsQuery {
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub level: Option<LogLevel>,
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Deserialize, Default)]
pub struct ClearLogsQuery {
    #[serde(default)]
    pub project_id: Option<String>,
}

pub async fn get_logs(
    State(state): State<AppState>,
    Query(query): Query<LogsQuery>,
) -> Result<Json<Vec<LogEntry>>, (StatusCode, String)> {
    if let Some(ref pid) = query.project_id {
        validate_safe_id(pid)?;
    }
    if query.limit == Some(0) {
        return Ok(Json(Vec::new()));
    }
    let limit = query.limit.unwrap_or(200);
    Ok(Json(state.list_logs(
        query.project_id.as_deref(),
        query.level,
        limit,
    )))
}

pub async fn clear_logs(
    State(state): State<AppState>,
    Query(query): Query<ClearLogsQuery>,
) -> Result<StatusCode, (StatusCode, String)> {
    if let Some(ref pid) = query.project_id {
        validate_safe_id(pid)?;
    }
    state.clear_logs(query.project_id.as_deref());
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::Duration;

    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request as HttpRequest, header};
    use serde_json::Value;
    use solaris_core_ir::{AudioSettings, Clip, ClipKind, Project, ProjectEngine, Transform2D};
    use tower::ServiceExt;

    use super::*;
    use crate::routes::build_router;
    use crate::state::ServerConfig;

    const REFINE_NEEDS_MODEL: &str =
        "Refinement needs an AI model. Add an OpenRouter key in AI Settings.";

    fn test_state(name: &str) -> (AppState, PathBuf) {
        let dir =
            std::env::temp_dir().join(format!("solaris_agent_route_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let state = AppState::initialize(&ServerConfig {
            workspace_dir: dir.clone(),
            ..ServerConfig::default()
        })
        .expect("init state");
        state.inner.openrouter.set_api_key(None);
        (state, dir)
    }

    async fn insert_project(state: &AppState, with_clip: bool) -> String {
        let mut project = Project::new_default("swarm-route", AspectPreset::Vertical9x16);
        if with_clip {
            let track = project
                .tracks
                .iter_mut()
                .find(|t| t.id == "v1")
                .expect("v1 track");
            track.clips.push(Clip {
                id: "hook-1".to_string(),
                label: "Hook".to_string(),
                start_sec: 0.0,
                duration_sec: 3.0,
                source_in_sec: 0.0,
                transform: Transform2D::default(),
                audio: AudioSettings::default(),
                payload: ClipKind::Media {
                    media_id: "m1".to_string(),
                    ken_burns: false,
                },
            });
        }
        let id = project.id.clone();
        state
            .inner
            .projects
            .write()
            .await
            .insert(id.clone(), ProjectEngine::new(project));
        id
    }

    async fn post_run(app: &Router, project_id: &str, body: &str) -> (StatusCode, Vec<u8>) {
        let res = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri(format!("/api/v1/projects/{project_id}/swarm/run"))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body.to_string()))
                    .expect("request"),
            )
            .await
            .expect("response");
        let status = res.status();
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .expect("body");
        (status, bytes.to_vec())
    }

    async fn await_run(app: &Router, project_id: &str, run_id: &str) -> Value {
        for _ in 0..400 {
            let res = app
                .clone()
                .oneshot(
                    HttpRequest::builder()
                        .uri(format!("/api/v1/projects/{project_id}/swarm/{run_id}"))
                        .body(Body::empty())
                        .expect("request"),
                )
                .await
                .expect("response");
            let run: Value = serde_json::from_slice(
                &axum::body::to_bytes(res.into_body(), usize::MAX)
                    .await
                    .expect("body"),
            )
            .expect("json");
            if matches!(
                run["state"].as_str(),
                Some("completed" | "failed" | "cancelled")
            ) {
                return run;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        panic!("swarm run {run_id} never finished");
    }

    #[tokio::test]
    async fn refine_without_clips_is_rejected_with_400() {
        let (state, dir) = test_state("refine_empty");
        let pid = insert_project(&state, false).await;
        let app = build_router(state);
        let (status, body) = post_run(
            &app,
            &pid,
            r#"{"prompt":"Tighten the pacing","mode":"refine"}"#,
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(
            String::from_utf8_lossy(&body).contains("Nothing to refine yet. Generate a cut first.")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn auto_with_clips_resolves_to_refine() {
        let (state, dir) = test_state("auto_refine");
        let pid = insert_project(&state, true).await;
        let app = build_router(state);
        let (status, body) = post_run(
            &app,
            &pid,
            r#"{"prompt":"Brighter grade","allow_local_fallback":false}"#,
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED);
        let accepted: Value = serde_json::from_slice(&body).expect("json");
        let run = await_run(&app, &pid, accepted["run_id"].as_str().expect("run id")).await;
        assert_eq!(run["state"], "failed");
        assert_eq!(run["error"], REFINE_NEEDS_MODEL);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn auto_without_clips_still_generates() {
        let (state, dir) = test_state("auto_generate");
        let pid = insert_project(&state, false).await;
        let app = build_router(state);
        let (status, body) = post_run(
            &app,
            &pid,
            r#"{"prompt":"Make a 30 second explainer","allow_local_fallback":false}"#,
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED);
        let accepted: Value = serde_json::from_slice(&body).expect("json");
        let run = await_run(&app, &pid, accepted["run_id"].as_str().expect("run id")).await;
        assert_eq!(run["state"], "failed");
        assert!(
            run["error"]
                .as_str()
                .unwrap_or_default()
                .contains("OPENROUTER_API_KEY is not configured")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn selected_clips_and_playhead_are_validated_before_a_run_starts() {
        let (state, dir) = test_state("refine_inputs");
        let pid = insert_project(&state, true).await;
        let app = build_router(state);
        for body in [
            r#"{"prompt":"x","selected_clip_ids":["ghost"]}"#,
            r#"{"prompt":"x","selected_clip_ids":["bad id!"]}"#,
            r#"{"prompt":"x","playhead_sec":-1.0}"#,
        ] {
            let (status, _) = post_run(&app, &pid, body).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
