import assert from "node:assert/strict";
import { describe, it } from "node:test";
import {
  computeRulerTicks,
  formatCompactDuration,
  snapTimeSec,
} from "./timelineFormat.ts";

describe("timelineFormat", () => {
  it("never produces out-of-bounds percentages and marks edge ticks", () => {
    const { majorStep, ticks } = computeRulerTicks(20, 1);
    assert.equal(majorStep, 2);
    assert.ok(ticks.length > 0);
    assert.equal(ticks[0].edge, "start");
    assert.equal(ticks[ticks.length - 1].edge, "end");
    for (const tick of ticks) {
      assert.ok(tick.pct >= 0 && tick.pct <= 100);
    }
  });

  it("increases tick resolution when zoomed in", () => {
    const normal = computeRulerTicks(24, 1);
    const zoomed = computeRulerTicks(24, 4);
    assert.ok(zoomed.majorStep <= normal.majorStep);
    assert.ok(zoomed.ticks.length >= normal.ticks.length);
  });

  it("formats compact durations cleanly", () => {
    assert.equal(formatCompactDuration(4), "4.0s");
    assert.equal(formatCompactDuration(12.34), "12.3s");
    assert.equal(formatCompactDuration(64), "1m 04s");
  });

  it("snaps to grid when enabled and preserves precision when disabled", () => {
    assert.equal(snapTimeSec(1.24, true, 0.1), 1.2);
    assert.equal(snapTimeSec(1.26, true, 0.1), 1.3);
    assert.equal(snapTimeSec(-3, true, 0.1), 0);
    assert.equal(snapTimeSec(1.246, false), 1.246);
  });
});
