use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use chrono::Utc;
use solaris_core_ir::{AgentRole, SwarmRunState, SwarmRunStatus};

/// Linearly interpolate progress within `[start, end]` clamped to `[0.0, 1.0]`.
pub(crate) fn lerp_progress(start: f32, end: f32, done: usize, total: usize) -> f32 {
    if total == 0 {
        return end.clamp(0.0, 1.0);
    }
    let frac = (done as f32 / total as f32).clamp(0.0, 1.0);
    (start + (end - start) * frac).clamp(0.0, 1.0)
}

/// Thread-safe, monotonic progress tracker for a single swarm run.
///
/// Holds a sync `Mutex` only long enough to update the status snapshot and push
/// it into the sequential `on_status` channel; never held across `.await`.
pub(crate) struct SwarmProgressTracker {
    status: Mutex<SwarmRunStatus>,
    ops_counter: Arc<AtomicU32>,
    on_status: Arc<dyn Fn(SwarmRunStatus) + Send + Sync>,
}

impl SwarmProgressTracker {
    pub(crate) fn new(
        mut initial: SwarmRunStatus,
        ops_counter: Arc<AtomicU32>,
        on_status: Arc<dyn Fn(SwarmRunStatus) + Send + Sync>,
    ) -> Self {
        initial.progress = if initial.progress.is_finite() {
            initial.progress.clamp(0.0, 1.0)
        } else {
            0.0
        };
        Self {
            status: Mutex::new(initial),
            ops_counter,
            on_status,
        }
    }

    pub(crate) fn emit_current(&self) {
        let guard = self.status.lock().unwrap_or_else(|e| e.into_inner());
        (self.on_status)(guard.clone());
    }

    pub(crate) fn update(&self, f: impl FnOnce(&mut SwarmRunStatus)) {
        let mut guard = self.status.lock().unwrap_or_else(|e| e.into_inner());
        let prev_progress = guard.progress;
        f(&mut guard);
        guard.progress = if guard.progress.is_finite() {
            guard.progress.clamp(prev_progress, 1.0)
        } else {
            prev_progress
        };
        guard.ops_committed = self.ops_counter.load(Ordering::Relaxed);
        guard.updated_at = Utc::now();
        (self.on_status)(guard.clone());
    }

    pub(crate) fn advance(&self, target_progress: f32, phase_label: Option<String>) {
        self.update(|s| {
            s.progress = target_progress;
            if let Some(label) = phase_label {
                s.phase_label = label;
            }
        });
    }

    pub(crate) fn complete_role(
        &self,
        role: Option<AgentRole>,
        target_progress: f32,
        phase_label: Option<String>,
    ) {
        self.update(|s| {
            if let Some(r) = role {
                s.active_roles.retain(|active| *active != r);
            }
            s.progress = target_progress;
            if let Some(label) = phase_label {
                s.phase_label = label;
            }
        });
    }

    pub(crate) fn finish_completed(&self, f: impl FnOnce(&mut SwarmRunStatus)) -> SwarmRunStatus {
        let mut guard = self.status.lock().unwrap_or_else(|e| e.into_inner());
        guard.state = SwarmRunState::Completed;
        guard.progress = 1.0;
        guard.active_roles = Vec::new();
        f(&mut guard);
        guard.progress = 1.0;
        guard.ops_committed = self.ops_counter.load(Ordering::Relaxed);
        guard.updated_at = Utc::now();
        let snapshot = guard.clone();
        (self.on_status)(snapshot.clone());
        snapshot
    }

    pub(crate) fn finish_failed(&self, err: String) -> SwarmRunStatus {
        let mut guard = self.status.lock().unwrap_or_else(|e| e.into_inner());
        guard.state = SwarmRunState::Failed;
        guard.active_roles = Vec::new();
        guard.phase_label = "Swarm failed".to_string();
        guard.error = Some(err);
        guard.ops_committed = self.ops_counter.load(Ordering::Relaxed);
        guard.updated_at = Utc::now();
        let snapshot = guard.clone();
        (self.on_status)(snapshot.clone());
        snapshot
    }

