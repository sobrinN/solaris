import { test } from "node:test";
import { deepStrictEqual, strictEqual, throws } from "node:assert/strict";
import type { Clip, TimelineCommand, Track } from "@solaris/engine-client";
import {
  buildAlignStartsBatch,
  buildBatchDelete,
  buildBatchSplitAtPlayhead,
  buildCloseGapsBatch,
  buildGroupMoveBatch,
  buildNudgeBatch,
  clampGroupDelta,
  lockedClipCount,
  marqueeHits,
  rangeSelection,
  resolveSelection,
  toggleSelection,
  type SelectedClip,
} from "./timelineSelection.ts";

function clipAt(id: string, startSec: number, durationSec = 2): Clip {
  return {
    id,
    label: id,
    start_sec: startSec,
    duration_sec: durationSec,
    source_in_sec: 0,
    transform: { x: 0, y: 0, scale: 1, rotation_deg: 0, opacity: 1, smart_reframe: null },
    audio: {
      gain_db: 0,
      ducking_enabled: false,
      duck_target_db: -18,
      fade_in_sec: 0,
      fade_out_sec: 0,
    },
    payload: { kind: "media", media_id: "m1" },
  };
}

function selected(trackId: string, clip: Clip, locked = false): SelectedClip {
  return { trackId, locked, clip };
}

function track(id: string, clips: Clip[], locked = false): Track {
  return {
    id,
    code: id.toUpperCase(),
    name: id,
    subtitle: "",
    kind: "a_roll",
    muted: false,
    solo: false,
    locked,
    clips,
  };
}

function toWire(command: TimelineCommand | null): unknown {
  return JSON.parse(JSON.stringify(command));
}

test("toggleSelection adds an absent id at the end and removes a present one", () => {
  deepStrictEqual(toggleSelection(["a", "b"], "c"), ["a", "b", "c"]);
  deepStrictEqual(toggleSelection(["a", "b"], "a"), ["b"]);
  deepStrictEqual(toggleSelection([], "a"), ["a"]);
});

test("rangeSelection returns the timeline span whichever way the drag ran", () => {
  const order = ["a", "b", "c", "d", "e"];
  deepStrictEqual(rangeSelection(order, "b", "d"), ["b", "c", "d"]);
  deepStrictEqual(rangeSelection(order, "d", "b"), ["b", "c", "d"]);
  deepStrictEqual(rangeSelection(order, "c", "c"), ["c"]);
});

test("rangeSelection falls back to the target when the anchor is missing", () => {
  const order = ["a", "b", "c"];
  deepStrictEqual(rangeSelection(order, "gone", "b"), ["b"]);
  deepStrictEqual(rangeSelection(order, "a", "gone"), []);
});

test("marqueeHits counts partial overlaps and ignores edges that only touch", () => {
  const rects = [
    { id: "a", x: 0, y: 0, width: 10, height: 10 },
    { id: "b", x: 20, y: 0, width: 10, height: 10 },
    { id: "c", x: 0, y: 20, width: 10, height: 10 },
  ];
  deepStrictEqual(marqueeHits(rects, { x: 5, y: 5, width: 10, height: 10 }), ["a"]);
  deepStrictEqual(marqueeHits(rects, { x: 10, y: 0, width: 10, height: 10 }), []);
  deepStrictEqual(marqueeHits(rects, { x: 0, y: 0, width: 30, height: 30 }), ["a", "b", "c"]);
});

test("marqueeHits normalizes a box dragged up and to the left", () => {
  const rects = [{ id: "a", x: 0, y: 0, width: 10, height: 10 }];
  deepStrictEqual(marqueeHits(rects, { x: 15, y: 15, width: -10, height: -10 }), ["a"]);
});

