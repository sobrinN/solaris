use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use futures_util::future::join_all;
use serde_json::{Value, json};
use solaris_core_ir::{
    AgentRole, BrandPlaybook, LogLevel, Project, SavedCodeAsset, SwarmRunState, SwarmRunStatus,
};
use tokio_util::sync::CancellationToken;

use super::llm::{emit_swarm_log, run_llm_worker_loop};
use super::progress::{SwarmProgressTracker, lerp_progress};
use super::qa::{QaPassOptions, run_qa_pass_with_options};
use super::refine::{
    RefineFocus, RefinementPlan, RefinementTarget, build_refine_snapshot, no_change_summary,
    refinement_agent_id, refinement_tracks,
};
use super::refine_plan::{RefineModels, build_refinement_plan, resolve_refine_models};
use super::request::{NOTHING_TO_REFINE, SwarmRunMode, timeline_has_clips};
use super::{SwarmRunRequest, TrackLockGuard, is_unrecoverable_openrouter_error};
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
    tracker: &Arc<SwarmProgressTracker>,
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
        return tracker.finish_failed(NOTHING_TO_REFINE.to_string());
    }
    if !openrouter.is_configured() {
        return tracker.finish_failed(REFINE_NEEDS_MODEL.to_string());
    }
    if cancel.is_cancelled() {
        return tracker.finish_cancelled();
    }

    let models = resolve_refine_models(advisor, req, cancel).await;
    if cancel.is_cancelled() {
        return tracker.finish_cancelled();
    }

    // Wait briefly if any newly attached media is still analyzing
    let mut current_project = project.clone();
    let needs_analysis = |p: &Project| {
        p.media_pool.iter().any(|m| {
            !m.path.is_empty()
                && matches!(
                    m.kind,
                    solaris_core_ir::MediaKind::Video | solaris_core_ir::MediaKind::Audio
                )
                && !m.intelligence.analyzed
        })
    };
    if needs_analysis(&current_project) {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
        while needs_analysis(&current_project) && tokio::time::Instant::now() < deadline {
            tokio::select! {
                _ = cancel.cancelled() => return tracker.finish_cancelled(),
                _ = tokio::time::sleep(std::time::Duration::from_millis(250)) => {}
            }
            if let Ok(updated) = backend.get_project(&project.id).await {
                current_project = updated;
            } else {
                break;
            }
        }
    }

    // Auto-transcribe untranscribed speech assets if captions/subtitles/transcript are requested or T1 is empty
    let prompt_lower = req.prompt.to_ascii_lowercase();
    let t1_empty = current_project
        .tracks
        .iter()
        .find(|t| t.id == "t1")
        .is_none_or(|t| t.clips.is_empty());
    let wants_stt = prompt_lower.contains("caption")
        || prompt_lower.contains("subtitle")
        || prompt_lower.contains("transcrib")
        || (t1_empty
            && (prompt_lower.contains("better")
                || prompt_lower.contains("enhance")
                || prompt_lower.contains("edit")
                || prompt_lower.contains("polish")));
    if wants_stt {
        let speech_ids: Vec<String> = current_project
            .media_pool
            .iter()
            .filter(|m| {
                !m.path.is_empty()
                    && m.intelligence.transcript.is_empty()
                    && ((m.kind == solaris_core_ir::MediaKind::Video && m.intelligence.has_audio)
                        || (m.kind == solaris_core_ir::MediaKind::Audio
                            && matches!(m.role_hint.as_str(), "voice" | "voiceover" | "audio")))
            })
            .map(|m| m.id.clone())
            .take(3)
            .collect();
        if !speech_ids.is_empty() {
            tracker.advance(
                0.06,
                Some(format!(
                    "Transcribing {} speech asset(s) before planning",
                    speech_ids.len()
                )),
            );
            let mut any_transcribed = false;
            for mid in speech_ids {
                if cancel.is_cancelled() {
                    return tracker.finish_cancelled();
                }
                match backend
                    .transcribe_media(&current_project.id, &mid, None)
                    .await
                {
                    Ok(words) if !words.is_empty() => {
                        any_transcribed = true;
                        emit_swarm_log(
                            backend.as_ref(),
                            LogLevel::Info,
                            "swarm",
                            &current_project.id,
                            run_id,
                            Some("director-01"),
                            format!("Transcribed {} word(s) for media {mid}", words.len()),
                            None,
                        );
                    }
                    Ok(_) => {}
                    Err(err) => {
                        emit_swarm_log(
                            backend.as_ref(),
                            LogLevel::Warn,
                            "swarm",
                            &current_project.id,
                            run_id,
                            Some("director-01"),
                            format!("Auto-transcription skipped for {mid}: {err}"),
                            None,
                        );
                    }
                }
            }
            if any_transcribed && let Ok(updated) = backend.get_project(&current_project.id).await {
                current_project = updated;
            }
        }
    }

    let snapshot = build_refine_snapshot(
        &current_project,
        &RefineFocus {
            selected_clip_ids: &req.selected_clip_ids,
            playhead_sec: req.playhead_sec,
            history: &req.conversation_history,
        },
    );

    tracker.update(|s| {
        s.state = SwarmRunState::Planning;
        s.model_used = Some(models.director.clone());
        s.active_roles = vec![AgentRole::Director];
        s.progress = 0.10;
        s.phase_label = "Director reviewing the cut against your request".to_string();
    });

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
        Err(AgentError::Cancelled) => return tracker.finish_cancelled(),
        Err(err) => return tracker.finish_failed(err.to_string()),
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
        return complete_refinement(tracker, summary, None);
    }

    let mut locked: Vec<&str> = Vec::new();
    let mut dispatch: Vec<(&RefinementTarget, Vec<&str>)> = Vec::new();
    for target in &plan.targets {
        let (locked_tracks, open): (Vec<&str>, Vec<&str>) = refinement_tracks(target.role)
            .iter()
            .copied()
            .partition(|track| is_locked(&current_project, track));
        locked.extend(locked_tracks);
        if !open.is_empty() {
            dispatch.push((target, open));
        }
    }
    if dispatch.is_empty() {
        let summary = with_locked_note(no_change_summary(&plan.summary), &locked);
        return complete_refinement(tracker, summary, None);
    }

    let spec_start: f32 = 0.25;
    let spec_end: f32 = if req.skip_qa { 0.96 } else { 0.75 };
    let role_names = dispatch
        .iter()
        .map(|(target, _)| refinement_agent_id(target.role))
        .collect::<Vec<_>>()
        .join(", ");
    tracker.update(|s| {
        s.state = SwarmRunState::ExecutingSpecialists;
        s.active_roles = dispatch.iter().map(|(target, _)| target.role).collect();
        s.progress = spec_start;
        s.phase_label = format!("Refining {role_names}");
    });

    let ctx = RefineContext {
        backend,
        openrouter,
        models: &models,
        run_id,
        project: &current_project,
        brand,
        assets,
        prompt: &req.prompt,
        snapshot: &snapshot,
        max_turns: req.max_turns_per_worker.unwrap_or(6).clamp(1, 14),
        cancel,
        ops_counter,
    };
    let dispatch_total = dispatch.len();
    let dispatch_done = Arc::new(AtomicUsize::new(0));
    let ctx_ref = &ctx;
    let outcomes = join_all(dispatch.iter().map(|(target, tracks)| {
        let tracker = tracker.clone();
        let dispatch_done = dispatch_done.clone();
        async move {
            let outcome = run_refine_role(ctx_ref, target, tracks).await;
            let done_now = dispatch_done.fetch_add(1, Ordering::Relaxed) + 1;
            tracker.complete_role(
                Some(target.role),
                lerp_progress(spec_start, spec_end, done_now, dispatch_total),
                None,
            );
            outcome
        }
    }))
    .await;
    if cancel.is_cancelled() {
        return tracker.finish_cancelled();
    }

    let fatal = outcomes.iter().any(|o| o.fatal);
    let errors: Vec<String> = outcomes.iter().filter_map(|o| o.error.clone()).collect();
    if fatal || outcomes.iter().all(|o| o.error.is_some()) {
        return tracker.finish_failed(errors.join("; "));
    }

    if !req.skip_qa {
        const QA_START: f32 = 0.75;
        const QA_END: f32 = 0.98;
        tracker.update(|s| {
            s.state = SwarmRunState::RunningQa;
            s.progress = QA_START;
            s.phase_label =
                "Director running Stage-3 playback QA (frames, -14 LUFS & safe zones)".to_string();
            s.active_roles = vec![AgentRole::Director];
        });

        let vision = Some((
            models.director.as_str(),
            models.director_fallbacks.as_slice(),
        ));
        let qa_cb = |frac: f32, label: String| {
            tracker.advance(
                QA_START + (QA_END - QA_START) * frac.clamp(0.0, 1.0),
                Some(label),
            );
        };
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
                on_progress: Some(&qa_cb),
            },
        )
        .await;
        tracker.update(|s| {
            s.qa_report = Some(qa);
        });
    }
    if cancel.is_cancelled() {
        return tracker.finish_cancelled();
    }

    let mut summary = plan.summary.clone();
    if ops_counter.load(Ordering::Relaxed) == 0 {
        summary = format!("{summary} No clips were changed.");
    }
    let summary = with_locked_note(summary, &locked);
    let error = (!errors.is_empty()).then(|| errors.join("; "));
    complete_refinement(tracker, summary, error)
}

fn complete_refinement(
    tracker: &SwarmProgressTracker,
    summary: String,
    error: Option<String>,
) -> SwarmRunStatus {
    tracker.finish_completed(|s| {
        s.phase_label = summary.clone();
        s.refinement_summary = Some(summary);
        s.error = error;
    })
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
        "Edit/Refinement request: \"{}\". Your change: {}. Preserve existing clips unless asked to modify or remove them; edit existing clips by clip id when modifying them, or add newly requested elements cleanly on your tracks without duplicating existing clips. Your tracks: {}. Selected clip ids: {}. Playhead (sec): {}.",
        ctx.prompt,
        target.instruction,
        json!(own_tracks),
        ctx.snapshot["selected_clip_ids"],
        ctx.snapshot["playhead_sec"]
    )
}

#[cfg(test)]
mod tests;
