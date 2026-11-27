import assert from "node:assert/strict";
import test from "node:test";
import { resolveSwarmProgress } from "./swarmProgress.ts";

test("resolveSwarmProgress maps queued and finite active progress accurately", () => {
  assert.deepEqual(resolveSwarmProgress({ state: "queued", progress: 0 }), {
    ratio: 0,
    percent: 0,
  });
  assert.deepEqual(resolveSwarmProgress({ state: "executing_specialists", progress: 0.42 }), {
    ratio: 0.42,
    percent: 42,
  });
});

test("resolveSwarmProgress clamps out-of-range values and caps running states at 99%", () => {
  assert.deepEqual(resolveSwarmProgress({ state: "planning", progress: -0.2 }), {
    ratio: 0,
    percent: 0,
  });
  assert.deepEqual(resolveSwarmProgress({ state: "running_qa", progress: 1.4 }), {
    ratio: 0.99,
    percent: 99,
  });
});

test("resolveSwarmProgress always returns 100% for completed runs", () => {
  assert.deepEqual(resolveSwarmProgress({ state: "completed", progress: 0.85 }), {
    ratio: 1,
    percent: 100,
  });
});

test("resolveSwarmProgress falls back by state when progress is non-finite or missing", () => {
  assert.deepEqual(resolveSwarmProgress({ state: "planning", progress: Number.NaN }), {
    ratio: 0.12,
    percent: 12,
  });
  assert.deepEqual(resolveSwarmProgress({ state: "running_qa", progress: Number.POSITIVE_INFINITY }), {
    ratio: 0.85,
    percent: 85,
  });
});

test("resolveSwarmProgress preserves last reached progress for failed and cancelled runs", () => {
  assert.deepEqual(resolveSwarmProgress({ state: "failed", progress: 0.64 }), {
    ratio: 0.64,
    percent: 64,
  });
  assert.deepEqual(resolveSwarmProgress({ state: "cancelled", progress: 0.37 }), {
    ratio: 0.37,
    percent: 37,
  });
});
