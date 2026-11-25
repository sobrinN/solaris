mod deterministic;
mod llm;
mod qa;
mod refine;
mod refine_plan;
mod refine_run;
mod request;
mod speech;
#[cfg(test)]
mod test_support;

pub use qa::run_qa_pass;
pub use request::{SwarmChatRole, SwarmChatTurn, SwarmRunMode, prepare_swarm_request};

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use solaris_core_ir::{
    AgentRole, BrandPlaybook, LogLevel, MediaKind, SavedCodeAsset, SwarmRunState, SwarmRunStatus,
};
use tokio_util::sync::CancellationToken;

use crate::advisor::{ModelAdvisor, ModelRoleKind};
use crate::backend::SolarisBackend;
use crate::error::AgentError;
use crate::openrouter::OpenRouterClient;
use crate::planner::CreativePlan;
use deterministic::{execute_deterministic_specialist, execute_deterministic_spine};
use llm::{
    build_openrouter_director_plan, emit_swarm_log, run_llm_worker_loop,
    run_programmatic_scene_worker,
};
pub(crate) use llm::{route_models, strip_json_fence};
use request::timeline_has_clips;
use speech::{is_music_or_sfx_asset, narrated_transcript_present, narration_expected};

enum TrackLockBackend<'a> {
    Owned(Arc<dyn SolarisBackend>),
    Borrowed(&'a dyn SolarisBackend),
}

impl TrackLockBackend<'_> {
    fn as_ref(&self) -> &dyn SolarisBackend {
        match self {
            Self::Owned(b) => b.as_ref(),
            Self::Borrowed(b) => *b,
        }
    }
}

/// RAII track lock guard that guarantees locks are released even on cancellation, panic, or task abort.
pub(crate) struct TrackLockGuard<'a> {
    backend: TrackLockBackend<'a>,
    project_id: String,
    track_ids: Vec<String>,
    agent_id: String,
    released: bool,
}

impl TrackLockGuard<'static> {
    pub(crate) async fn acquire(
        backend: Arc<dyn SolarisBackend>,
        project_id: &str,
        track_ids: &[&str],
        agent_id: &str,
        role: AgentRole,
    ) -> Result<Self, AgentError> {
        let acquired =
            acquire_tracks(backend.as_ref(), project_id, track_ids, agent_id, role).await?;
        Ok(Self {
            backend: TrackLockBackend::Owned(backend),
            project_id: project_id.to_string(),
            track_ids: acquired,
            agent_id: agent_id.to_string(),
            released: false,
        })
    }
}

impl<'a> TrackLockGuard<'a> {
    pub(crate) async fn acquire_ref(
        backend: &'a dyn SolarisBackend,
        project_id: &str,
        track_ids: &[&str],
        agent_id: &str,
        role: AgentRole,
    ) -> Result<Self, AgentError> {
        let acquired = acquire_tracks(backend, project_id, track_ids, agent_id, role).await?;
        Ok(Self {
            backend: TrackLockBackend::Borrowed(backend),
            project_id: project_id.to_string(),
            track_ids: acquired,
            agent_id: agent_id.to_string(),
            released: false,
        })
    }

    pub(crate) async fn release(mut self) {
        self.released = true;
        let backend = self.backend.as_ref();
        for tid in self.track_ids.iter().rev() {
            let _ = backend
                .release_track_lock(&self.project_id, tid, &self.agent_id)
                .await;
        }
    }
}

async fn acquire_tracks(
    backend: &dyn SolarisBackend,
    project_id: &str,
    track_ids: &[&str],
    agent_id: &str,
    role: AgentRole,
) -> Result<Vec<String>, AgentError> {
    let mut acquired: Vec<String> = Vec::with_capacity(track_ids.len());
    for &tid in track_ids {
        if let Err(err) = backend
            .acquire_track_lock(project_id, tid, agent_id, role)
            .await
        {
            for held in acquired.into_iter().rev() {
                let _ = backend
                    .release_track_lock(project_id, &held, agent_id)
                    .await;
            }
            return Err(err);
        }
        acquired.push(tid.to_string());
    }
    Ok(acquired)
}

impl Drop for TrackLockGuard<'_> {
    fn drop(&mut self) {
        if self.released || self.track_ids.is_empty() {
            return;
        }
        let project_id = self.project_id.clone();
        let track_ids = std::mem::take(&mut self.track_ids);
        let agent_id = self.agent_id.clone();
        match &self.backend {
            TrackLockBackend::Owned(arc) => {
                let backend = arc.clone();
                if let Ok(handle) = tokio::runtime::Handle::try_current() {
                    handle.spawn(async move {
                        for tid in track_ids.into_iter().rev() {
                            let _ = backend
                                .release_track_lock(&project_id, &tid, &agent_id)
                                .await;
                        }
                    });
                }
            }
            TrackLockBackend::Borrowed(backend) => {
                for tid in track_ids.into_iter().rev() {
                    backend.spawn_release_track_lock(&project_id, &tid, &agent_id);
                }
            }
        }
    }
}

pub(crate) fn is_voice_hint(hint: &str) -> bool {
    matches!(
        hint.to_ascii_lowercase().as_str(),
        "voice" | "voiceover" | "vo" | "dialogue" | "speech" | "narration" | "a_roll"
    )
}

pub(crate) fn is_broll_hint(hint: &str) -> bool {
    matches!(
        hint.to_ascii_lowercase().as_str(),
        "b_roll" | "broll" | "cutaway" | "insert"
    )
}

