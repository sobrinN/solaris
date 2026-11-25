use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use chrono::Utc;
use futures_util::future::join_all;
use serde_json::{Value, json};
use solaris_core_ir::{
    AgentRole, BrandPlaybook, LogLevel, Project, SavedCodeAsset, SwarmRunState, SwarmRunStatus,
};
use tokio_util::sync::CancellationToken;

use super::llm::{emit_swarm_log, run_llm_worker_loop};
use super::qa::{QaPassOptions, run_qa_pass_with_options};
use super::refine::{
    RefineFocus, RefinementPlan, RefinementTarget, build_refine_snapshot, no_change_summary,
    refinement_agent_id, refinement_tracks,
};
use super::refine_plan::{RefineModels, build_refinement_plan, resolve_refine_models};
use super::request::{NOTHING_TO_REFINE, SwarmRunMode, timeline_has_clips};
use super::{
    SwarmRunRequest, TrackLockGuard, finish_cancelled, finish_failed,
    is_unrecoverable_openrouter_error,
};
use crate::advisor::ModelAdvisor;
use crate::backend::SolarisBackend;
use crate::error::AgentError;
use crate::openrouter::OpenRouterClient;

const REFINE_NEEDS_MODEL: &str =
    "Refinement needs an AI model. Add an OpenRouter key in AI Settings.";

pub(super) struct RefineRun<'a> {
    pub backend: &'a Arc<dyn SolarisBackend>,
    pub openrouter: &'a OpenRouterClient,
    pub advisor: &'a ModelAdvisor,
    pub project: &'a Project,
    pub brand: &'a BrandPlaybook,
    pub assets: &'a [SavedCodeAsset],
    pub req: &'a SwarmRunRequest,
    pub run_id: &'a str,
    pub cancel: &'a CancellationToken,
    pub ops_counter: &'a Arc<AtomicU32>,
}

struct RefineContext<'a> {
    backend: &'a Arc<dyn SolarisBackend>,
    openrouter: &'a OpenRouterClient,
    models: &'a RefineModels,
    run_id: &'a str,
    project: &'a Project,
    brand: &'a BrandPlaybook,
    assets: &'a [SavedCodeAsset],
    prompt: &'a str,
    snapshot: &'a Value,
    max_turns: usize,
    cancel: &'a CancellationToken,
    ops_counter: &'a Arc<AtomicU32>,
}

#[derive(Default)]
struct RoleOutcome {
    fatal: bool,
    error: Option<String>,
}

