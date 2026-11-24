mod deterministic;
mod llm;
mod qa;

pub use qa::run_qa_pass;

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use solaris_core_ir::{
    AgentRole, BrandPlaybook, MediaKind, SavedCodeAsset, SwarmRunState, SwarmRunStatus,
};
use tokio_util::sync::CancellationToken;

use crate::advisor::{ModelAdvisor, ModelRoleKind};
use crate::backend::SolarisBackend;
use crate::error::AgentError;
use crate::openrouter::OpenRouterClient;
use deterministic::{execute_deterministic_specialist, execute_deterministic_spine};
use llm::{build_openrouter_director_plan, run_llm_worker_loop};

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

fn is_music_or_sfx_hint(hint: &str) -> bool {
    matches!(
        hint.to_ascii_lowercase().as_str(),
        "music" | "bgm" | "beat" | "soundtrack" | "sfx" | "sound_effect" | "ambience"
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
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct DirectorPlan {
    pub hook_summary: String,
    pub target_duration_sec: f64,
    pub spine_notes: String,
    pub broll_notes: String,
    pub grade_notes: String,
    pub captions_notes: String,
    pub gfx_notes: String,
    pub mix_notes: String,
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
                MediaKind::Video => m.intelligence.has_audio && !is_broll_hint(&m.role_hint),
                MediaKind::Audio => {
                    is_voice_hint(&m.role_hint)
                        || (!has_any_video
                            && prompt_mentions_vo
                            && !is_music_or_sfx_hint(&m.role_hint))
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

    let (
        director_model,
        director_fallbacks,
        video_director_fallbacks,
        audio_director_fallbacks,
        worker_model,
        worker_fallbacks,
        supports_native_video,
        supports_native_audio,
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
        let dir_inputs = advisor.model_input_modalities(&primary_dir).await;
        let native_audio = dir_inputs.iter().any(|m| m == "audio");
        let native_video = dir_inputs.iter().any(|m| m == "video") && native_audio;
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
            req.model_override.clone().unwrap_or(wrk_rec.primary),
            wrk_rec.fallbacks,
            native_video,
            native_audio,
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
        )
    };

    status.model_used = Some(director_model.clone());
    status.used_local_fallback = !use_openrouter;
    let mut openrouter_unrecoverable = !use_openrouter;

    // Stage 1b: Director Plan
    status.phase_label = "Director inspecting media intelligence & brand playbook".to_string();
    status.updated_at = Utc::now();
    on_status(status.clone());

    let plan = if use_openrouter {
        match build_openrouter_director_plan(
            backend.as_ref(),
            &openrouter,
            &director_model,
            &director_fallbacks,
            &video_director_fallbacks,
            &audio_director_fallbacks,
            supports_native_video,
            supports_native_audio,
            &project,
            &brand,
            &assets,
            &req.prompt,
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
                status.used_local_fallback = true;
                build_deterministic_plan(&project, &brand, &req.prompt)
            }
            Err(err) => {
                return finish_failed(status, err.to_string(), &ops_counter, &on_status);
            }
        }
    } else {
        build_deterministic_plan(&project, &brand, &req.prompt)
    };

    if cancel.is_cancelled() {
        return finish_cancelled(status, &ops_counter, &on_status);
    }

    // Stage 2: Spine (razor-01 on v1 + a1)
    status.state = SwarmRunState::ExecutingSpine;
    status.phase_label = "Razor cutting A-Roll & voice spine (V1/A1)".to_string();
    status.active_roles = vec![AgentRole::Razor];
    status.updated_at = Utc::now();
    on_status(status.clone());

    let max_turns = req.max_turns_per_worker.unwrap_or(6).clamp(1, 10);

    let spine_res = run_spine_stage(
        &backend,
        &openrouter,
        use_openrouter && !openrouter_unrecoverable,
        req.allow_local_fallback,
        &worker_model,
        &worker_fallbacks,
        &project,
        &brand,
        &assets,
        &plan,
        max_turns,
        &cancel,
        &ops_counter,
    )
    .await;

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
            return finish_failed(status, err.to_string(), &ops_counter, &on_status);
        }
    }

    if cancel.is_cancelled() {
        return finish_cancelled(status, &ops_counter, &on_status);
    }

    // Stage 3: Parallel 5-Specialist Fan-Out
    status.state = SwarmRunState::ExecutingSpecialists;
    status.phase_label = "Parallel specialists editing V2, FX, T1, G1 & A2".to_string();
    status.active_roles = vec![
        AgentRole::BRoll,
        AgentRole::Grade,
        AgentRole::Caps,
        AgentRole::Gfx,
        AgentRole::Mix,
    ];
    status.ops_committed = ops_counter.load(Ordering::Relaxed);
    status.updated_at = Utc::now();
    on_status(status.clone());

    let refreshed = backend.get_project(&project_id).await.unwrap_or(project);
    let use_llm_workers = use_openrouter && !openrouter_unrecoverable;

    let (r_broll, r_grade, r_caps, r_gfx, r_mix) = tokio::join!(
        run_specialist_with_lock(
            backend.clone(),
            openrouter.clone(),
            use_llm_workers,
            req.allow_local_fallback,
            worker_model.clone(),
            worker_fallbacks.clone(),
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
        ),
        run_specialist_with_lock(
            backend.clone(),
            openrouter.clone(),
            use_llm_workers,
            req.allow_local_fallback,
            worker_model.clone(),
            worker_fallbacks.clone(),
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
        ),
        run_specialist_with_lock(
            backend.clone(),
            openrouter.clone(),
            use_llm_workers,
            req.allow_local_fallback,
            worker_model.clone(),
            worker_fallbacks.clone(),
            refreshed.clone(),
            brand.clone(),
            assets.clone(),
            plan.clone(),
            "t1",
            "caps-01",
            AgentRole::Caps,
            max_turns,
            cancel.clone(),
            ops_counter.clone(),
        ),
        run_specialist_with_lock(
            backend.clone(),
            openrouter.clone(),
            use_llm_workers,
            req.allow_local_fallback,
            worker_model.clone(),
            worker_fallbacks.clone(),
            refreshed.clone(),
            brand.clone(),
            assets.clone(),
            plan.clone(),
            "g1",
            "gfx-01",
            AgentRole::Gfx,
            max_turns,
            cancel.clone(),
            ops_counter.clone(),
        ),
        run_specialist_with_lock(
            backend.clone(),
            openrouter.clone(),
            use_llm_workers,
            req.allow_local_fallback,
            worker_model.clone(),
            worker_fallbacks.clone(),
            refreshed,
            brand,
            assets,
            plan.clone(),
            "a2",
            "mix-01",
            AgentRole::Mix,
            max_turns,
            cancel.clone(),
            ops_counter.clone(),
        ),
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
    let specialist_errors: Vec<String> = specialist_results
        .into_iter()
        .filter_map(|(_, _, err)| err)
        .collect();

    if !req.allow_local_fallback && specialist_errors.len() == 5 {
        return finish_failed(
            status,
            format!(
                "All specialists failed without fallback: {}",
                specialist_errors.join("; ")
            ),
            &ops_counter,
            &on_status,
        );
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
        let qa = run_qa_pass(
            backend.as_ref(),
            qa_client,
            vision,
            &project_id,
            true,
            &cancel,
            Some(&ops_counter),
        )
        .await;
        status.qa_report = Some(qa);
    }

    if cancel.is_cancelled() {
        return finish_cancelled(status, &ops_counter, &on_status);
    }

    status.state = SwarmRunState::Completed;
    status.active_roles = Vec::new();
    status.ops_committed = ops_counter.load(Ordering::Relaxed);
    status.phase_label = if let Some(ref qa) = status.qa_report {
        format!(
            "Swarm complete ({} ops committed · QA {}/100)",
            status.ops_committed, qa.score
        )
    } else {
        format!("Swarm complete ({} ops committed)", status.ops_committed)
    };
    status.summary = Some(plan.hook_summary);
    if !specialist_errors.is_empty() {
        status.error = Some(specialist_errors.join("; "));
    }
    status.updated_at = Utc::now();
    on_status(status.clone());
    status
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
    let max_dur = project
        .media_pool
        .iter()
        .map(|m| m.intelligence.duration_sec)
        .fold(0.0_f64, f64::max)
        .clamp(6.0, 30.0);
    DirectorPlan {
        hook_summary: format!(
            "{} ({})",
            prompt.chars().take(72).collect::<String>(),
            brand.name
        ),
        target_duration_sec: max_dur,
        spine_notes:
            "Assemble A-Roll/Voice on V1/A1, cut >0.25s silences with 0.12s pad, 1.08x hook zoom."
                .to_string(),
        broll_notes: "Place B-Roll/Image inserts on V2 with ken_burns on stills.".to_string(),
        grade_notes: "Recover Log footage to Rec.709 and blend brand LUT if present.".to_string(),
        captions_notes: "Generate 2-4 word social safe-zone captions on T1.".to_string(),
        gfx_notes: "Place Brand-tokenized title/stat cards on G1.".to_string(),
        mix_notes: "Normalize speech to -14 LUFS and enable -18 dB ducking on A2.".to_string(),
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
async fn run_specialist_with_lock(
    backend: Arc<dyn SolarisBackend>,
    openrouter: OpenRouterClient,
    use_llm: bool,
    allow_local_fallback: bool,
    model: String,
    fallbacks: Vec<String>,
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
        AgentRole::Caps => format!("You own t1 (TEXT). {}", plan.captions_notes),
        AgentRole::Gfx => format!("You own g1 (GFX). {}", plan.gfx_notes),
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
            &project,
            &brand,
            &assets,
            agent_id,
            role,
            &[track_id],
            &instruction,
            max_turns,
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
                used_fallback = true;
                let _ = execute_deterministic_specialist(
                    &backend,
                    &project,
                    &brand,
                    &assets,
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
                error_note = Some(format!("{agent_id}: {err}"));
            }
        }
    } else {
        let _ = execute_deterministic_specialist(
            &backend,
            &project,
            &brand,
            &assets,
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
    use crate::backend::{BoxFuture, CapturedFrame, KeyframeResult};
    use crate::openrouter::OpenRouterConfig;
    use solaris_core_ir::{
        AspectPreset, CommandMeta, ExportJobStatus, ExportRequest, MediaAsset,
        MediaIntelligenceManifest, MediaKind, OpResult, Project, ProjectEngine, TimeSpan,
        TimelineCommand, TranscriptWord,
    };
    use std::collections::HashMap;
    use tokio::sync::RwLock;

    struct InMemBackend {
        engine: RwLock<ProjectEngine>,
        assets: Vec<SavedCodeAsset>,
        brand: BrandPlaybook,
    }

    impl SolarisBackend for InMemBackend {
        fn list_projects(&self) -> BoxFuture<'_, Result<Vec<serde_json::Value>, AgentError>> {
            Box::pin(async { Ok(vec![]) })
        }
        fn create_project(
            &self,
            _name: String,
            _aspect: Option<AspectPreset>,
            _brand: Option<String>,
            _prompt: Option<String>,
            _template: Option<String>,
        ) -> BoxFuture<'_, Result<Project, AgentError>> {
            Box::pin(async { Err(AgentError::Backend("unused".into())) })
        }
        fn get_project(&self, _id: &str) -> BoxFuture<'_, Result<Project, AgentError>> {
            Box::pin(async { Ok(self.engine.read().await.project().clone()) })
        }
        fn apply_op(
            &self,
            _id: &str,
            cmd: TimelineCommand,
            meta: CommandMeta,
        ) -> BoxFuture<'_, Result<(OpResult, Project), AgentError>> {
            Box::pin(async move {
                let mut g = self.engine.write().await;
                let (res, _) = g
                    .apply(cmd, meta)
                    .map_err(|e| AgentError::ToolError(e.to_string()))?;
                Ok((res, g.project().clone()))
            })
        }
        fn undo(&self, _id: &str) -> BoxFuture<'_, Result<Project, AgentError>> {
            Box::pin(async { Err(AgentError::Backend("unused".into())) })
        }
        fn redo(&self, _id: &str) -> BoxFuture<'_, Result<Project, AgentError>> {
            Box::pin(async { Err(AgentError::Backend("unused".into())) })
        }
        fn acquire_track_lock(
            &self,
            _pid: &str,
            tid: &str,
            aid: &str,
            role: AgentRole,
        ) -> BoxFuture<'_, Result<(), AgentError>> {
            let tid = tid.to_string();
            let aid = aid.to_string();
            Box::pin(async move {
                let mut g = self.engine.write().await;
                g.acquire_track_lock(&tid, aid, role)
                    .map(|_| ())
                    .map_err(|e| AgentError::ToolError(e.to_string()))
            })
        }
        fn release_track_lock(
            &self,
            _pid: &str,
            tid: &str,
            aid: &str,
        ) -> BoxFuture<'_, Result<(), AgentError>> {
            let tid = tid.to_string();
            let aid = aid.to_string();
            Box::pin(async move {
                let mut g = self.engine.write().await;
                g.release_track_lock(&tid, &aid)
                    .map(|_| ())
                    .map_err(|e| AgentError::ToolError(e.to_string()))
            })
        }
        fn import_media_paths(
            &self,
            _pid: &str,
            _paths: Vec<(String, Option<String>)>,
        ) -> BoxFuture<'_, Result<Vec<MediaAsset>, AgentError>> {
            Box::pin(async { Ok(vec![]) })
        }
        fn transcribe_media(
            &self,
            _pid: &str,
            _mid: &str,
            _lang: Option<String>,
        ) -> BoxFuture<'_, Result<Vec<TranscriptWord>, AgentError>> {
            Box::pin(async { Ok(vec![]) })
        }
        fn extract_keyframes(
            &self,
            _pid: &str,
            _mid: &str,
            _ts: Vec<f64>,
        ) -> BoxFuture<'_, Result<Vec<KeyframeResult>, AgentError>> {
            Box::pin(async { Ok(vec![]) })
        }
        fn capture_preview_frame(
            &self,
            _pid: &str,
            _ts: f64,
        ) -> BoxFuture<'_, Result<CapturedFrame, AgentError>> {
            Box::pin(async { Err(AgentError::Backend("unused".into())) })
        }
        fn list_brands(&self) -> BoxFuture<'_, Result<Vec<BrandPlaybook>, AgentError>> {
            Box::pin(async { Ok(vec![self.brand.clone()]) })
        }
        fn get_brand(&self, _bid: &str) -> BoxFuture<'_, Result<BrandPlaybook, AgentError>> {
            Box::pin(async { Ok(self.brand.clone()) })
        }
        fn list_assets(&self) -> BoxFuture<'_, Result<Vec<SavedCodeAsset>, AgentError>> {
            Box::pin(async { Ok(self.assets.clone()) })
        }
        fn save_asset(
            &self,
            a: SavedCodeAsset,
        ) -> BoxFuture<'_, Result<SavedCodeAsset, AgentError>> {
            Box::pin(async { Ok(a) })
        }
        fn start_export(
            &self,
            _pid: &str,
            _req: ExportRequest,
        ) -> BoxFuture<'_, Result<ExportJobStatus, AgentError>> {
            Box::pin(async { Err(AgentError::Backend("unused".into())) })
        }
        fn emit_cursor_pulse(
            &self,
            _pid: &str,
            _aid: &str,
            _role: AgentRole,
            _anchor: &str,
            _label: &str,
        ) -> BoxFuture<'_, Result<(), AgentError>> {
            Box::pin(async { Ok(()) })
        }
        fn emit_playhead_seek(
            &self,
            _pid: &str,
            _ts: f64,
            _aid: &str,
            _role: AgentRole,
        ) -> BoxFuture<'_, Result<(), AgentError>> {
            Box::pin(async { Ok(()) })
        }
    }

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

        let backend = Arc::new(InMemBackend {
            engine: RwLock::new(ProjectEngine::new(proj)),
            assets: vec![asset],
            brand: BrandPlaybook::solaris_default(),
        });
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
}