    pub(crate) fn finish_cancelled(&self) -> SwarmRunStatus {
        let mut guard = self.status.lock().unwrap_or_else(|e| e.into_inner());
        guard.state = SwarmRunState::Cancelled;
        guard.active_roles = Vec::new();
        guard.phase_label = "Swarm cancelled".to_string();
        guard.ops_committed = self.ops_counter.load(Ordering::Relaxed);
        guard.updated_at = Utc::now();
        let snapshot = guard.clone();
        (self.on_status)(snapshot.clone());
        snapshot
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_status(progress: f32) -> SwarmRunStatus {
        let now = Utc::now();
        SwarmRunStatus {
            run_id: "run-1".to_string(),
            project_id: "proj-1".to_string(),
            state: SwarmRunState::Planning,
            phase_label: "Planning".to_string(),
            progress,
            model_used: None,
            used_local_fallback: false,
            active_roles: vec![AgentRole::Director, AgentRole::Gfx],
            ops_committed: 0,
            summary: None,
            error: None,
            qa_report: None,
            refinement_summary: None,
            started_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn lerp_progress_handles_zero_total_and_bounds() {
        assert!((lerp_progress(0.2, 0.8, 0, 0) - 0.8).abs() < 1e-6);
        assert!((lerp_progress(0.2, 0.8, 0, 4) - 0.2).abs() < 1e-6);
        assert!((lerp_progress(0.2, 0.8, 2, 4) - 0.5).abs() < 1e-6);
        assert!((lerp_progress(0.2, 0.8, 6, 4) - 0.8).abs() < 1e-6);
    }

    #[test]
    fn tracker_enforces_monotonic_progress_and_refreshes_ops() {
        let emitted = Arc::new(Mutex::new(Vec::<SwarmRunStatus>::new()));
        let sink = emitted.clone();
        let ops = Arc::new(AtomicU32::new(2));
        let tracker = SwarmProgressTracker::new(
            sample_status(0.1),
            ops.clone(),
            Arc::new(move |s| sink.lock().unwrap().push(s)),
        );

        tracker.advance(0.45, Some("Building".to_string()));
        ops.store(5, Ordering::Relaxed);
        // Attempting to regress or pass NaN must clamp to 0.45
        tracker.advance(0.2, None);
        tracker.advance(f32::NAN, None);
        tracker.complete_role(Some(AgentRole::Gfx), 0.7, None);
        let final_status = tracker.finish_completed(|s| {
            s.phase_label = "Done".to_string();
        });

        let log = emitted.lock().unwrap();
        assert_eq!(log.len(), 5);
        assert!((log[0].progress - 0.45).abs() < 1e-6);
        assert_eq!(log[0].ops_committed, 2);
        assert!((log[1].progress - 0.45).abs() < 1e-6);
        assert_eq!(log[1].ops_committed, 5);
        assert!((log[2].progress - 0.45).abs() < 1e-6);
        assert!((log[3].progress - 0.7).abs() < 1e-6);
        assert!(!log[3].active_roles.contains(&AgentRole::Gfx));
        assert_eq!(final_status.state, SwarmRunState::Completed);
        assert!((final_status.progress - 1.0).abs() < 1e-6);
        assert!(final_status.active_roles.is_empty());
    }

    #[test]
    fn failed_and_cancelled_preserve_last_reached_progress() {
        let ops = Arc::new(AtomicU32::new(3));
        let tracker_fail =
            SwarmProgressTracker::new(sample_status(0.42), ops.clone(), Arc::new(|_| {}));
        let failed = tracker_fail.finish_failed("boom".to_string());
        assert_eq!(failed.state, SwarmRunState::Failed);
        assert!((failed.progress - 0.42).abs() < 1e-6);
        assert_eq!(failed.ops_committed, 3);

        let tracker_cancel = SwarmProgressTracker::new(sample_status(0.63), ops, Arc::new(|_| {}));
        let cancelled = tracker_cancel.finish_cancelled();
        assert_eq!(cancelled.state, SwarmRunState::Cancelled);
        assert!((cancelled.progress - 0.63).abs() < 1e-6);
    }
}
