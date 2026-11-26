use axum::Json;
use axum::extract::{Path as AxumPath, Query, State};
use axum::http::StatusCode;
use chrono::Utc;
use serde::{Deserialize, Serialize};

use solaris_core_ir::{
    AgentRole, AspectPreset, CommandMeta, OpResult, Project, ProjectEngine, ProjectTemplateInfo,
    SavedCodeAsset, TimelineCommand, builtin_templates, instantiate_template,
};

use crate::state::{AppState, map_core_error, validate_safe_id};

#[derive(Serialize)]
pub struct ProjectSummary {
    pub id: String,
    pub name: String,
    pub revision: u64,
    pub aspect_preset: AspectPreset,
    pub duration_sec: f64,
    pub media_count: usize,
    pub updated_at: chrono::DateTime<Utc>,
}

pub async fn list_projects(State(state): State<AppState>) -> Json<Vec<ProjectSummary>> {
    let guard = state.inner.projects.read().await;
    let mut items: Vec<ProjectSummary> = guard
        .values()
        .map(|engine| {
            let p = engine.project();
            ProjectSummary {
                id: p.id.clone(),
                name: p.name.clone(),
                revision: p.revision,
                aspect_preset: p.canvas.aspect_preset,
                duration_sec: p.duration_sec(),
                media_count: p.media_pool.len(),
                updated_at: p.updated_at,
            }
        })
        .collect();
    items.sort_by_key(|a| std::cmp::Reverse(a.updated_at));
    Json(items)
}

pub async fn list_templates() -> Json<Vec<ProjectTemplateInfo>> {
    Json(builtin_templates())
}

#[derive(Deserialize)]
pub struct CreateProjectRequest {
    name: String,
    #[serde(default)]
    aspect_preset: Option<AspectPreset>,
    #[serde(default)]
    brand_playbook_id: Option<String>,
    #[serde(default)]
    prompt: Option<String>,
    #[serde(default)]
    template_id: Option<String>,
}

pub async fn create_project(
    State(state): State<AppState>,
    Json(req): Json<CreateProjectRequest>,
) -> Result<(StatusCode, Json<Project>), (StatusCode, String)> {
    let project = if let Some(tid) = req.template_id.filter(|t| !t.trim().is_empty()) {
        let assets: Vec<SavedCodeAsset> =
            state.inner.assets.read().await.values().cloned().collect();
        instantiate_template(
            &tid,
            Some(req.name),
            req.brand_playbook_id,
            req.prompt,
            req.aspect_preset,
            &assets,
        )
        .ok_or_else(|| (StatusCode::BAD_REQUEST, format!("unknown template '{tid}'")))?
    } else {
        let mut project = Project::new_default(
            req.name,
            req.aspect_preset.unwrap_or(AspectPreset::Vertical9x16),
        );
        if let Some(brand_id) = req.brand_playbook_id {
            project.brand_playbook_id = brand_id;
        }
        if let Some(prompt) = req.prompt {
            project.prompt = prompt;
        }
        project
    };
    state.persist_project_locked(&project).await;
    let mut guard = state.inner.projects.write().await;
    guard.insert(project.id.clone(), ProjectEngine::new(project.clone()));
    Ok((StatusCode::CREATED, Json(project)))
}

pub async fn get_project(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<Project>, (StatusCode, String)> {
    validate_safe_id(&id)?;
    let guard = state.inner.projects.read().await;
    let engine = guard
        .get(&id)
        .ok_or_else(|| (StatusCode::NOT_FOUND, format!("project '{id}' not found")))?;
    Ok(Json(engine.project().clone()))
}

#[derive(Deserialize)]
pub struct UpdateProjectMetadataRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub prompt: Option<String>,
}

pub async fn update_project_metadata(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
    Json(req): Json<UpdateProjectMetadataRequest>,
) -> Result<Json<Project>, (StatusCode, String)> {
    validate_safe_id(&id)?;
    let project = {
        let mut guard = state.inner.projects.write().await;
        let engine = guard
            .get_mut(&id)
            .ok_or_else(|| (StatusCode::NOT_FOUND, format!("project '{id}' not found")))?;
        let event = engine
            .update_metadata(req.name, req.prompt)
            .map_err(map_core_error)?;
        let project = engine.project().clone();
        state.emit(event);
        project
    };
    state.persist_project_locked(&project).await;
    Ok(Json(project))
}

