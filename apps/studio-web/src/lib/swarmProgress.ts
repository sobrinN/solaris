import type { SwarmRunState, SwarmRunStatus } from "@solaris/engine-client";

export interface SwarmProgressDisplay {
  ratio: number;
  percent: number;
}

const RUNNING_STATES: ReadonlySet<SwarmRunState> = new Set([
  "queued",
  "planning",
  "executing_spine",
  "executing_specialists",
  "running_qa",
]);

const STATE_FALLBACK_PROGRESS: Record<SwarmRunState, number> = {
  queued: 0,
  planning: 0.12,
  executing_spine: 0.3,
  executing_specialists: 0.55,
  running_qa: 0.85,
  completed: 1,
  failed: 0,
  cancelled: 0,
};

/**
 * Resolve normalized `[0, 1]` ratio and integer `[0, 100]` percentage for
 * the Director live swarm loading bar.
 */
export function resolveSwarmProgress(
  run: Pick<SwarmRunStatus, "state" | "progress">,
): SwarmProgressDisplay {
  if (run.state === "completed") {
    return { ratio: 1, percent: 100 };
  }

  const raw =
    typeof run.progress === "number" && Number.isFinite(run.progress)
      ? Math.max(0, Math.min(1, run.progress))
      : STATE_FALLBACK_PROGRESS[run.state];

  if (RUNNING_STATES.has(run.state)) {
    const ratio = Math.min(0.99, raw);
    const percent = Math.min(99, Math.round(ratio * 100));
    return { ratio, percent };
  }

  return {
    ratio: raw,
    percent: Math.round(raw * 100),
  };
}