fn is_unrecoverable_openrouter_error(err: &AgentError) -> bool {
    match err {
        AgentError::NotConfigured | AgentError::Transport(_) => true,
        AgentError::HttpStatus { status, .. } => matches!(status, 401..=403),
        _ => false,
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SwarmRunRequest {
    pub prompt: String,
    #[serde(default)]
    pub model_override: Option<String>,
    #[serde(default = "default_true")]
    pub allow_local_fallback: bool,
    #[serde(default)]
    pub max_turns_per_worker: Option<usize>,
    #[serde(default)]
    pub skip_qa: bool,
    #[serde(default)]
    pub approved_plan: Option<CreativePlan>,
    #[serde(default)]
    pub mode: SwarmRunMode,
    #[serde(default)]
    pub selected_clip_ids: Vec<String>,
    #[serde(default)]
    pub playhead_sec: Option<f64>,
    #[serde(default)]
    pub conversation_history: Vec<SwarmChatTurn>,
}

fn default_true() -> bool {
    true
}

pub(crate) const MAX_PARALLEL_SCENE_AGENTS: usize = 7;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct SceneSpec {
    #[serde(default)]
    pub scene_index: usize,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub start_sec: f64,
    #[serde(default)]
    pub duration_sec: f64,
    #[serde(default)]
    pub visual_brief: String,
    #[serde(default)]
    pub motion_cue: String,
    #[serde(default)]
    pub narration: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct DirectorPlan {
    #[serde(default)]
    pub hook_summary: String,
    #[serde(default)]
    pub target_duration_sec: f64,
    #[serde(default)]
    pub art_direction: String,
    #[serde(default)]
    pub spine_notes: String,
    #[serde(default)]
    pub broll_notes: String,
    #[serde(default)]
    pub grade_notes: String,
    #[serde(default)]
    pub captions_notes: String,
    #[serde(default)]
    pub gfx_notes: String,
    #[serde(default)]
    pub mix_notes: String,
    #[serde(default)]
    pub scenes: Vec<SceneSpec>,
}

/// Normalize a Director's structured `scenes` (or fallback `[start-end]` beats parsed from `gfx_notes`)
/// into contiguous, gap-free scene specs spanning `0.0..target_duration_sec` for parallel micro-agent fan-out.
pub(crate) fn resolve_programmatic_scenes(
    plan: &DirectorPlan,
    target_duration_sec: f64,
) -> Vec<SceneSpec> {
    let total_dur = if target_duration_sec.is_finite() {
        target_duration_sec.clamp(4.0, 180.0)
    } else {
        24.0
    };

    let mut raw: Vec<SceneSpec> = plan
        .scenes
        .iter()
        .filter(|s| {
            s.start_sec.is_finite()
                && s.duration_sec.is_finite()
                && (!s.visual_brief.trim().is_empty() || !s.title.trim().is_empty())
        })
        .cloned()
        .collect();

    if raw.len() < 2 {
        let parsed = parse_bracketed_scenes_from_notes(&plan.gfx_notes, &plan.captions_notes);
        if parsed.len() >= 2 {
            raw = parsed;
        }
    }

    if raw.is_empty() {
        let count = if total_dur <= 15.0 { 4 } else { 6 };
        let beat_titles = [
            "Opening Hook",
            "Core Concept",
            "Architecture Breakdown",
            "How It Works Live",
            "Key Advantages",
            "Closing Takeaway",
        ];
        let seg = total_dur / (count as f64);
        for idx in 0..count {
            let start = (idx as f64) * seg;
            let end = if idx + 1 == count {
                total_dur
            } else {
                ((idx + 1) as f64) * seg
            };
            raw.push(SceneSpec {
                scene_index: idx + 1,
                title: beat_titles[idx % beat_titles.len()].to_string(),
                start_sec: start,
                duration_sec: (end - start).max(0.5),
                visual_brief: format!(
                    "Editorial motion graphic scene for '{}' — focus on {}",
                    plan.hook_summary,
                    beat_titles[idx % beat_titles.len()]
                ),
                motion_cue: "Crisp GSAP stagger reveal with subtle ambient depth motion"
                    .to_string(),
                narration: String::new(),
            });
        }
    }

    raw.sort_by(|a, b| a.start_sec.total_cmp(&b.start_sec));
    if raw.len() > MAX_PARALLEL_SCENE_AGENTS {
        // Fold trailing scenes (including the outro) into the 7th scene instead of dropping them.
        let overflow = raw.split_off(MAX_PARALLEL_SCENE_AGENTS);
        if let Some(last) = raw.last_mut() {
            for extra in overflow {
                if extra.duration_sec.is_finite() && extra.duration_sec > 0.0 {
                    last.duration_sec =
                        (last.duration_sec.max(0.0) + extra.duration_sec).min(total_dur);
                }
                let extra_brief = extra.visual_brief.trim();
                let extra_title = extra.title.trim();
                if !extra_brief.is_empty() {
                    if !last.visual_brief.trim().is_empty() {
                        last.visual_brief.push_str(" / ");
                    }
                    if !extra_title.is_empty() {
                        last.visual_brief.push_str(extra_title);
                        last.visual_brief.push_str(": ");
                    }
                    last.visual_brief.push_str(extra_brief);
                } else if !extra_title.is_empty() {
                    if !last.visual_brief.trim().is_empty() {
                        last.visual_brief.push_str(" / ");
                    }
                    last.visual_brief.push_str(extra_title);
                }
                let extra_nar = extra.narration.trim();
                if !extra_nar.is_empty() {
                    if !last.narration.trim().is_empty() {
                        last.narration.push(' ');
                    }
                    last.narration.push_str(extra_nar);
                }
            }
        }
    }

    // Stitch contiguously so scene 1 starts at 0.0 and final scene reaches `total_dur` with zero gaps.
    let n = raw.len();
    let min_dur = (((total_dur / (n as f64)).min(1.2) * 100.0).floor() / 100.0).max(0.5);
    let mut desired_durs: Vec<f64> = raw
        .iter()
        .map(|s| {
            if s.duration_sec.is_finite() && s.duration_sec >= 0.5 {
                s.duration_sec
            } else {
                total_dur / (n as f64)
            }
        })
        .collect();
    let desired_sum: f64 = desired_durs.iter().sum();
    if desired_sum > 0.0 && (desired_sum > total_dur * 1.02 || desired_sum < total_dur * 0.85) {
        let scale = total_dur / desired_sum;
        for d in &mut desired_durs {
            *d *= scale;
        }
    }

    let mut normalized = Vec::with_capacity(n);
    let mut cursor = 0.0_f64;
    for (idx, mut scene) in raw.into_iter().enumerate() {
        let remaining_after = (n - idx - 1) as f64;
        let dur = if idx + 1 == n {
            (total_dur - cursor).max(min_dur)
        } else {
            let max_allowed = (total_dur - cursor - min_dur * remaining_after).max(min_dur);
            desired_durs[idx].clamp(min_dur, max_allowed)
        };
        let start_rounded = (cursor * 100.0).round() / 100.0;
        let dur_rounded = if idx + 1 == n {
            ((total_dur - start_rounded).max(min_dur) * 100.0).round() / 100.0
        } else {
            ((dur * 100.0).round() / 100.0).max(min_dur)
        };
        scene.scene_index = idx + 1;
        scene.start_sec = start_rounded;
        scene.duration_sec = dur_rounded;
        if scene.title.trim().is_empty() {
            scene.title = format!("Scene {}", idx + 1);
        }
        if scene.visual_brief.trim().is_empty() {
            scene.visual_brief = scene.title.clone();
        }
        cursor = start_rounded + dur_rounded;
        normalized.push(scene);
    }
    normalized
}

fn parse_bracketed_scenes_from_notes(gfx_notes: &str, captions_notes: &str) -> Vec<SceneSpec> {
    let mut spans: Vec<(f64, f64, String)> = Vec::new();
    let bytes = gfx_notes.as_bytes();
    let mut i = 0usize;
    let mut markers: Vec<(usize, usize, f64, f64)> = Vec::new();
    while i < bytes.len() {
        if bytes[i] == b'['
            && let Some(close_rel) = gfx_notes[i + 1..].find(']')
        {
            let inner = &gfx_notes[i + 1..i + 1 + close_rel];
            if let Some((a, b)) = inner.split_once('-').or_else(|| inner.split_once('–')) {
                let s = a.trim().trim_end_matches('s').parse::<f64>().ok();
                let e = b.trim().trim_end_matches('s').parse::<f64>().ok();
                if let (Some(sv), Some(ev)) = (s, e)
                    && sv.is_finite()
                    && ev.is_finite()
                    && ev > sv
                {
                    markers.push((i, i + 1 + close_rel + 1, sv, ev));
                }
            }
            i += close_rel + 2;
            continue;
        }
        i += 1;
    }

    for (idx, &(_, content_start, sv, ev)) in markers.iter().enumerate() {
        let content_end = markers.get(idx + 1).map(|m| m.0).unwrap_or(gfx_notes.len());
        let body = gfx_notes[content_start..content_end].trim().to_string();
        if !body.is_empty() {
            spans.push((sv, ev, body));
        }
    }

    let caption_lines: Vec<&str> = captions_notes
        .split('[')
        .filter_map(|part| part.split_once(']').map(|(_, rest)| rest.trim()))
        .filter(|s| !s.is_empty())
        .collect();

    spans
        .into_iter()
        .enumerate()
        .map(|(idx, (sv, ev, body))| {
            let title = body
                .split(':')
                .next()
                .map(str::trim)
                .filter(|s| !s.is_empty() && s.len() <= 42)
                .unwrap_or("Scene")
                .to_string();
            SceneSpec {
                scene_index: idx + 1,
                title,
                start_sec: sv,
                duration_sec: ev - sv,
                visual_brief: body,
                motion_cue: "Smooth GSAP staggered choreography".to_string(),
                narration: caption_lines.get(idx).copied().unwrap_or("").to_string(),
            }
        })
        .collect()
}

pub(crate) fn infer_target_duration_sec(prompt: &str, project: &solaris_core_ir::Project) -> f64 {
    let media_max = project
        .media_pool
        .iter()
        .map(|m| m.intelligence.duration_sec)
        .fold(0.0_f64, f64::max);
    infer_target_duration_from_prompt(prompt, media_max)
}

pub(crate) fn infer_target_duration_from_prompt(prompt: &str, media_fallback_sec: f64) -> f64 {
    let lower = prompt.to_ascii_lowercase();
    let tokens: Vec<&str> = lower
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '.'))
        .filter(|s| !s.is_empty())
        .collect();

    for (idx, tok) in tokens.iter().enumerate() {
        let prev = idx.checked_sub(1).and_then(|p| tokens.get(p).copied());
        let next = tokens.get(idx + 1).copied();

        // Handle explicit second suffixes like "30sec", "45secs", "30seconds"
        if let Some(num_str) = tok
            .strip_suffix("seconds")
            .or_else(|| tok.strip_suffix("second"))
            .or_else(|| tok.strip_suffix("secs"))
            .or_else(|| tok.strip_suffix("sec"))
            && let Ok(val) = num_str.parse::<f64>()
            && val.is_finite()
            && (3.0..1000.0).contains(&val)
        {
            return val.clamp(4.0, 180.0);
        }

        // Handle short "30s" while rejecting 4-digit year decades ("2020s", "1980s") and
        // decade style descriptors ("the 80s", "in 90s", "80s retro style", "90s cinema").
        if let Some(num_str) = tok.strip_suffix('s')
            && let Ok(val) = num_str.parse::<f64>()
            && val.is_finite()
            && (3.0..1000.0).contains(&val)
        {
            let is_decade_context = matches!(prev, Some("the" | "in"))
                || matches!(
                    next,
                    Some(
                        "retro"
                            | "style"
                            | "aesthetic"
                            | "vibe"
                            | "vibes"
                            | "music"
                            | "synth"
                            | "cinema"
                            | "film"
                            | "look"
                            | "era"
                    )
                );
            if !is_decade_context {
                return val.clamp(4.0, 180.0);
            }
        }

        // Handle minute suffixes like "1m", "2m", "2min", "1minute"
        if let Some(num_str) = tok
            .strip_suffix("minutes")
            .or_else(|| tok.strip_suffix("minute"))
            .or_else(|| tok.strip_suffix("mins"))
            .or_else(|| tok.strip_suffix("min"))
            .or_else(|| tok.strip_suffix('m'))
            && let Ok(val) = num_str.parse::<f64>()
            && val.is_finite()
            && val > 0.0
            && val <= 60.0
        {
            return (val * 60.0).clamp(4.0, 180.0);
        }

        // Handle separated forms like "30 seconds", "30 sec", "1 minute"
        if let Ok(val) = tok.parse::<f64>()
            && val.is_finite()
            && let Some(next_tok) = next
        {
            if matches!(next_tok, "s" | "sec" | "secs" | "second" | "seconds")
                && (3.0..1000.0).contains(&val)
            {
                return val.clamp(4.0, 180.0);
            }
            if matches!(next_tok, "m" | "min" | "mins" | "minute" | "minutes")
                && val > 0.0
                && val <= 60.0
            {
                return (val * 60.0).clamp(4.0, 180.0);
            }
        }
    }

    if media_fallback_sec.is_finite() && media_fallback_sec > 1.0 {
        media_fallback_sec.clamp(6.0, 60.0)
    } else {
        24.0
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn run_swarm<F>(
    backend: Arc<dyn SolarisBackend>,
    openrouter: OpenRouterClient,
    advisor: Arc<ModelAdvisor>,
    project_id: String,
    run_id: String,
    req: SwarmRunRequest,
    cancel: CancellationToken,
    on_status: F,
) -> SwarmRunStatus
where
    F: Fn(SwarmRunStatus) + Send + Sync + 'static,
{
    let started_at = Utc::now();
    let ops_counter = Arc::new(AtomicU32::new(0));
    let mut status = SwarmRunStatus {
        run_id: run_id.clone(),
        project_id: project_id.clone(),
        state: SwarmRunState::Planning,
        phase_label: "Director inspecting media intelligence & brand playbook".to_string(),
        model_used: None,
        used_local_fallback: false,
        active_roles: vec![AgentRole::Director],
        ops_committed: 0,
        summary: None,
        error: None,
        qa_report: None,
        refinement_summary: None,
        started_at,
        updated_at: started_at,
    };
    on_status(status.clone());

    let _ = backend
        .emit_cursor_pulse(
            &project_id,
            "director-01",
            AgentRole::Director,
            "media.pool",
            "reading media manifest",
        )
        .await;

    let mut project = match backend.get_project(&project_id).await {
        Ok(p) => p,
        Err(err) => {
            return finish_failed(status, err.to_string(), &ops_counter, &on_status);
        }
    };
    let brand = backend
        .get_brand(&project.brand_playbook_id)
        .await
        .unwrap_or_else(|_| BrandPlaybook::solaris_default());
    let assets = backend.list_assets().await.unwrap_or_default();

    if cancel.is_cancelled() {
        return finish_cancelled(status, &ops_counter, &on_status);
    }

    let use_openrouter = openrouter.is_configured();
    if req.mode.resolve(timeline_has_clips(&project)) == SwarmRunMode::Refine {
        let run = refine_run::RefineRun {
            backend: &backend,
            openrouter: &openrouter,
            advisor: &advisor,
            project: &project,
            brand: &brand,
            assets: &assets,
            req: &req,
            run_id: &run_id,
            cancel: &cancel,
            ops_counter: &ops_counter,
        };
        return refine_run::run_refinement(run, status, &on_status).await;
    }
    if !use_openrouter && !req.allow_local_fallback {
        return finish_failed(
            status,
            "OPENROUTER_API_KEY is not configured on the server. Set OPENROUTER_API_KEY or pass allow_local_fallback=true.".to_string(),
            &ops_counter,
            &on_status,
        );
    }

    // Stage 1.0: Wait briefly for background FFmpeg intelligence analysis to complete on newly imported A/V assets
    let needs_analysis = |p: &solaris_core_ir::Project| {
        p.media_pool.iter().any(|m| {
            !m.path.is_empty()
                && matches!(m.kind, MediaKind::Video | MediaKind::Audio)
                && !m.intelligence.analyzed
        })
    };
    if needs_analysis(&project) {
        status.phase_label =
            "Waiting for media intelligence analysis to finish before planning".to_string();
        status.updated_at = Utc::now();
        on_status(status.clone());
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(15);
        while needs_analysis(&project) && tokio::time::Instant::now() < deadline {
            tokio::select! {
                _ = cancel.cancelled() => return finish_cancelled(status, &ops_counter, &on_status),
                _ = tokio::time::sleep(std::time::Duration::from_millis(250)) => {}
            }
            if let Ok(updated) = backend.get_project(&project_id).await {
                project = updated;
            } else {
                break;
            }
        }
    }

    // Stage 1a: Auto-transcribe untranscribed speech assets before Director/Razor/Caps run
    let has_any_video = project
        .media_pool
        .iter()
        .any(|m| m.kind == MediaKind::Video);
    let prompt_lower = req.prompt.to_ascii_lowercase();
    let prompt_mentions_vo = prompt_lower.contains("voiceover")
        || prompt_lower.contains("voice over")
        || prompt_lower.contains("voice-over")
        || prompt_lower.contains("narration")
        || prompt_lower.contains("speech")
        || prompt_lower.contains("audio")
        || prompt_lower.contains("caption")
        || prompt_lower.contains("subtitle");

    let untranscribed_speech: Vec<String> = project
        .media_pool
        .iter()
        .filter(|m| {
            if m.path.is_empty() || !m.intelligence.transcript.is_empty() {
                return false;
            }
            match m.kind {
                MediaKind::Video => {
                    m.intelligence.has_audio
                        && !is_broll_hint(&m.role_hint)
                        && !is_music_or_sfx_asset(m)
                }
                MediaKind::Audio => {
                    is_voice_hint(&m.role_hint)
                        || (!has_any_video && prompt_mentions_vo && !is_music_or_sfx_asset(m))
                }
                _ => false,
            }
        })
        .map(|m| m.id.clone())
        .take(4)
        .collect();

    if !untranscribed_speech.is_empty() {
        status.phase_label = format!(
            "Transcribing {} speech asset(s) before Director planning",
            untranscribed_speech.len()
        );
        status.updated_at = Utc::now();
        on_status(status.clone());

        let mut transcribed_any = false;
        for mid in untranscribed_speech {
            if cancel.is_cancelled() {
                return finish_cancelled(status, &ops_counter, &on_status);
            }
            let tr_res = tokio::select! {
                _ = cancel.cancelled() => return finish_cancelled(status, &ops_counter, &on_status),
                r = backend.transcribe_media(&project_id, &mid, None) => r,
            };
            match tr_res {
                Ok(words) if !words.is_empty() => {
                    transcribed_any = true;
                }
                Ok(_) => {}
                Err(err) => {
                    tracing::debug!(media_id = %mid, "stage-1 auto-transcribe skipped: {err}");
                }
            }
        }
        if transcribed_any && let Ok(updated) = backend.get_project(&project_id).await {
            project = updated;
        }
    }

    if cancel.is_cancelled() {
        return finish_cancelled(status, &ops_counter, &on_status);
    }

    let has_visual_media = project
        .media_pool
        .iter()
        .any(|m| matches!(m.kind, MediaKind::Video | MediaKind::Image));
    let has_audio_media = project.media_pool.iter().any(|m| {
        m.kind == MediaKind::Audio || (m.kind == MediaKind::Video && m.intelligence.has_audio)
    });
    let programmatic_visuals = !has_visual_media;
    let inferred_duration_sec = req
        .approved_plan
        .as_ref()
        .map(|p| p.target_duration_sec)
        .filter(|d| d.is_finite() && *d >= 3.0)
        .map(|d| d.clamp(4.0, 180.0))
        .unwrap_or_else(|| infer_target_duration_sec(&req.prompt, &project));

    emit_swarm_log(
        backend.as_ref(),
        LogLevel::Info,
        "swarm",
        &project_id,
        &run_id,
        Some("director-01"),
        format!(
            "Swarm started ({}, media={}, target={:.1}s, openrouter={})",
            if programmatic_visuals {
                "programmatic zero-asset mode"
            } else {
                "footage edit mode"
            },
            project.media_pool.len(),
            inferred_duration_sec,
            use_openrouter
        ),
        Some(format!("prompt: {}", req.prompt)),
    );

    let (
        director_model,
        director_fallbacks,
        video_director_fallbacks,
        audio_director_fallbacks,
        worker_model,
        worker_fallbacks,
        supports_native_video,
        supports_native_audio,
        supports_json_mode,
        worker_supports_reasoning,
        director_supports_reasoning,
    ) = if use_openrouter {
        let dir_rec = advisor
            .recommend_with_cancel(ModelRoleKind::DirectorMultimodal, Some(&cancel))
            .await;
        if cancel.is_cancelled() {
            return finish_cancelled(status, &ops_counter, &on_status);
        }
        let wrk_rec = advisor
            .recommend_with_cancel(ModelRoleKind::SpecialistToolWorker, Some(&cancel))
            .await;
        if cancel.is_cancelled() {
            return finish_cancelled(status, &ops_counter, &on_status);
        }
        let primary_dir = req
            .model_override
            .clone()
            .unwrap_or_else(|| dir_rec.primary.clone());
        let primary_wrk = req
            .model_override
            .clone()
            .unwrap_or_else(|| wrk_rec.primary.clone());
        let dir_inputs = advisor.model_input_modalities(&primary_dir).await;
        let native_audio = dir_inputs.iter().any(|m| m == "audio");
        let native_video = dir_inputs.iter().any(|m| m == "video") && native_audio;
        let json_mode = advisor
            .model_supports_parameter(&primary_dir, "response_format")
            .await
            || advisor
                .model_supports_parameter(&primary_dir, "structured_outputs")
                .await;
        let wrk_reasoning = advisor
            .model_supports_parameter(&primary_wrk, "reasoning")
            .await;
        let dir_reasoning = advisor
            .model_supports_parameter(&primary_dir, "reasoning")
            .await;
        let video_fbs = if native_video {
            advisor
                .filter_video_capable_models(&dir_rec.fallbacks)
                .await
        } else {
            Vec::new()
        };
        let audio_fbs = if native_audio {
            advisor
                .filter_audio_capable_models(&dir_rec.fallbacks)
                .await
        } else {
            Vec::new()
        };
        (
            primary_dir,
            dir_rec.fallbacks,
            video_fbs,
            audio_fbs,
            primary_wrk,
            wrk_rec.fallbacks,
            native_video,
            native_audio,
            json_mode,
            wrk_reasoning,
            dir_reasoning,
        )
    } else {
        (
            "local-deterministic-director".to_string(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            "local-deterministic-worker".to_string(),
            Vec::new(),
            false,
            false,
            false,
            false,
            false,
        )
    };

    status.model_used = Some(director_model.clone());
    status.used_local_fallback = !use_openrouter;
    let mut openrouter_unrecoverable = !use_openrouter;

    // Stage 1b: Director Plan
    status.phase_label = if programmatic_visuals {
        "Director designing programmatic scene storyboard & script".to_string()
    } else {
        "Director inspecting media intelligence & brand playbook".to_string()
    };
    status.updated_at = Utc::now();
    on_status(status.clone());

    let plan = if use_openrouter {
        match build_openrouter_director_plan(
            &backend,
            &openrouter,
            &director_model,
            &director_fallbacks,
            &video_director_fallbacks,
            &audio_director_fallbacks,
            supports_native_video,
            supports_native_audio,
            supports_json_mode,
            director_supports_reasoning,
            programmatic_visuals,
            inferred_duration_sec,
            &run_id,
            &project,
            &brand,
            &assets,
            &req.prompt,
            req.approved_plan.as_ref(),
            &cancel,
        )
        .await
        {
            Ok(p) => p,
            Err(AgentError::Cancelled) => {
                return finish_cancelled(status, &ops_counter, &on_status);
            }
            Err(err) if req.allow_local_fallback => {
                if is_unrecoverable_openrouter_error(&err) {
                    openrouter_unrecoverable = true;
                }
                tracing::warn!(
                    "OpenRouter director failed ({err}); falling back to deterministic plan"
                );
                emit_swarm_log(
                    backend.as_ref(),
                    LogLevel::Warn,
                    "swarm",
                    &project_id,
                    &run_id,
                    Some("director-01"),
                    format!("Director failed; falling back to local plan: {err}"),
                    None,
                );
                status.used_local_fallback = true;
                build_deterministic_plan(&project, &brand, &req.prompt)
            }
            Err(err) => {
                emit_swarm_log(
                    backend.as_ref(),
                    LogLevel::Error,
                    "swarm",
                    &project_id,
                    &run_id,
                    Some("director-01"),
                    format!("Swarm aborted during Director planning: {err}"),
                    None,
                );
                return finish_failed(status, err.to_string(), &ops_counter, &on_status);
            }
        }
    } else {
        build_deterministic_plan(&project, &brand, &req.prompt)
    };

    if cancel.is_cancelled() {
        return finish_cancelled(status, &ops_counter, &on_status);
    }

    let max_turns = req.max_turns_per_worker.unwrap_or(6).clamp(1, 14);
    let mut specialist_errors: Vec<String> = Vec::new();
    let mut wave1_grade_done = false;

    // Wave 1 (Footage/Audio Mode): Run razor-01 (V1/A1) IN PARALLEL with grade-01 (FX).
    // mix-01 (A2) runs in Wave 2 after razor-01 commits the V1/A1 spine and releases its locks
    // so mix-01 sees the real spine duration and can normalize speech LUFS without lock collisions.
    if has_visual_media || has_audio_media {
        let mut wave1_roles = vec![AgentRole::Razor];
        if has_visual_media {
            wave1_roles.push(AgentRole::Grade);
            wave1_grade_done = true;
        }
        status.state = SwarmRunState::ExecutingSpine;
        status.phase_label = if has_visual_media {
            "Wave 1: Razor (V1/A1) & Grade (FX) editing in parallel".to_string()
        } else {
            "Wave 1: Razor (A1) assembling voiceover spine".to_string()
        };
        status.active_roles = wave1_roles;
        status.updated_at = Utc::now();
        on_status(status.clone());

        let use_llm_wave1 = use_openrouter && !openrouter_unrecoverable;
        let grade_cancel = cancel.child_token();
        let spine_grade_cancel = grade_cancel.clone();
        let (spine_res, r_grade_w1) = tokio::join!(
            async {
                let res = run_spine_stage(
                    &backend,
                    &openrouter,
                    use_llm_wave1,
                    req.allow_local_fallback,
                    &worker_model,
                    &worker_fallbacks,
                    worker_supports_reasoning,
                    &run_id,
                    programmatic_visuals,
                    &project,
                    &brand,
                    &assets,
                    &plan,
                    max_turns,
                    &cancel,
                    &ops_counter,
                )
                .await;
                if res.is_err() {
                    spine_grade_cancel.cancel();
                }
                res
            },
            async {
                if has_visual_media {
                    run_specialist_with_lock(
                        backend.clone(),
                        openrouter.clone(),
                        use_llm_wave1,
                        req.allow_local_fallback,
                        worker_model.clone(),
                        worker_fallbacks.clone(),
                        worker_supports_reasoning,
                        run_id.clone(),
                        programmatic_visuals,
                        req.prompt.clone(),
                        project.clone(),
                        brand.clone(),
                        assets.clone(),
                        plan.clone(),
                        "fx",
                        "grade-01",
                        AgentRole::Grade,
                        max_turns,
                        grade_cancel,
                        ops_counter.clone(),
                    )
                    .await
                } else {
                    (false, false, None)
                }
            }
        );

        let (fb, fatal, err) = r_grade_w1;
        if fb {
            status.used_local_fallback = true;
        }
        if fatal {
            openrouter_unrecoverable = true;
        }
        if let Some(e) = err {
            specialist_errors.push(e);
        }

        match spine_res {
            Ok((used_fallback, fatal)) => {
                if used_fallback {
                    status.used_local_fallback = true;
                }
                if fatal {
                    openrouter_unrecoverable = true;
                }
            }
            Err(AgentError::Cancelled) => {
                return finish_cancelled(status, &ops_counter, &on_status);
            }
            Err(err) => {
                emit_swarm_log(
                    backend.as_ref(),
                    LogLevel::Error,
                    "swarm",
                    &project_id,
                    &run_id,
                    Some("razor-01"),
                    format!("Spine stage failed: {err}"),
                    None,
                );
                return finish_failed(status, err.to_string(), &ops_counter, &on_status);
            }
        }
    } else {
        emit_swarm_log(
            backend.as_ref(),
            LogLevel::Info,
            "swarm",
            &project_id,
            &run_id,
            Some("director-01"),
            "Programmatic zero-asset mode: skipping V1/A1 media spine and fanning out parallel G1 scene micro-agents",
            None,
        );
    }

    if cancel.is_cancelled() {
        return finish_cancelled(status, &ops_counter, &on_status);
    }

    let refreshed = backend.get_project(&project_id).await.unwrap_or(project);
    let narrated = narrated_transcript_present(&refreshed);
    if !narrated {
        emit_swarm_log(
            backend.as_ref(),
            LogLevel::Info,
            "swarm",
            &project_id,
            &run_id,
            Some("caps-01"),
            "No narration detected: captions skipped".to_string(),
            None,
        );
    }

    // Stage 3 / Wave 2: Parallel Specialist & Scene Micro-Agent Fan-Out
    let programmatic_scenes = if programmatic_visuals {
        resolve_programmatic_scenes(&plan, plan.target_duration_sec)
    } else {
        Vec::new()
    };

    let caps_plan = if programmatic_visuals && !programmatic_scenes.is_empty() {
        let mut p = plan.clone();
        let beats = programmatic_scenes
            .iter()
            .map(|s| {
                let text = if !s.narration.trim().is_empty() {
                    s.narration.trim()
                } else {
                    s.title.trim()
                };
                format!(
                    "[{:.2}-{:.2}s] {}: {}",
                    s.start_sec,
                    s.start_sec + s.duration_sec,
                    s.title.trim(),
                    text
                )
            })
            .collect::<Vec<_>>()
            .join(" | ");
        p.captions_notes = if p.captions_notes.trim().is_empty() {
            format!("Align T1 captions 1:1 with normalized G1 scene windows: {beats}")
        } else {
            format!(
                "{} | Synchronized G1 scene windows (match these exact timings): {beats}",
                p.captions_notes.trim()
            )
        };
        p
    } else {
        plan.clone()
    };

    let mut active_roles = Vec::new();
    if has_visual_media {
        active_roles.push(AgentRole::BRoll);
        if !wave1_grade_done {
            active_roles.push(AgentRole::Grade);
        }
    }
    if narrated {
        active_roles.push(AgentRole::Caps);
    }
    active_roles.push(AgentRole::Gfx);
    if has_audio_media {
        active_roles.push(AgentRole::Mix);
    }

    status.state = SwarmRunState::ExecutingSpecialists;
    status.phase_label = wave2_label(
        programmatic_visuals,
        has_audio_media,
        programmatic_scenes.len(),
        narrated,
    );
    status.active_roles = active_roles;
    status.ops_committed = ops_counter.load(Ordering::Relaxed);
    status.updated_at = Utc::now();
    on_status(status.clone());

    let use_llm_workers = use_openrouter && !openrouter_unrecoverable;
    let gfx_turns = if programmatic_visuals {
        max_turns.max(12)
    } else {
        max_turns
    };
    let caps_turns = if programmatic_visuals {
        max_turns.max(6)
    } else {
        max_turns
    };

    let mut active_specialist_count = 0usize;
    if has_visual_media {
        active_specialist_count += 2; // BRoll + Grade
    }
    active_specialist_count += 1 + usize::from(narrated); // Caps (when narrated) + Gfx
    if has_audio_media {
        active_specialist_count += 1; // Mix
    }

    let gfx_worker_fallbacks: Vec<String> = if programmatic_visuals {
        let mut list = Vec::new();
        if director_model != worker_model {
            list.push(director_model.clone());
        }
        for fb in &worker_fallbacks {
            if fb != &worker_model && !list.contains(fb) {
                list.push(fb.clone());
            }
        }
        for fb in &director_fallbacks {
            if fb != &worker_model && !list.contains(fb) {
                list.push(fb.clone());
            }
        }
        list
    } else {
        worker_fallbacks.clone()
    };

    let (r_broll, r_grade, r_caps, r_gfx, r_mix) = tokio::join!(
        async {
            if has_visual_media {
                run_specialist_with_lock(
                    backend.clone(),
                    openrouter.clone(),
                    use_llm_workers,
                    req.allow_local_fallback,
                    worker_model.clone(),
                    worker_fallbacks.clone(),
                    worker_supports_reasoning,
                    run_id.clone(),
                    programmatic_visuals,
                    req.prompt.clone(),
                    refreshed.clone(),
                    brand.clone(),
                    assets.clone(),
                    plan.clone(),
                    "v2",
                    "broll-01",
                    AgentRole::BRoll,
                    max_turns,
                    cancel.clone(),
                    ops_counter.clone(),
                )
                .await
            } else {
                (false, false, None)
            }
        },
        async {
            if has_visual_media && !wave1_grade_done {
                run_specialist_with_lock(
                    backend.clone(),
                    openrouter.clone(),
                    use_llm_workers,
                    req.allow_local_fallback,
                    worker_model.clone(),
                    worker_fallbacks.clone(),
                    worker_supports_reasoning,
                    run_id.clone(),
                    programmatic_visuals,
                    req.prompt.clone(),
                    refreshed.clone(),
                    brand.clone(),
                    assets.clone(),
                    plan.clone(),
                    "fx",
                    "grade-01",
                    AgentRole::Grade,
                    max_turns,
                    cancel.clone(),
                    ops_counter.clone(),
                )
                .await
            } else {
                (false, false, None)
            }
        },
        async {
            if narrated {
                run_specialist_with_lock(
                    backend.clone(),
                    openrouter.clone(),
                    use_llm_workers,
                    req.allow_local_fallback,
                    worker_model.clone(),
                    worker_fallbacks.clone(),
                    worker_supports_reasoning,
                    run_id.clone(),
                    programmatic_visuals,
                    req.prompt.clone(),
                    refreshed.clone(),
                    brand.clone(),
                    assets.clone(),
                    caps_plan,
                    "t1",
                    "caps-01",
                    AgentRole::Caps,
                    caps_turns,
                    cancel.clone(),
                    ops_counter.clone(),
                )
                .await
            } else {
                (false, false, None)
            }
        },
        async {
            if programmatic_visuals {
                run_programmatic_gfx_scene_fanout(
                    backend.clone(),
                    openrouter.clone(),
                    use_llm_workers,
                    req.allow_local_fallback,
                    worker_model.clone(),
                    gfx_worker_fallbacks.clone(),
                    worker_supports_reasoning,
                    director_model.clone(),
                    director_fallbacks.clone(),
                    director_supports_reasoning,
                    run_id.clone(),
                    req.prompt.clone(),
                    refreshed.clone(),
                    brand.clone(),
                    assets.clone(),
                    plan.clone(),
                    programmatic_scenes.clone(),
                    cancel.clone(),
                    ops_counter.clone(),
                )
                .await
            } else {
                run_specialist_with_lock(
                    backend.clone(),
                    openrouter.clone(),
                    use_llm_workers,
                    req.allow_local_fallback,
                    worker_model.clone(),
                    gfx_worker_fallbacks.clone(),
                    worker_supports_reasoning,
                    run_id.clone(),
                    programmatic_visuals,
                    req.prompt.clone(),
                    refreshed.clone(),
                    brand.clone(),
                    assets.clone(),
                    plan.clone(),
                    "g1",
                    "gfx-01",
                    AgentRole::Gfx,
                    gfx_turns,
                    cancel.clone(),
                    ops_counter.clone(),
                )
                .await
            }
        },
        async {
            if has_audio_media {
                run_specialist_with_lock(
                    backend.clone(),
                    openrouter.clone(),
                    use_llm_workers,
                    req.allow_local_fallback,
                    worker_model.clone(),
                    worker_fallbacks.clone(),
                    worker_supports_reasoning,
                    run_id.clone(),
                    programmatic_visuals,
                    req.prompt.clone(),
                    refreshed.clone(),
                    brand.clone(),
                    assets.clone(),
                    plan.clone(),
                    "a2",
                    "mix-01",
                    AgentRole::Mix,
                    max_turns,
                    cancel.clone(),
                    ops_counter.clone(),
                )
                .await
            } else {
                (false, false, None)
            }
        },
    );

    if cancel.is_cancelled() {
        return finish_cancelled(status, &ops_counter, &on_status);
    }

    let specialist_results = [r_broll, r_grade, r_caps, r_gfx, r_mix];
    if specialist_results.iter().any(|(fb, _, _)| *fb) {
        status.used_local_fallback = true;
    }
    if specialist_results.iter().any(|(_, fatal, _)| *fatal) {
        openrouter_unrecoverable = true;
    }
    specialist_errors.extend(specialist_results.into_iter().filter_map(|(_, _, err)| err));

    // Programmatic zero-asset safety net: if G1 still has 0 clips after the parallel fan-out,
    // run a targeted Director-model recovery pass on G1 before entering QA.
    if programmatic_visuals
        && use_llm_workers
        && !openrouter_unrecoverable
        && !cancel.is_cancelled()
        && let Ok(post_pass) = backend.get_project(&project_id).await
    {
        let g1_empty = post_pass
            .tracks
            .iter()
            .find(|t| t.id == "g1")
            .is_none_or(|t| t.clips.is_empty());
        if g1_empty {
            emit_swarm_log(
                backend.as_ref(),
                LogLevel::Warn,
                "swarm",
                &project_id,
                &run_id,
                Some("director-01"),
                format!(
                    "G1 is empty after scene fan-out; retrying programmatic GFX on {director_model}"
                ),
                None,
            );
            status.phase_label =
                "Recovering programmatic G1 scenes with Director model".to_string();
            status.active_roles = vec![AgentRole::Gfx];
            status.updated_at = Utc::now();
            on_status(status.clone());

            let (rec_fb, rec_fatal, rec_err) = run_specialist_with_lock(
                backend.clone(),
                openrouter.clone(),
                true,
                req.allow_local_fallback,
                director_model.clone(),
                director_fallbacks.clone(),
                director_supports_reasoning,
                run_id.clone(),
                true,
                req.prompt.clone(),
                post_pass,
                brand.clone(),
                assets.clone(),
                plan.clone(),
                "g1",
                "gfx-01",
                AgentRole::Gfx,
                gfx_turns,
                cancel.clone(),
                ops_counter.clone(),
            )
            .await;
            if rec_fb {
                status.used_local_fallback = true;
            }
            if rec_fatal {
                openrouter_unrecoverable = true;
            }
            let g1_now_populated = backend
                .get_project(&project_id)
                .await
                .ok()
                .and_then(|p| p.tracks.into_iter().find(|t| t.id == "g1"))
                .is_some_and(|t| !t.clips.is_empty());
            if g1_now_populated {
                specialist_errors.retain(|e| !e.starts_with("gfx-"));
            } else if let Some(err) = rec_err {
                specialist_errors.retain(|e| !e.starts_with("gfx-"));
                specialist_errors.push(err);
            }
        }
    }

    if !req.allow_local_fallback && specialist_errors.len() >= active_specialist_count {
        let msg = format!(
            "All specialists failed without fallback: {}",
            specialist_errors.join("; ")
        );
        emit_swarm_log(
            backend.as_ref(),
            LogLevel::Error,
            "swarm",
            &project_id,
            &run_id,
            Some("director-01"),
            msg.clone(),
            None,
        );
        return finish_failed(status, msg, &ops_counter, &on_status);
    }

    if !req.skip_qa && !cancel.is_cancelled() {
        status.state = SwarmRunState::RunningQa;
        status.phase_label =
            "Director running Stage-3 playback QA (frames, -14 LUFS & safe zones)".to_string();
        status.active_roles = vec![AgentRole::Director];
        status.ops_committed = ops_counter.load(Ordering::Relaxed);
        status.updated_at = Utc::now();
        on_status(status.clone());

        let qa_client = if openrouter.is_configured() && !openrouter_unrecoverable {
            Some(&openrouter)
        } else {
            None
        };
        let vision = Some((director_model.as_str(), director_fallbacks.as_slice()));
        let qa = qa::run_qa_pass_with_options(
            backend.as_ref(),
            qa_client,
            vision,
            &project_id,
            true,
            &cancel,
            Some(&ops_counter),
            qa::QaPassOptions {
                specialist_errors: &specialist_errors,
                director_supports_reasoning,
            },
        )
        .await;
        status.qa_report = Some(qa);
    }

    if cancel.is_cancelled() {
        return finish_cancelled(status, &ops_counter, &on_status);
    }

    let has_issues =
        !specialist_errors.is_empty() || status.qa_report.as_ref().is_some_and(|q| !q.passed);
    status.state = SwarmRunState::Completed;
    status.active_roles = Vec::new();
    status.ops_committed = ops_counter.load(Ordering::Relaxed);
    status.phase_label = if let Some(ref qa) = status.qa_report {
        if has_issues {
            format!(
                "Swarm finished with issues ({} ops committed · QA {}/100)",
                status.ops_committed, qa.score
            )
        } else {
            format!(
                "Swarm complete ({} ops committed · QA {}/100)",
                status.ops_committed, qa.score
            )
        }
    } else if !specialist_errors.is_empty() {
        format!(
            "Swarm finished with issues ({} ops committed)",
            status.ops_committed
        )
    } else {
        format!("Swarm complete ({} ops committed)", status.ops_committed)
    };
    status.summary = Some(plan.hook_summary);
    if !specialist_errors.is_empty() {
        status.error = Some(specialist_errors.join("; "));
    }
    status.updated_at = Utc::now();
    emit_swarm_log(
        backend.as_ref(),
        if specialist_errors.is_empty() {
            LogLevel::Info
        } else {
            LogLevel::Warn
        },
        "swarm",
        &project_id,
        &run_id,
        Some("director-01"),
        status.phase_label.clone(),
        status.error.clone(),
    );
    on_status(status.clone());
    status
}

fn wave2_label(
    programmatic_visuals: bool,
    has_audio_media: bool,
    scene_count: usize,
    narrated: bool,
) -> String {
    let mut label = match (programmatic_visuals, has_audio_media, narrated) {
        (true, _, true) => format!(
            "Fanning out {scene_count} parallel scene micro-agents (G1) & kinetic captions (T1)"
        ),
        (true, _, false) => format!("Fanning out {scene_count} parallel scene micro-agents (G1)"),
        (false, true, true) => "Wave 2: Parallel specialists editing V2, T1, G1 & A2".to_string(),
        (false, true, false) => "Wave 2: Parallel specialists editing V2, G1 & A2".to_string(),
        (false, false, true) => "Wave 2: Parallel specialists editing V2, T1 & G1".to_string(),
        (false, false, false) => "Wave 2: Parallel specialists editing V2 & G1".to_string(),
    };
    if !narrated {
        label.push_str(" · No narration detected: captions skipped");
    }
    label
}

fn finish_failed(
    mut status: SwarmRunStatus,
    err: String,
    ops_counter: &AtomicU32,
    on_status: &impl Fn(SwarmRunStatus),
) -> SwarmRunStatus {
    status.state = SwarmRunState::Failed;
    status.active_roles = Vec::new();
    status.ops_committed = ops_counter.load(Ordering::Relaxed);
    status.phase_label = "Swarm failed".to_string();
    status.error = Some(err);
    status.updated_at = Utc::now();
    on_status(status.clone());
    status
}

fn finish_cancelled(
    mut status: SwarmRunStatus,
    ops_counter: &AtomicU32,
    on_status: &impl Fn(SwarmRunStatus),
) -> SwarmRunStatus {
    status.state = SwarmRunState::Cancelled;
    status.active_roles = Vec::new();
    status.ops_committed = ops_counter.load(Ordering::Relaxed);
    status.phase_label = "Swarm cancelled".to_string();
    status.updated_at = Utc::now();
    on_status(status.clone());
    status
}

pub(crate) fn build_deterministic_plan(
    project: &solaris_core_ir::Project,
    brand: &BrandPlaybook,
    prompt: &str,
) -> DirectorPlan {
    let target_dur = infer_target_duration_sec(prompt, project);
    let has_visual = project
        .media_pool
        .iter()
        .any(|m| matches!(m.kind, MediaKind::Video | MediaKind::Image));
    DirectorPlan {
        hook_summary: format!(
            "{} ({})",
            prompt.chars().take(72).collect::<String>(),
            brand.name
        ),
        target_duration_sec: target_dur,
        art_direction: format!(
            "Solar Horizon editorial motion system using {} backdrop, high-contrast {} typography, and crisp {} accent diagrams.",
            brand.tokens.bg, brand.tokens.ink, brand.tokens.accent
        ),
        spine_notes: if has_visual {
            "Assemble A-Roll/Voice on V1/A1, cut >0.25s silences with 0.12s pad, 1.08x hook zoom."
                .to_string()
        } else {
            "Zero-asset programmatic mode: full-canvas HTML/GSAP scenes on G1 and no captions (T1 stays empty without narration audio).".to_string()
        },
        broll_notes: "Place B-Roll/Image inserts on V2 with ken_burns on stills.".to_string(),
        grade_notes: "Recover Log footage to Rec.709 and blend brand LUT if present.".to_string(),
        captions_notes: if narration_expected(project) {
            "Captions only from transcribed narration (captions.generate_from_transcript on T1, 2-4 words per burst).".to_string()
        } else {
            "No narration detected: place no captions on T1.".to_string()
        },
        gfx_notes: if has_visual {
            "Place Brand-tokenized title/stat cards on G1.".to_string()
        } else {
            format!(
                "Program 6 full-canvas responsive HTML/CSS/SVG/GSAP explainer scenes on G1 covering 0.0s..{target_dur:.1}s with zero gaps."
            )
        },
        mix_notes: "Normalize speech to -14 LUFS and enable -18 dB ducking on A2.".to_string(),
        scenes: Vec::new(),
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_spine_stage(
    backend: &Arc<dyn SolarisBackend>,
    openrouter: &OpenRouterClient,
    use_llm: bool,
    allow_local_fallback: bool,
    model: &str,
    fallbacks: &[String],
    low_reasoning: bool,
    run_id: &str,
    programmatic_visuals: bool,
    project: &solaris_core_ir::Project,
    brand: &BrandPlaybook,
    assets: &[SavedCodeAsset],
    plan: &DirectorPlan,
    max_turns: usize,
    cancel: &CancellationToken,
    ops_counter: &Arc<AtomicU32>,
) -> Result<(bool, bool), AgentError> {
    let lock_guard = TrackLockGuard::acquire(
        backend.clone(),
        &project.id,
        &["v1", "a1"],
        "razor-01",
        AgentRole::Razor,
    )
    .await?;

    let _ = backend
        .emit_cursor_pulse(
            &project.id,
            "razor-01",
            AgentRole::Razor,
            "timeline.track.v1",
            "cutting A-roll spine",
        )
        .await;

    let v1_empty = project
        .tracks
        .iter()
        .find(|t| t.id == "v1")
        .is_none_or(|t| t.clips.is_empty());
    let has_visual_or_voice = project.media_pool.iter().any(|m| {
        matches!(
            m.kind,
            MediaKind::Video | MediaKind::Image | MediaKind::Audio
        )
    });

    let mut used_fallback = !use_llm;
    let mut openrouter_fatal = false;
    let res = if use_llm {
        match run_llm_worker_loop(
            backend,
            openrouter,
            model,
            fallbacks,
            low_reasoning,
            run_id,
            programmatic_visuals,
            plan.target_duration_sec,
            project,
            brand,
            assets,
            "razor-01",
            AgentRole::Razor,
            &["v1", "a1"],
            &format!(
                "You own tracks v1 and a1. Plan: {}. Place A-roll video/voice clips, trim dead silences (>0.25s with 0.12s breath pad), and add a subtle 1.08x punch-in on the hook.",
                plan.spine_notes
            ),
            max_turns,
            SwarmRunMode::Generate,
            cancel,
            ops_counter,
        )
        .await
        {
            Ok(0) if v1_empty && has_visual_or_voice && allow_local_fallback => {
                tracing::warn!(
                    "Razor LLM loop committed 0 ops on empty spine; falling back to deterministic spine"
                );
                used_fallback = true;
                execute_deterministic_spine(backend, project, cancel, ops_counter).await
            }
            Ok(0) if v1_empty && has_visual_or_voice => Err(AgentError::ToolError(
                "Razor LLM worker committed 0 ops on empty timeline spine".to_string(),
            )),
            Ok(_) => Ok(()),
            Err(AgentError::Cancelled) => Err(AgentError::Cancelled),
            Err(err) if allow_local_fallback => {
                if is_unrecoverable_openrouter_error(&err) {
                    openrouter_fatal = true;
                }
                tracing::warn!("Razor LLM loop failed ({err}); falling back to deterministic spine");
                used_fallback = true;
                execute_deterministic_spine(backend, project, cancel, ops_counter).await
            }
            Err(err) => Err(err),
        }
    } else {
        execute_deterministic_spine(backend, project, cancel, ops_counter).await
    };

    lock_guard.release().await;
    res.map(|()| (used_fallback, openrouter_fatal))
}

#[allow(clippy::too_many_arguments)]
async fn run_programmatic_gfx_scene_fanout(
    backend: Arc<dyn SolarisBackend>,
    openrouter: OpenRouterClient,
    use_llm: bool,
    allow_local_fallback: bool,
    worker_model: String,
    worker_fallbacks: Vec<String>,
    worker_supports_reasoning: bool,
    director_model: String,
    director_fallbacks: Vec<String>,
    director_supports_reasoning: bool,
    run_id: String,
    prompt: String,
    project: solaris_core_ir::Project,
    brand: BrandPlaybook,
    assets: Vec<SavedCodeAsset>,
    plan: DirectorPlan,
    scenes: Vec<SceneSpec>,
    cancel: CancellationToken,
    ops_counter: Arc<AtomicU32>,
) -> (bool, bool, Option<String>) {
    if cancel.is_cancelled() {
        return (false, false, None);
    }
    let fresh_project = backend
        .get_project(&project.id)
        .await
        .unwrap_or_else(|_| project.clone());
    if fresh_project
        .tracks
        .iter()
        .find(|t| t.id == "g1")
        .is_some_and(|t| t.locked)
    {
        return (
            false,
            false,
            Some("gfx-fanout: skipped (track g1 is locked)".to_string()),
        );
    }
    if !use_llm {
        let lock_guard = match TrackLockGuard::acquire(
            backend.clone(),
            &project.id,
            &["g1"],
            "gfx-01",
            AgentRole::Gfx,
        )
        .await
        {
            Ok(g) => g,
            Err(err) => {
                return (false, false, Some(format!("gfx-fanout: skipped ({err})")));
            }
        };
        let _ = execute_deterministic_specialist(
            &backend,
            &fresh_project,
            &brand,
            &assets,
            &plan,
            &prompt,
            AgentRole::Gfx,
            &cancel,
            &ops_counter,
        )
        .await;
        lock_guard.release().await;
        return (true, false, None);
    }

    const GFX_SWARM_LOCK_OWNER: &str = "gfx-swarm";
    let lock_guard = match TrackLockGuard::acquire(
        backend.clone(),
        &project.id,
        &["g1"],
        GFX_SWARM_LOCK_OWNER,
        AgentRole::Gfx,
    )
    .await
    {
        Ok(g) => g,
        Err(err) => {
            return (false, false, Some(format!("gfx-fanout: skipped ({err})")));
        }
    };

    let run_tag: String = uuid::Uuid::new_v4().simple().to_string()[..6].to_string();
    let total_scenes = scenes.len();
    let art_direction = if plan.art_direction.trim().is_empty() {
        format!(
            "Cohesive Solar Horizon studio motion system on {} dark canvas with {} ink, {} primary accents, {} secondary glow, subtle architectural grid backdrop, and smooth staggered GSAP choreography.",
            brand.tokens.bg, brand.tokens.ink, brand.tokens.accent, brand.tokens.accent_secondary
        )
    } else {
        plan.art_direction.clone()
    };

    emit_swarm_log(
        backend.as_ref(),
        LogLevel::Info,
        "swarm",
        &project.id,
        &run_id,
        Some("director-01"),
        format!(
            "Orchestrator spawning {total_scenes} parallel scene micro-agents on G1 (gfx-01..gfx-{total_scenes:02})"
        ),
        None,
    );

    let mut join_set = tokio::task::JoinSet::new();
    for scene in scenes {
        let b = backend.clone();
        let or = openrouter.clone();
        let wm = worker_model.clone();
        let wfb = worker_fallbacks.clone();
        let dm = director_model.clone();
        let dfb = director_fallbacks.clone();
        let rid = run_id.clone();
        let rtag = run_tag.clone();
        let pr = prompt.clone();
        let proj = fresh_project.clone();
        let br = brand.clone();
        let ad = art_direction.clone();
        let c = cancel.clone();
        let ops = ops_counter.clone();
        let agent_id = format!("gfx-{:02}", scene.scene_index);
        let save_asset = scene.scene_index == 1;

        join_set.spawn(async move {
            let first = run_programmatic_scene_worker(
                &b,
                &or,
                &wm,
                &wfb,
                worker_supports_reasoning,
                &rid,
                &rtag,
                &proj,
                &br,
                &ad,
                &pr,
                &scene,
                total_scenes,
                save_asset,
                &agent_id,
                GFX_SWARM_LOCK_OWNER,
                &c,
                &ops,
            )
            .await;

            match first {
                Ok(_) | Err(AgentError::Cancelled) => (agent_id, false, None),
                Err(err) => {
                    let fatal = is_unrecoverable_openrouter_error(&err);
                    // Per-scene self-healing: retry ONLY this failed scene once on director_model
                    if !fatal && dm != wm && !c.is_cancelled() {
                        emit_swarm_log(
                            b.as_ref(),
                            LogLevel::Warn,
                            "swarm",
                            &proj.id,
                            &rid,
                            Some(&agent_id),
                            format!(
                                "{agent_id} failed on {wm}; retrying Scene {} on {dm}",
                                scene.scene_index
                            ),
                            Some(err.to_string()),
                        );
                        match run_programmatic_scene_worker(
                            &b,
                            &or,
                            &dm,
                            &dfb,
                            director_supports_reasoning,
                            &rid,
                            &rtag,
                            &proj,
                            &br,
                            &ad,
                            &pr,
                            &scene,
                            total_scenes,
                            save_asset,
                            &agent_id,
                            GFX_SWARM_LOCK_OWNER,
                            &c,
                            &ops,
                        )
                        .await
                        {
                            Ok(_) | Err(AgentError::Cancelled) => return (agent_id, false, None),
                            Err(retry_err) => {
                                return (
                                    agent_id,
                                    is_unrecoverable_openrouter_error(&retry_err),
                                    Some(retry_err.to_string()),
                                );
                            }
                        }
                    }
                    (agent_id, fatal, Some(err.to_string()))
                }
            }
        });
    }

    let mut fatal_any = false;
    let mut scene_errors: Vec<String> = Vec::new();
    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                join_set.abort_all();
                break;
            }
            next = join_set.join_next() => {
                let Some(res) = next else {
                    break;
                };
                match res {
                    Ok((aid, fatal, err)) => {
                        if fatal {
                            fatal_any = true;
                        }
                        if let Some(e) = err {
                            scene_errors.push(format!("{aid}: {e}"));
                        }
                    }
                    Err(join_err) => {
                        scene_errors.push(format!("gfx-scene-task: {join_err}"));
                    }
                }
            }
        }
    }

    lock_guard.release().await;

    if cancel.is_cancelled() {
        return (false, fatal_any, None);
    }

    let mut used_fallback = false;
    if !scene_errors.is_empty() && allow_local_fallback && !cancel.is_cancelled() {
        let g1_empty = backend
            .get_project(&project.id)
            .await
            .ok()
            .and_then(|p| p.tracks.into_iter().find(|t| t.id == "g1"))
            .is_none_or(|t| t.clips.is_empty());
        if g1_empty
            && let Ok(fb_lock) = TrackLockGuard::acquire(
                backend.clone(),
                &project.id,
                &["g1"],
                "gfx-01",
                AgentRole::Gfx,
            )
            .await
        {
            used_fallback = true;
            let _ = execute_deterministic_specialist(
                &backend,
                &fresh_project,
                &brand,
                &assets,
                &plan,
                &prompt,
                AgentRole::Gfx,
                &cancel,
                &ops_counter,
            )
            .await;
            fb_lock.release().await;
            return (used_fallback, fatal_any, None);
        }
    }

    let error_note = if scene_errors.is_empty() {
        None
    } else {
        scene_errors.sort();
        Some(format!("gfx-fanout: {}", scene_errors.join("; ")))
    };
    (used_fallback, fatal_any, error_note)
}

#[allow(clippy::too_many_arguments)]
async fn run_specialist_with_lock(
    backend: Arc<dyn SolarisBackend>,
    openrouter: OpenRouterClient,
    use_llm: bool,
    allow_local_fallback: bool,
    model: String,
    fallbacks: Vec<String>,
    low_reasoning: bool,
    run_id: String,
    programmatic_visuals: bool,
    prompt: String,
    project: solaris_core_ir::Project,
    brand: BrandPlaybook,
    assets: Vec<SavedCodeAsset>,
    plan: DirectorPlan,
    track_id: &'static str,
    agent_id: &'static str,
    role: AgentRole,
    max_turns: usize,
    cancel: CancellationToken,
    ops_counter: Arc<AtomicU32>,
) -> (bool, bool, Option<String>) {
    if cancel.is_cancelled() {
        return (false, false, None);
    }
    let lock_guard =
        match TrackLockGuard::acquire(backend.clone(), &project.id, &[track_id], agent_id, role)
            .await
        {
            Ok(g) => g,
            Err(err) => return (false, false, Some(format!("{agent_id}: skipped ({err})"))),
        };

    let anchor = match role {
        AgentRole::Grade => "color.inspector",
        AgentRole::Caps => "captions.inspector",
        AgentRole::Gfx => "effects.library",
        AgentRole::Mix => "audio.mixer",
        _ => "timeline.track.v2",
    };
    let _ = backend
        .emit_cursor_pulse(
            &project.id,
            agent_id,
            role,
            anchor,
            "executing specialist pass",
        )
        .await;

    let instruction = match role {
        AgentRole::BRoll => format!("You own v2 (B-ROLL). {}", plan.broll_notes),
        AgentRole::Grade => format!("You own fx (GRADE). {}", plan.grade_notes),
        AgentRole::Caps => format!(
            "You own t1 (TEXT). User request: \"{}\". Plan: {}",
            prompt, plan.captions_notes
        ),
        AgentRole::Gfx => format!(
            "You own g1 (GFX). User request: \"{}\". Plan: {}",
            prompt, plan.gfx_notes
        ),
        AgentRole::Mix => format!("You own a2 (MUSIC). {}", plan.mix_notes),
        _ => String::new(),
    };

    let mut used_fallback = !use_llm;
    let mut openrouter_fatal = false;
    let mut error_note: Option<String> = None;
    if use_llm {
        match run_llm_worker_loop(
            &backend,
            &openrouter,
            &model,
            &fallbacks,
            low_reasoning,
            &run_id,
            programmatic_visuals,
            plan.target_duration_sec,
            &project,
            &brand,
            &assets,
            agent_id,
            role,
            &[track_id],
            &instruction,
            max_turns,
            SwarmRunMode::Generate,
            &cancel,
            &ops_counter,
        )
        .await
        {
            Ok(0) if allow_local_fallback => {
                let fallback_ops = execute_deterministic_specialist(
                    &backend,
                    &project,
                    &brand,
                    &assets,
                    &plan,
                    &prompt,
                    role,
                    &cancel,
                    &ops_counter,
                )
                .await;
                if fallback_ops > 0 {
                    tracing::warn!(
                        "{agent_id} LLM loop committed 0 ops; local specialist fallback committed {fallback_ops} op(s)"
                    );
                    used_fallback = true;
                }
            }
            Ok(_) | Err(AgentError::Cancelled) => {}
            Err(err) if allow_local_fallback => {
                if is_unrecoverable_openrouter_error(&err) {
                    openrouter_fatal = true;
                }
                tracing::warn!(
                    "{agent_id} LLM loop failed ({err}); running local specialist fallback"
                );
                emit_swarm_log(
                    backend.as_ref(),
                    LogLevel::Warn,
                    "swarm",
                    &project.id,
                    &run_id,
                    Some(agent_id),
                    format!("{agent_id} failed ({err}); running local specialist fallback"),
                    None,
                );
                used_fallback = true;
                let _ = execute_deterministic_specialist(
                    &backend,
                    &project,
                    &brand,
                    &assets,
                    &plan,
                    &prompt,
                    role,
                    &cancel,
                    &ops_counter,
                )
                .await;
            }
            Err(err) => {
                if is_unrecoverable_openrouter_error(&err) {
                    openrouter_fatal = true;
                }
                emit_swarm_log(
                    backend.as_ref(),
                    LogLevel::Error,
                    "swarm",
                    &project.id,
                    &run_id,
                    Some(agent_id),
                    format!("{agent_id} failed: {err}"),
                    None,
                );
                error_note = Some(format!("{agent_id}: {err}"));
            }
        }
    } else {
        let _ = execute_deterministic_specialist(
            &backend,
            &project,
            &brand,
            &assets,
            &plan,
            &prompt,
            role,
            &cancel,
            &ops_counter,
        )
        .await;
    }

    lock_guard.release().await;
    (used_fallback, openrouter_fatal, error_note)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openrouter::OpenRouterConfig;
    use solaris_core_ir::{
        AspectPreset, MediaAsset, MediaIntelligenceManifest, MediaKind, Project, TimeSpan,
        TranscriptWord,
    };
    use std::collections::HashMap;

    use super::test_support::InMemBackend;

    #[tokio::test]
    async fn deterministic_swarm_cuts_silences_maps_captions_and_releases_all_locks() {
        let mut proj = Project::new_default("swarm_test", AspectPreset::Vertical9x16);
        let pid = proj.id.clone();
        proj.media_pool.push(MediaAsset {
            id: "m1".to_string(),
            name: "aroll.mp4".to_string(),
            kind: MediaKind::Video,
            role_hint: "a_roll".to_string(),
            path: "/tmp/a.mp4".to_string(),
            added_at: Utc::now(),
            intelligence: MediaIntelligenceManifest {
                analyzed: true,
                duration_sec: 6.0,
                is_log_footage: true,
                has_audio: true,
                integrated_lufs: Some(-18.0),
                silences: vec![TimeSpan {
                    start_sec: 2.0,
                    end_sec: 3.0,
                }],
                transcript: vec![
                    TranscriptWord {
                        word: "Three".into(),
                        start_sec: 0.2,
                        end_sec: 0.6,
                    },
                    TranscriptWord {
                        word: "AI".into(),
                        start_sec: 0.6,
                        end_sec: 1.0,
                    },
                    TranscriptWord {
                        word: "Tools".into(),
                        start_sec: 3.2,
                        end_sec: 3.8,
                    },
                ],
                ..MediaIntelligenceManifest::default()
            },
        });
        proj.media_pool.push(MediaAsset {
            id: "m2".to_string(),
            name: "broll.png".to_string(),
            kind: MediaKind::Image,
            role_hint: "b_roll".to_string(),
            path: "/tmp/b.png".to_string(),
            added_at: Utc::now(),
            intelligence: MediaIntelligenceManifest::default(),
        });
        proj.media_pool.push(MediaAsset {
            id: "m3".to_string(),
            name: "beat.wav".to_string(),
            kind: MediaKind::Audio,
            role_hint: "music".to_string(),
            path: "/tmp/beat.wav".to_string(),
            added_at: Utc::now(),
            intelligence: MediaIntelligenceManifest {
                analyzed: true,
                duration_sec: 12.0,
                has_audio: true,
                ..MediaIntelligenceManifest::default()
            },
        });

        let asset = SavedCodeAsset {
            id: "stat-card-solar".into(),
            name: "Stat Card".into(),
            category: "cards".into(),
            tags: vec![],
            props_schema: serde_json::json!({}),
            default_props: serde_json::json!({"value": "10X"}),
            html_source: r#"<div class="clip" data-start="0" data-duration="2.0" style="color:var(--brand-ink)">{{value}}</div>"#.into(),
            default_duration_sec: 2.0,
            created_at: Utc::now(),
        };

        let backend = InMemBackend::new(proj, vec![asset]);
        let client = OpenRouterClient::new(OpenRouterConfig::default());
        let advisor = Arc::new(ModelAdvisor::new(client.clone()));

        let status = run_swarm(
            backend.clone(),
            client,
            advisor,
            pid,
            "run-1".into(),
            SwarmRunRequest {
                prompt: "Edit a 9:16 hook reel".into(),
                model_override: None,
                allow_local_fallback: true,
                max_turns_per_worker: Some(2),
                skip_qa: false,
                approved_plan: None,
                mode: SwarmRunMode::Auto,
                selected_clip_ids: Vec::new(),
                playhead_sec: None,
                conversation_history: Vec::new(),
            },
            CancellationToken::new(),
            |_| {},
        )
        .await;

        assert_eq!(status.state, SwarmRunState::Completed);
        assert!(status.used_local_fallback);
        assert!(status.ops_committed >= 6);
        assert!(status.qa_report.is_some());

        let guard = backend.engine.read().await;
        assert_eq!(guard.track_locks(), &HashMap::new());
        let p = guard.project();
        // Silence [2.0, 3.0] cut into 2 V1 segments
        assert_eq!(
            p.tracks.iter().find(|t| t.id == "v1").unwrap().clips.len(),
            2
        );
        assert!(
            !p.tracks
                .iter()
                .find(|t| t.id == "v2")
                .unwrap()
                .clips
                .is_empty()
        );
        assert!(
            !p.tracks
                .iter()
                .find(|t| t.id == "t1")
                .unwrap()
                .clips
                .is_empty()
        );
        assert!(
            !p.tracks
                .iter()
                .find(|t| t.id == "g1")
                .unwrap()
                .clips
                .is_empty()
        );
        assert!(
            !p.tracks
                .iter()
                .find(|t| t.id == "a2")
                .unwrap()
                .clips
                .is_empty()
        );
        assert!((p.active_grade.contrast - 1.18).abs() < 1e-6);
    }

    #[tokio::test]
    async fn zero_asset_programmatic_swarm_builds_full_30s_explainer_on_g1_without_captions() {
        let proj = Project::new_default("agent_skills_explainer", AspectPreset::Vertical9x16);
        let pid = proj.id.clone();

        let backend = InMemBackend::new(proj, Vec::new());
        let client = OpenRouterClient::new(OpenRouterConfig::default());
        let advisor = Arc::new(ModelAdvisor::new(client.clone()));

        let status = run_swarm(
            backend.clone(),
            client,
            advisor,
            pid,
            "run-prog-1".into(),
            SwarmRunRequest {
                prompt: "Create a 30 seconds 9:16 explainer on how agent skills work".into(),
                model_override: None,
                allow_local_fallback: true,
                max_turns_per_worker: Some(4),
                skip_qa: false,
                approved_plan: None,
                mode: SwarmRunMode::Auto,
                selected_clip_ids: Vec::new(),
                playhead_sec: None,
                conversation_history: Vec::new(),
            },
            CancellationToken::new(),
            |_| {},
        )
        .await;

        assert_eq!(status.state, SwarmRunState::Completed);
        assert!(status.ops_committed >= 6);
        let qa = status.qa_report.expect("qa report");
        assert!(
            qa.checks.iter().any(
                |c| c.id == "gfx-lint-clean" && c.severity == solaris_core_ir::QaSeverity::Pass
            )
        );

        let guard = backend.engine.read().await;
        assert_eq!(guard.track_locks(), &HashMap::new());
        let p = guard.project();
        let g1 = p.tracks.iter().find(|t| t.id == "g1").unwrap();
        let t1 = p.tracks.iter().find(|t| t.id == "t1").unwrap();
        assert_eq!(g1.clips.len(), 6);
        assert!(
            t1.clips.is_empty(),
            "silent zero-asset videos must not get captions"
        );
        assert!((p.duration_sec() - 30.0).abs() < 0.5);
    }

    #[test]
    fn infers_duration_from_prompt_and_parses_partial_director_plan() {
        let proj = Project::new_default("dur_test", AspectPreset::Vertical9x16);
        assert_eq!(
            infer_target_duration_sec("Make a 30 seconds 9:16 explainer", &proj),
            30.0
        );
        assert_eq!(infer_target_duration_sec("45s launch teaser", &proj), 45.0);
        assert_eq!(
            infer_target_duration_sec("30s product highlight", &proj),
            30.0
        );
        assert_eq!(
            infer_target_duration_sec("1 minute product tour", &proj),
            60.0
        );
        assert_eq!(infer_target_duration_sec("1m product tour", &proj), 60.0);
        assert_eq!(infer_target_duration_sec("2m deep dive", &proj), 120.0);
        // Decade style descriptors and 4-digit year decades must NOT be mistaken for durations
        assert_eq!(
            infer_target_duration_sec("80s retro style neon montage", &proj),
            24.0
        );
        assert_eq!(
            infer_target_duration_sec("Inspired by the 90s cinema vibe", &proj),
            24.0
        );
        assert_eq!(
            infer_target_duration_sec("Design trends of the 2020s", &proj),
            24.0
        );
        assert_eq!(
            infer_target_duration_sec("1980s synthwave promo", &proj),
            24.0
        );

        let partial_json = r#"{"hook_summary":"Fast hook","gfx_notes":"6 scenes"}"#;
        let plan = llm::parse_director_plan(partial_json, 30.0).unwrap();
        assert_eq!(plan.hook_summary, "Fast hook");
        assert_eq!(plan.target_duration_sec, 30.0);
        assert_eq!(plan.gfx_notes, "6 scenes");
    }

    #[test]
    fn resolve_programmatic_scenes_handles_short_nan_oversized_and_outro_folding() {
        let make_scenes = |durs: &[f64]| -> Vec<SceneSpec> {
            durs.iter()
                .enumerate()
                .map(|(i, &d)| SceneSpec {
                    scene_index: i + 1,
                    title: format!("Beat {}", i + 1),
                    start_sec: i as f64,
                    duration_sec: d,
                    visual_brief: format!("Visual {}", i + 1),
                    motion_cue: "Stagger".into(),
                    narration: format!("Line {}", i + 1),
                })
                .collect()
        };

        // 1. Short durations (4.0s and 5.0s) with 7 scenes must never panic and must stay contiguous
        for target in [4.0, 5.0] {
            let plan = DirectorPlan {
                scenes: make_scenes(&[2.0, 2.0, 2.0, 2.0, 2.0, 2.0, 2.0]),
                ..DirectorPlan::default()
            };
            let res = resolve_programmatic_scenes(&plan, target);
            assert_eq!(res.len(), MAX_PARALLEL_SCENE_AGENTS);
            assert_eq!(res[0].start_sec, 0.0);
            let last = res.last().unwrap();
            assert!(
                ((last.start_sec + last.duration_sec) - target).abs() < 0.05,
                "expected total {target}, got {}",
                last.start_sec + last.duration_sec
            );
            for pair in res.windows(2) {
                let prev_end = pair[0].start_sec + pair[0].duration_sec;
                assert!((pair[1].start_sec - prev_end).abs() < 0.02);
            }
        }

        // 2. NaN target duration falls back safely to 24.0s
        let nan_plan = DirectorPlan {
            hook_summary: "NaN test".into(),
            ..DirectorPlan::default()
        };
        let nan_scenes = resolve_programmatic_scenes(&nan_plan, f64::NAN);
        assert!(!nan_scenes.is_empty());
        let nan_last = nan_scenes.last().unwrap();
        assert!(((nan_last.start_sec + nan_last.duration_sec) - 24.0).abs() < 0.05);

        // 3. Oversized 25s Scene 1 on a 30s timeline with 6 scenes must never panic
        let oversized_plan = DirectorPlan {
            scenes: make_scenes(&[25.0, 1.0, 1.0, 1.0, 1.0, 1.0]),
            ..DirectorPlan::default()
        };
        let oversized = resolve_programmatic_scenes(&oversized_plan, 30.0);
        assert_eq!(oversized.len(), 6);
        let over_last = oversized.last().unwrap();
        assert!(((over_last.start_sec + over_last.duration_sec) - 30.0).abs() < 0.05);
        for s in &oversized {
            assert!(s.duration_sec >= 1.2);
        }

        // 4. >7 scenes fold trailing outro briefs & narration into Scene 7 instead of dropping
        let mut nine = make_scenes(&[3.0; 9]);
        nine[8].title = "Final Outro CTA".into();
        nine[8].visual_brief = "Closing logo & subscribe button".into();
        nine[8].narration = "Try Solaris today!".into();
        let nine_plan = DirectorPlan {
            scenes: nine,
            ..DirectorPlan::default()
        };
        let folded = resolve_programmatic_scenes(&nine_plan, 30.0);
        assert_eq!(folded.len(), MAX_PARALLEL_SCENE_AGENTS);
        let seventh = &folded[MAX_PARALLEL_SCENE_AGENTS - 1];
        assert!(
            seventh
                .visual_brief
                .contains("Closing logo & subscribe button"),
            "expected folded outro brief in scene 7: {}",
            seventh.visual_brief
        );
        assert!(
            seventh.narration.contains("Try Solaris today!"),
            "expected folded outro narration in scene 7: {}",
            seventh.narration
        );
    }

    #[test]
    fn resolve_programmatic_scenes_stitches_structured_and_bracketed_plans_contiguously() {
        // 1. Bracketed `[start-end]` fallback parsing from `gfx_notes`
        let bracketed_plan = DirectorPlan {
            hook_summary: "Agent Skills Explainer".into(),
            target_duration_sec: 24.0,
            gfx_notes: "[0.0-3.0] Hook: kinetic title [3.0-6.5] Prompt vs Skill: comparison [6.5-10.0] Anatomy: folder explode [10.0-14.0] Loading: scan [14.0-20.0] Workflow: loop [20.0-24.0] Outro: CTA".into(),
            captions_notes: "[0.0-3.0] Skills make agents capable. [3.0-6.5] Not prompts.".into(),
            ..DirectorPlan::default()
        };
        let scenes = resolve_programmatic_scenes(&bracketed_plan, 24.0);
        assert_eq!(scenes.len(), 6);
        assert_eq!(scenes[0].start_sec, 0.0);
        assert_eq!(scenes[0].title, "Hook");
        assert_eq!(scenes[0].narration, "Skills make agents capable.");
        let last = scenes.last().unwrap();
        assert!(((last.start_sec + last.duration_sec) - 24.0).abs() < 0.05);
        for pair in scenes.windows(2) {
            let prev_end = pair[0].start_sec + pair[0].duration_sec;
            assert!(
                (pair[1].start_sec - prev_end).abs() < 0.02,
                "expected contiguous scenes, got gap between {prev_end} and {}",
                pair[1].start_sec
            );
        }

        // 2. Structured `scenes` capped at MAX_PARALLEL_SCENE_AGENTS and stitched with zero gaps
        let many_scenes: Vec<SceneSpec> = (0..10)
            .map(|i| SceneSpec {
                scene_index: i + 1,
                title: format!("Beat {}", i + 1),
                start_sec: (i as f64) * 3.0,
                duration_sec: 2.5,
                visual_brief: format!("Diagram {}", i + 1),
                motion_cue: "Stagger".into(),
                narration: String::new(),
            })
            .collect();
        let structured_plan = DirectorPlan {
            scenes: many_scenes,
            ..DirectorPlan::default()
        };
        let capped = resolve_programmatic_scenes(&structured_plan, 24.0);
        assert_eq!(capped.len(), MAX_PARALLEL_SCENE_AGENTS);
        let cap_last = capped.last().unwrap();
        assert!(((cap_last.start_sec + cap_last.duration_sec) - 24.0).abs() < 0.05);
    }
}