pub async fn delete_project(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    validate_safe_id(&id)?;
    let existed = {
        let mut guard = state.inner.projects.write().await;
        guard.remove(&id).is_some()
    };
    if !existed {
        return Err((StatusCode::NOT_FOUND, format!("project '{id}' not found")));
    }

    // Cancel and remove any swarm runs associated with this project
    let run_ids: Vec<String> = {
        let mut runs = state.inner.swarm_runs.write().await;
        let matching: Vec<String> = runs
            .iter()
            .filter(|(_, run)| run.project_id == id)
            .map(|(rid, _)| rid.clone())
            .collect();
        for rid in &matching {
            runs.remove(rid);
        }
        matching
    };
    if !run_ids.is_empty() {
        let mut cancels = state.inner.swarm_cancels.write().await;
        for rid in run_ids {
            if let Some(token) = cancels.remove(&rid) {
                token.cancel();
            }
        }
    }

    state
        .inner
        .export_jobs
        .write()
        .await
        .retain(|_, job| job.project_id != id);
    state.clear_logs(Some(&id));

    let project_file = state.project_file_path(&id);
    let uploads_dir = state.inner.workspace_dir.join("uploads").join(&id);
    // Serialize project JSON removal against any in-flight persist_project_locked
    {
        let mut revs = state.inner.persist_revisions.lock().await;
        revs.remove(&id);
        let project_id = id.clone();
        let _ = tokio::task::spawn_blocking(move || {
            if let Err(err) = std::fs::remove_file(&project_file)
                && err.kind() != std::io::ErrorKind::NotFound
            {
                tracing::warn!(
                    project_id = %project_id,
                    path = %project_file.display(),
                    "failed to remove project file on delete: {err}"
                );
            }
            if uploads_dir.is_dir()
                && let Err(err) = std::fs::remove_dir_all(&uploads_dir)
            {
                tracing::warn!(
                    project_id = %project_id,
                    path = %uploads_dir.display(),
                    "failed to remove project uploads directory on delete: {err}"
                );
            }
        })
        .await;
    }

    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct ApplyOpRequest {
    command: TimelineCommand,
    #[serde(default)]
    meta: CommandMeta,
}

#[derive(Serialize)]
pub struct ApplyOpResponse {
    result: OpResult,
    project: Project,
}

pub async fn apply_op(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
    Json(req): Json<ApplyOpRequest>,
) -> Result<Json<ApplyOpResponse>, (StatusCode, String)> {
    validate_safe_id(&id)?;
    state.validate_timeline_command(&req.command).await?;

    let mut guard = state.inner.projects.write().await;
    let engine = guard
        .get_mut(&id)
        .ok_or_else(|| (StatusCode::NOT_FOUND, format!("project '{id}' not found")))?;

    let (result, event) = engine
        .apply(req.command, req.meta)
        .map_err(map_core_error)?;

    let project = engine.project().clone();
    state.emit(event);
    drop(guard);

    state.persist_project_locked(&project).await;
    Ok(Json(ApplyOpResponse { result, project }))
}

pub async fn undo_project(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<Project>, (StatusCode, String)> {
    validate_safe_id(&id)?;
    let mut guard = state.inner.projects.write().await;
    let engine = guard
        .get_mut(&id)
        .ok_or_else(|| (StatusCode::NOT_FOUND, format!("project '{id}' not found")))?;

    let event = engine.undo().map_err(map_core_error)?;
    let project = engine.project().clone();
    state.emit(event);
    drop(guard);

    state.persist_project_locked(&project).await;
    Ok(Json(project))
}

pub async fn redo_project(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<Project>, (StatusCode, String)> {
    validate_safe_id(&id)?;
    let mut guard = state.inner.projects.write().await;
    let engine = guard
        .get_mut(&id)
        .ok_or_else(|| (StatusCode::NOT_FOUND, format!("project '{id}' not found")))?;

    let event = engine.redo().map_err(map_core_error)?;
    let project = engine.project().clone();
    state.emit(event);
    drop(guard);

    state.persist_project_locked(&project).await;
    Ok(Json(project))
}

#[derive(Deserialize)]
pub struct TrackLockRequest {
    pub agent_id: String,
    pub role: AgentRole,
}

pub async fn acquire_track_lock(
    State(state): State<AppState>,
    AxumPath((id, track_id)): AxumPath<(String, String)>,
    Json(req): Json<TrackLockRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    validate_safe_id(&id)?;
    let mut guard = state.inner.projects.write().await;
    let engine = guard
        .get_mut(&id)
        .ok_or_else(|| (StatusCode::NOT_FOUND, format!("project '{id}' not found")))?;
    let event = engine
        .acquire_track_lock(&track_id, req.agent_id, req.role)
        .map_err(map_core_error)?;
    state.emit(event);
    drop(guard);
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct ReleaseLockQuery {
    pub agent_id: String,
}

pub async fn release_track_lock(
    State(state): State<AppState>,
    AxumPath((id, track_id)): AxumPath<(String, String)>,
    Query(query): Query<ReleaseLockQuery>,
) -> Result<StatusCode, (StatusCode, String)> {
    validate_safe_id(&id)?;
    let mut guard = state.inner.projects.write().await;
    let engine = guard
        .get_mut(&id)
        .ok_or_else(|| (StatusCode::NOT_FOUND, format!("project '{id}' not found")))?;
    let event = engine
        .release_track_lock(&track_id, &query.agent_id)
        .map_err(map_core_error)?;
    state.emit(event);
    drop(guard);
    Ok(StatusCode::NO_CONTENT)
}