test("clampGroupDelta stops the earliest clip at zero", () => {
  const clips = [clipAt("a", 2), clipAt("b", 5)];
  strictEqual(clampGroupDelta(clips, -3), -2);
  strictEqual(clampGroupDelta(clips, -1), -1);
  strictEqual(clampGroupDelta(clips, 4), 4);
  strictEqual(clampGroupDelta([], -3), -3);
});

test("lockedClipCount reports the clips the builders skip", () => {
  strictEqual(
    lockedClipCount([
      selected("v1", clipAt("a", 0)),
      selected("a2", clipAt("m", 1), true),
      selected("a2", clipAt("n", 2), true),
    ]),
    2,
  );
  strictEqual(lockedClipCount([]), 0);
});

test("resolveSelection keeps timeline order, carries lock state, and ignores stale ids", () => {
  const tracks = [
    track("v1", [clipAt("b", 4), clipAt("a", 0)]),
    track("a2", [clipAt("m", 1)], true),
  ];
  const resolved = resolveSelection(tracks, ["m", "b", "a", "gone"]);
  deepStrictEqual(
    resolved.map(({ trackId, locked, clip }) => [trackId, locked, clip.id]),
    [
      ["v1", false, "a"],
      ["v1", false, "b"],
      ["a2", true, "m"],
    ],
  );
});

test("buildGroupMoveBatch shifts every selected clip by the same delta across tracks", () => {
  const selection = [
    selected("v1", clipAt("a", 2)),
    selected("v1", clipAt("b", 6)),
    selected("g1", clipAt("g", 4)),
  ];
  deepStrictEqual(toWire(buildGroupMoveBatch(selection, 1.5)), {
    type: "batch",
    summary: "moved 3 clips by +1.50s",
    commands: [
      { type: "trim_clip", track_id: "v1", clip_id: "a", new_start_sec: 3.5 },
      { type: "trim_clip", track_id: "v1", clip_id: "b", new_start_sec: 7.5 },
      { type: "trim_clip", track_id: "g1", clip_id: "g", new_start_sec: 5.5 },
    ],
  });
});

test("buildGroupMoveBatch clamps the group so no clip starts before zero", () => {
  const selection = [
    selected("v1", clipAt("a", 2)),
    selected("v1", clipAt("b", 6)),
    selected("g1", clipAt("g", 4)),
  ];
  deepStrictEqual(toWire(buildGroupMoveBatch(selection, -5)), {
    type: "batch",
    summary: "moved 3 clips by -2.00s",
    commands: [
      { type: "trim_clip", track_id: "v1", clip_id: "a", new_start_sec: 0 },
      { type: "trim_clip", track_id: "v1", clip_id: "b", new_start_sec: 4 },
      { type: "trim_clip", track_id: "g1", clip_id: "g", new_start_sec: 2 },
    ],
  });
});

test("buildGroupMoveBatch skips clips on locked tracks", () => {
  const selection = [
    selected("v1", clipAt("a", 2)),
    selected("a2", clipAt("m", 0), true),
    selected("g1", clipAt("g", 4)),
  ];
  deepStrictEqual(toWire(buildGroupMoveBatch(selection, 1)), {
    type: "batch",
    summary: "moved 2 clips by +1.00s",
    commands: [
      { type: "trim_clip", track_id: "v1", clip_id: "a", new_start_sec: 3 },
      { type: "trim_clip", track_id: "g1", clip_id: "g", new_start_sec: 5 },
    ],
  });
});

test("buildGroupMoveBatch returns null when nothing applies", () => {
  const lockedOnly = [selected("a2", clipAt("m", 1), true)];
  strictEqual(buildGroupMoveBatch([], 1), null);
  strictEqual(buildGroupMoveBatch(lockedOnly, 1), null);
  strictEqual(buildGroupMoveBatch([selected("v1", clipAt("a", 2))], 0), null);
  strictEqual(buildGroupMoveBatch([selected("v1", clipAt("a", 2))], Number.NaN), null);
  strictEqual(buildGroupMoveBatch([selected("v1", clipAt("a", 0))], -1), null);
  strictEqual(buildNudgeBatch([], 1), null);
});