pub(super) async fn run_refinement(
    run: RefineRun<'_>,
    mut status: SwarmRunStatus,
    on_status: &impl Fn(SwarmRunStatus),
) -> SwarmRunStatus {
    let RefineRun {
        backend,
        openrouter,
        advisor,
        project,
        brand,
        assets,
        req,
        run_id,
        cancel,
        ops_counter,
    } = run;
    if !timeline_has_clips(project) {
        return finish_failed(
            status,
            NOTHING_TO_REFINE.to_string(),
            ops_counter,
            on_status,
        );
    }
    if !openrouter.is_configured() {
        return finish_failed(
            status,
            REFINE_NEEDS_MODEL.to_string(),
            ops_counter,
            on_status,
        );
    }
    if cancel.is_cancelled() {
        return finish_cancelled(status, ops_counter, on_status);
    }

    let models = resolve_refine_models(advisor, req, cancel).await;
    if cancel.is_cancelled() {
        return finish_cancelled(status, ops_counter, on_status);
    }
    let snapshot = build_refine_snapshot(
        project,
        &RefineFocus {
            selected_clip_ids: &req.selected_clip_ids,
            playhead_sec: req.playhead_sec,
            history: &req.conversation_history,
        },
    );

    status.state = SwarmRunState::Planning;
    status.model_used = Some(models.director.clone());
    status.active_roles = vec![AgentRole::Director];
    status.phase_label = "Director reviewing the cut against your request".to_string();
    status.updated_at = Utc::now();
    on_status(status.clone());

    let plan = match build_refinement_plan(
        backend,
        openrouter,
        &models,
        run_id,
        &project.id,
        &req.prompt,
        &snapshot,
        cancel,
    )
    .await
    {
        Ok(plan) => plan,
        Err(AgentError::Cancelled) => return finish_cancelled(status, ops_counter, on_status),
        Err(err) => return finish_failed(status, err.to_string(), ops_counter, on_status),
    };
    emit_swarm_log(
        backend.as_ref(),
        LogLevel::Info,
        "swarm",
        &project.id,
        run_id,
        Some("director-01"),
        format!("Refinement plan: {}", plan.summary),
        (!plan.targets.is_empty()).then(|| describe_targets(&plan)),
    );

    if plan.targets.is_empty() {
        let summary = no_change_summary(&plan.summary);
        return complete_refinement(status, summary, None, ops_counter, on_status);
    }

    let mut locked: Vec<&str> = Vec::new();
    let mut dispatch: Vec<(&RefinementTarget, Vec<&str>)> = Vec::new();
    for target in &plan.targets {
        let (locked_tracks, open): (Vec<&str>, Vec<&str>) = refinement_tracks(target.role)
            .iter()
            .copied()
            .partition(|track| is_locked(project, track));
        locked.extend(locked_tracks);
        if !open.is_empty() {
            dispatch.push((target, open));
        }
    }
    if dispatch.is_empty() {
        let summary = with_locked_note(no_change_summary(&plan.summary), &locked);
        return complete_refinement(status, summary, None, ops_counter, on_status);
    }

    status.state = SwarmRunState::ExecutingSpecialists;
    status.active_roles = dispatch.iter().map(|(target, _)| target.role).collect();
    status.phase_label = format!(
        "Refining {}",
        dispatch
            .iter()
            .map(|(target, _)| refinement_agent_id(target.role))
            .collect::<Vec<_>>()
            .join(", ")
    );
    status.updated_at = Utc::now();
    on_status(status.clone());

    let ctx = RefineContext {
        backend,
        openrouter,
        models: &models,
        run_id,
        project,
        brand,
        assets,
        prompt: &req.prompt,
        snapshot: &snapshot,
        max_turns: req.max_turns_per_worker.unwrap_or(6).clamp(1, 14),
        cancel,
        ops_counter,
    };
    let outcomes = join_all(
        dispatch
            .iter()
            .map(|(target, tracks)| run_refine_role(&ctx, target, tracks)),
    )
    .await;
    if cancel.is_cancelled() {
        return finish_cancelled(status, ops_counter, on_status);
    }

    let fatal = outcomes.iter().any(|o| o.fatal);
    let errors: Vec<String> = outcomes.iter().filter_map(|o| o.error.clone()).collect();
    if fatal || outcomes.iter().all(|o| o.error.is_some()) {
        return finish_failed(status, errors.join("; "), ops_counter, on_status);
    }

    if !req.skip_qa {
        status.state = SwarmRunState::RunningQa;
        status.phase_label =
            "Director running Stage-3 playback QA (frames, -14 LUFS & safe zones)".to_string();
        status.active_roles = vec![AgentRole::Director];
        status.updated_at = Utc::now();
        on_status(status.clone());

        let vision = Some((
            models.director.as_str(),
            models.director_fallbacks.as_slice(),
        ));
        let qa = run_qa_pass_with_options(
            backend.as_ref(),
            Some(openrouter),
            vision,
            &project.id,
            true,
            cancel,
            Some(ops_counter.as_ref()),
            QaPassOptions {
                specialist_errors: &errors,
                director_supports_reasoning: models.director_low_reasoning,
            },
        )
        .await;
        status.qa_report = Some(qa);
    }
    if cancel.is_cancelled() {
        return finish_cancelled(status, ops_counter, on_status);
    }

    let mut summary = plan.summary.clone();
    if ops_counter.load(Ordering::Relaxed) == 0 {
        summary = format!("{summary} No clips were changed.");
    }
    let summary = with_locked_note(summary, &locked);
    let error = (!errors.is_empty()).then(|| errors.join("; "));
    complete_refinement(status, summary, error, ops_counter, on_status)
}

