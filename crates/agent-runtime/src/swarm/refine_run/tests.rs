use std::time::Duration;

use solaris_core_ir::{AspectPreset, AudioSettings, Clip, ClipKind, Transform2D};

use super::*;
use crate::swarm::run_swarm;
use crate::swarm::test_support::{
    InMemBackend, StubReply, chat_reply, local_request, offline_client, spawn_openrouter_stub,
    stub_client,
};

const GRADE_PLAN: &str = r#"{"summary":"Lift the contrast on the grade.","target_roles":["grade"],"role_instructions":{"grade":"Raise contrast on the FX master grade."}}"#;

fn cut_project() -> Project {
    let mut project = Project::new_default("refine", AspectPreset::Vertical9x16);
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
    project
}

fn refine_request(skip_qa: bool) -> SwarmRunRequest {
    SwarmRunRequest {
        mode: SwarmRunMode::Refine,
        selected_clip_ids: vec!["hook-1".to_string()],
        playhead_sec: Some(3.2),
        skip_qa,
        ..local_request("Make the hook punchier")
    }
}

async fn run_refine(
    client: OpenRouterClient,
    backend: Arc<InMemBackend>,
    cancel: CancellationToken,
    req: SwarmRunRequest,
) -> SwarmRunStatus {
    let pid = backend.engine.read().await.project().id.clone();
    run_swarm(
        backend,
        client.clone(),
        Arc::new(ModelAdvisor::new(client)),
        pid,
        "run-refine".to_string(),
        req,
        cancel,
        |_| {},
    )
    .await
}

#[tokio::test]
async fn refine_without_an_ai_model_fails_without_falling_back() {
    let backend = InMemBackend::new(cut_project(), Vec::new());
    let status = run_refine(
        offline_client(),
        backend,
        CancellationToken::new(),
        refine_request(true),
    )
    .await;
    assert_eq!(status.state, SwarmRunState::Failed);
    assert_eq!(status.error.as_deref(), Some(REFINE_NEEDS_MODEL));
}

#[tokio::test]
async fn refine_waiting_on_the_planner_honours_cancel() {
    let base = spawn_openrouter_stub(vec![StubReply::Stall]).await;
    let cancel = CancellationToken::new();
    let canceller = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        canceller.cancel();
    });
    let status = tokio::time::timeout(
        Duration::from_secs(10),
        run_refine(
            stub_client(base),
            InMemBackend::new(cut_project(), Vec::new()),
            cancel,
            refine_request(true),
        ),
    )
    .await
    .expect("refine honours cancel");
    assert_eq!(status.state, SwarmRunState::Cancelled);
}