test("buildNudgeBatch moves by a small step and clamps the group at zero", () => {
  const selection = [selected("v1", clipAt("a", 0.5)), selected("v1", clipAt("b", 3))];
  deepStrictEqual(toWire(buildNudgeBatch(selection, -0.1)), {
    type: "batch",
    summary: "nudged 2 clips by -0.10s",
    commands: [
      { type: "trim_clip", track_id: "v1", clip_id: "a", new_start_sec: 0.4 },
      { type: "trim_clip", track_id: "v1", clip_id: "b", new_start_sec: 2.9 },
    ],
  });
  deepStrictEqual(toWire(buildNudgeBatch(selection, -1)), {
    type: "batch",
    summary: "nudged 2 clips by -0.50s",
    commands: [
      { type: "trim_clip", track_id: "v1", clip_id: "a", new_start_sec: 0 },
      { type: "trim_clip", track_id: "v1", clip_id: "b", new_start_sec: 2.5 },
    ],
  });
});

test("buildBatchDelete emits one delete per unlocked clip", () => {
  const selection = [
    selected("v1", clipAt("a", 2)),
    selected("a2", clipAt("m", 0), true),
    selected("g1", clipAt("g", 4)),
  ];
  deepStrictEqual(toWire(buildBatchDelete(selection)), {
    type: "batch",
    summary: "deleted 2 clips",
    commands: [
      { type: "delete_clip", track_id: "v1", clip_id: "a" },
      { type: "delete_clip", track_id: "g1", clip_id: "g" },
    ],
  });
  deepStrictEqual(toWire(buildBatchDelete([selected("v1", clipAt("a", 2))])), {
    type: "batch",
    summary: "deleted 1 clip",
    commands: [{ type: "delete_clip", track_id: "v1", clip_id: "a" }],
  });
  strictEqual(buildBatchDelete([]), null);
  strictEqual(buildBatchDelete([selected("a2", clipAt("m", 0), true)]), null);
});

test("buildBatchSplitAtPlayhead splits only the clips that contain the playhead", () => {
  const selection = [
    selected("v1", clipAt("a", 0, 4)),
    selected("v1", clipAt("b", 4, 2)),
    selected("g1", clipAt("g", 1, 3)),
    selected("a2", clipAt("m", 0, 4), true),
  ];
  let issued = 0;
  deepStrictEqual(
    toWire(buildBatchSplitAtPlayhead(selection, 2.5, () => `split-${++issued}`)),
    {
      type: "batch",
      summary: "split 2 clips at 2.50s",
      commands: [
        {
          type: "split_clip",
          track_id: "v1",
          clip_id: "a",
          split_at_timeline_sec: 2.5,
          right_clip_id: "split-1",
        },
        {
          type: "split_clip",
          track_id: "g1",
          clip_id: "g",
          split_at_timeline_sec: 2.5,
          right_clip_id: "split-2",
        },
      ],
    },
  );
  strictEqual(issued, 2);
});

test("buildBatchSplitAtPlayhead leaves clips alone when the playhead is on an edge or outside", () => {
  const selection = [selected("v1", clipAt("a", 0, 4))];
  const newId = () => "right-1";
  strictEqual(buildBatchSplitAtPlayhead([], 2, newId), null);
  strictEqual(buildBatchSplitAtPlayhead(selection, 0.01, newId), null);
  strictEqual(buildBatchSplitAtPlayhead(selection, 3.99, newId), null);
  strictEqual(buildBatchSplitAtPlayhead(selection, 4.5, newId), null);
  strictEqual(buildBatchSplitAtPlayhead(selection, Number.NaN, newId), null);
});