fn complete_refinement(
    mut status: SwarmRunStatus,
    summary: String,
    error: Option<String>,
    ops_counter: &AtomicU32,
    on_status: &impl Fn(SwarmRunStatus),
) -> SwarmRunStatus {
    status.state = SwarmRunState::Completed;
    status.active_roles = Vec::new();
    status.ops_committed = ops_counter.load(Ordering::Relaxed);
    status.phase_label = summary.clone();
    status.refinement_summary = Some(summary);
    status.error = error;
    status.updated_at = Utc::now();
    on_status(status.clone());
    status
}

fn with_locked_note(summary: String, locked: &[&str]) -> String {
    if locked.is_empty() {
        summary
    } else {
        format!("{summary} Skipped locked tracks: {}.", locked.join(", "))
    }
}

fn is_locked(project: &Project, track_id: &str) -> bool {
    project
        .tracks
        .iter()
        .any(|track| track.id == track_id && track.locked)
}

fn describe_targets(plan: &RefinementPlan) -> String {
    plan.targets
        .iter()
        .map(|t| format!("{}: {}", refinement_agent_id(t.role), t.instruction))
        .collect::<Vec<_>>()
        .join("\n")
}

async fn run_refine_role(
    ctx: &RefineContext<'_>,
    target: &RefinementTarget,
    tracks: &[&str],
) -> RoleOutcome {
    let agent_id = refinement_agent_id(target.role);
    let project_id = ctx.project.id.as_str();
    let anchor = tracks
        .first()
        .map_or_else(|| "timeline".to_string(), |t| format!("timeline.track.{t}"));
    let _ = ctx
        .backend
        .emit_cursor_pulse(
            project_id,
            agent_id,
            target.role,
            &anchor,
            "refining existing clips",
        )
        .await;
    emit_swarm_log(
        ctx.backend.as_ref(),
        LogLevel::Info,
        "swarm",
        project_id,
        ctx.run_id,
        Some(agent_id),
        format!(
            "{agent_id} refining {}: {}",
            tracks.join(", "),
            target.instruction
        ),
        None,
    );
    let lock = match TrackLockGuard::acquire(
        ctx.backend.clone(),
        project_id,
        tracks,
        agent_id,
        target.role,
    )
    .await
    {
        Ok(lock) => lock,
        Err(err) => {
            return RoleOutcome {
                error: Some(format!("{agent_id}: skipped ({err})")),
                ..RoleOutcome::default()
            };
        }
    };

    let instruction = role_instruction(ctx, target, tracks);
    let result = run_llm_worker_loop(
        ctx.backend,
        ctx.openrouter,
        &ctx.models.worker,
        &ctx.models.worker_fallbacks,
        ctx.models.worker_low_reasoning,
        ctx.run_id,
        false,
        ctx.project.duration_sec(),
        ctx.project,
        ctx.brand,
        ctx.assets,
        agent_id,
        target.role,
        tracks,
        &instruction,
        ctx.max_turns,
        SwarmRunMode::Refine,
        ctx.cancel,
        ctx.ops_counter,
    )
    .await;
    lock.release().await;

    match result {
        Ok(_) | Err(AgentError::Cancelled) => RoleOutcome::default(),
        Err(err) => {
            emit_swarm_log(
                ctx.backend.as_ref(),
                LogLevel::Warn,
                "swarm",
                project_id,
                ctx.run_id,
                Some(agent_id),
                format!("{agent_id} failed: {err}"),
                None,
            );
            RoleOutcome {
                fatal: is_unrecoverable_openrouter_error(&err),
                error: Some(format!("{agent_id}: {err}")),
            }
        }
    }
}

fn role_instruction(ctx: &RefineContext<'_>, target: &RefinementTarget, tracks: &[&str]) -> String {
    let own_tracks: Vec<&Value> = ctx.snapshot["tracks"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|track| track["id"].as_str().is_some_and(|id| tracks.contains(&id)))
        .collect();
    format!(
        "Refinement request: \"{}\". Your change: {}. Edit only existing clips on your tracks, addressed by clip id. Never place new clips or GFX scenes and never re-place scenes that already exist. Your tracks: {}. Selected clip ids: {}. Playhead (sec): {}.",
        ctx.prompt,
        target.instruction,
        json!(own_tracks),
        ctx.snapshot["selected_clip_ids"],
        ctx.snapshot["playhead_sec"]
    )
}

#[cfg(test)]
mod tests;