#[tokio::test]
async fn cancel_during_a_worker_releases_its_track_lock() {
    let base = spawn_openrouter_stub(vec![chat_reply(GRADE_PLAN), StubReply::Stall]).await;
    let backend = InMemBackend::new(cut_project(), Vec::new());
    let cancel = CancellationToken::new();
    let watcher = backend.clone();
    let canceller = cancel.clone();
    tokio::spawn(async move {
        for _ in 0..500 {
            if watcher.engine.read().await.track_locks().contains_key("fx") {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        canceller.cancel();
    });
    let status = tokio::time::timeout(
        Duration::from_secs(10),
        run_refine(
            stub_client(base),
            backend.clone(),
            cancel,
            refine_request(true),
        ),
    )
    .await
    .expect("refine honours cancel");
    assert_eq!(status.state, SwarmRunState::Cancelled);
    assert!(backend.engine.read().await.track_locks().is_empty());
}

#[tokio::test]
async fn malformed_planner_json_is_retried_once_then_fails_clearly() {
    let base =
        spawn_openrouter_stub(vec![chat_reply("not json"), chat_reply("still not json")]).await;
    let status = run_refine(
        stub_client(base),
        InMemBackend::new(cut_project(), Vec::new()),
        CancellationToken::new(),
        refine_request(true),
    )
    .await;
    assert_eq!(status.state, SwarmRunState::Failed);
    assert!(
        status
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("malformed JSON after retry")
    );
}

#[tokio::test]
async fn planner_retry_recovers_and_only_the_target_role_runs() {
    let base = spawn_openrouter_stub(vec![chat_reply("not json"), chat_reply(GRADE_PLAN)]).await;
    let status = run_refine(
        stub_client(base),
        InMemBackend::new(cut_project(), Vec::new()),
        CancellationToken::new(),
        refine_request(true),
    )
    .await;
    assert_eq!(status.state, SwarmRunState::Completed);
    assert_eq!(
        status.refinement_summary.as_deref(),
        Some("Lift the contrast on the grade. No clips were changed.")
    );
    assert_eq!(
        Some(status.phase_label.as_str()),
        status.refinement_summary.as_deref()
    );
    assert!(status.qa_report.is_none());
}

#[tokio::test]
async fn refine_runs_qa_unless_skipped_and_labels_the_run_with_its_summary() {
    let base = spawn_openrouter_stub(vec![chat_reply(GRADE_PLAN)]).await;
    let status = run_refine(
        stub_client(base),
        InMemBackend::new(cut_project(), Vec::new()),
        CancellationToken::new(),
        refine_request(false),
    )
    .await;
    assert_eq!(status.state, SwarmRunState::Completed);
    assert!(status.qa_report.is_some());
    assert_eq!(
        Some(status.phase_label.as_str()),
        status.refinement_summary.as_deref()
    );
}

#[tokio::test]
async fn empty_target_roles_finish_as_no_change_without_dispatch_or_qa() {
    let base = spawn_openrouter_stub(vec![chat_reply(
        r#"{"summary":"The hook already works.","target_roles":[]}"#,
    )])
    .await;
    let status = run_refine(
        stub_client(base),
        InMemBackend::new(cut_project(), Vec::new()),
        CancellationToken::new(),
        refine_request(false),
    )
    .await;
    assert_eq!(status.state, SwarmRunState::Completed);
    assert_eq!(
        status.refinement_summary.as_deref(),
        Some("No change needed: The hook already works.")
    );
    assert_eq!(status.ops_committed, 0);
    assert!(status.qa_report.is_none());
}

#[tokio::test]
async fn locked_target_tracks_are_reported_and_never_dispatched() {
    let mut project = cut_project();
    project
        .tracks
        .iter_mut()
        .find(|t| t.id == "fx")
        .expect("fx track")
        .locked = true;
    let backend = InMemBackend::new(project, Vec::new());
    let base = spawn_openrouter_stub(vec![chat_reply(GRADE_PLAN)]).await;
    let status = run_refine(
        stub_client(base),
        backend.clone(),
        CancellationToken::new(),
        refine_request(true),
    )
    .await;
    assert_eq!(status.state, SwarmRunState::Completed);
    assert_eq!(
        status.refinement_summary.as_deref(),
        Some("No change needed: Lift the contrast on the grade. Skipped locked tracks: fx.")
    );
    assert!(backend.engine.read().await.track_locks().is_empty());
}

#[tokio::test]
async fn track_lock_guard_releases_its_lock_when_dropped() {
    let project = cut_project();
    let pid = project.id.clone();
    let backend = InMemBackend::new(project, Vec::new());
    let guard =
        TrackLockGuard::acquire(backend.clone(), &pid, &["fx"], "grade-01", AgentRole::Grade)
            .await
            .expect("lock acquired");
    assert!(backend.engine.read().await.track_locks().contains_key("fx"));
    drop(guard);
    for _ in 0..100 {
        if backend.engine.read().await.track_locks().is_empty() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("track lock survived the guard drop");
}

#[test]
fn describe_targets_lists_each_role_with_its_instruction() {
    let plan = RefinementPlan {
        summary: "s".to_string(),
        targets: vec![RefinementTarget {
            role: AgentRole::Caps,
            instruction: "Shorten captions".to_string(),
        }],
    };
    assert_eq!(describe_targets(&plan), "caps-01: Shorten captions");
}