test("buildBatchSplitAtPlayhead accepts 64-character ids and rejects ids outside the safe-id rule", () => {
  const selection = [selected("v1", clipAt("a", 0, 4))];
  deepStrictEqual(toWire(buildBatchSplitAtPlayhead(selection, 2, () => "x".repeat(64))), {
    type: "batch",
    summary: "split 1 clip at 2.00s",
    commands: [
      {
        type: "split_clip",
        track_id: "v1",
        clip_id: "a",
        split_at_timeline_sec: 2,
        right_clip_id: "x".repeat(64),
      },
    ],
  });
  throws(() => buildBatchSplitAtPlayhead(selection, 2, () => "not safe!"), RangeError);
  throws(() => buildBatchSplitAtPlayhead(selection, 2, () => ""), RangeError);
  throws(() => buildBatchSplitAtPlayhead(selection, 2, () => "x".repeat(65)), RangeError);
});

test("buildAlignStartsBatch moves unlocked clips to the earliest selected start", () => {
  const selection = [
    selected("v1", clipAt("a", 3)),
    selected("g1", clipAt("g", 1)),
    selected("a2", clipAt("m", 5), true),
  ];
  deepStrictEqual(toWire(buildAlignStartsBatch(selection)), {
    type: "batch",
    summary: "aligned 1 clip to 1.00s",
    commands: [{ type: "trim_clip", track_id: "v1", clip_id: "a", new_start_sec: 1 }],
  });
});

test("buildAlignStartsBatch uses a locked clip as the target but never moves it", () => {
  const selection = [selected("a2", clipAt("m", 0.5), true), selected("v1", clipAt("a", 2))];
  deepStrictEqual(toWire(buildAlignStartsBatch(selection)), {
    type: "batch",
    summary: "aligned 1 clip to 0.50s",
    commands: [{ type: "trim_clip", track_id: "v1", clip_id: "a", new_start_sec: 0.5 }],
  });
});

test("buildAlignStartsBatch returns null when the selection already lines up", () => {
  strictEqual(buildAlignStartsBatch([]), null);
  strictEqual(
    buildAlignStartsBatch([selected("v1", clipAt("a", 2)), selected("g1", clipAt("g", 2))]),
    null,
  );
});

test("buildCloseGapsBatch packs each track's selected clips in order", () => {
  const selection = [
    selected("v1", clipAt("a", 0, 2)),
    selected("v1", clipAt("b", 5, 1)),
    selected("v1", clipAt("c", 9, 3)),
    selected("g1", clipAt("x", 12, 1)),
    selected("g1", clipAt("y", 10, 1)),
  ];
  deepStrictEqual(toWire(buildCloseGapsBatch(selection)), {
    type: "batch",
    summary: "closed gaps by moving 3 clips",
    commands: [
      { type: "trim_clip", track_id: "v1", clip_id: "b", new_start_sec: 2 },
      { type: "trim_clip", track_id: "v1", clip_id: "c", new_start_sec: 3 },
      { type: "trim_clip", track_id: "g1", clip_id: "x", new_start_sec: 11 },
    ],
  });
});

test("buildCloseGapsBatch pushes an overlapping clip after its predecessor", () => {
  deepStrictEqual(
    toWire(
      buildCloseGapsBatch([selected("v1", clipAt("a", 0, 4)), selected("v1", clipAt("b", 2, 1))]),
    ),
    {
      type: "batch",
      summary: "closed gaps by moving 1 clip",
      commands: [{ type: "trim_clip", track_id: "v1", clip_id: "b", new_start_sec: 4 }],
    },
  );
});

test("buildCloseGapsBatch returns null for locked tracks and already packed selections", () => {
  strictEqual(buildCloseGapsBatch([]), null);
  strictEqual(
    buildCloseGapsBatch([
      selected("a2", clipAt("p", 0, 1), true),
      selected("a2", clipAt("q", 5, 1), true),
    ]),
    null,
  );
  strictEqual(
    buildCloseGapsBatch([selected("v1", clipAt("a", 0, 2)), selected("v1", clipAt("b", 2, 1))]),
    null,
  );
});
