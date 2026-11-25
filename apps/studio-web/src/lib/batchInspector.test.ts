import { test } from "node:test";
import { deepStrictEqual, strictEqual } from "node:assert/strict";
import type { Clip, TimelineCommand } from "@solaris/engine-client";
import {
  buildAudioFieldBatch,
  buildCaptionStyleBatch,
  buildTransformFieldBatch,
  mixedValue,
  summarizeSelection,
} from "./batchInspector.ts";
import type { SelectedClip } from "./timelineSelection.ts";

function clipWith(id: string, extra: Partial<Clip> = {}): Clip {
  return {
    id,
    label: id,
    start_sec: 0,
    duration_sec: 2,
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
    ...extra,
  };
}

function selected(trackId: string, clip: Clip, locked = false): SelectedClip {
  return { trackId, locked, clip };
}

function toWire(command: TimelineCommand | null): unknown {
  return JSON.parse(JSON.stringify(command));
}

const captionWords = [
  { text: "hello", start_sec: 0, end_sec: 0.5 },
  { text: "world", start_sec: 0.5, end_sec: 1 },
];

function captionClip(id: string): Clip {
  return clipWith(id, {
    payload: {
      kind: "caption",
      caption: {
        words: captionWords,
        preset: "neon_glow",
        font_family: "Sora",
        highlight_color: "#F7B733",
      },
    },
  });
}

test("summarizeSelection counts clips and tracks and sums the durations", () => {
  const selection = [
    selected("v1", clipWith("a", { duration_sec: 2 })),
    selected("v1", clipWith("b", { duration_sec: 3.5 })),
    selected("g1", clipWith("g", { duration_sec: 1.25 })),
    selected("a2", clipWith("m", { duration_sec: 4 }), true),
  ];
  deepStrictEqual(summarizeSelection(selection), {
    clipCount: 4,
    trackCount: 3,
    durationSec: 10.75,
  });
  deepStrictEqual(summarizeSelection([]), { clipCount: 0, trackCount: 0, durationSec: 0 });
});

test("mixedValue reports a shared value, a mix, an empty list, and a single value", () => {
  deepStrictEqual(mixedValue([0.5, 0.5, 0.5]), { mixed: false, value: 0.5 });
  deepStrictEqual(mixedValue([1, 2]), { mixed: true, value: null });
  deepStrictEqual(mixedValue(["minimal", "karaoke"]), { mixed: true, value: null });
  deepStrictEqual(mixedValue([]), { mixed: false, value: null });
  deepStrictEqual(mixedValue([7]), { mixed: false, value: 7 });
});

test("buildTransformFieldBatch changes only the edited field and keeps the rest of each transform", () => {
  const box = { x: 0.1, y: 0.2, width: 0.5, height: 0.6 };
  const selection = [
    selected(
      "v1",
      clipWith("a", {
        transform: { x: 0, y: 0, scale: 1, rotation_deg: 0, opacity: 1, smart_reframe: box },
      }),
    ),
    selected(
      "g1",
      clipWith("g", {
        transform: { x: 10, y: 4, scale: 2, rotation_deg: 15, opacity: 0.5, smart_reframe: null },
      }),
    ),
    selected("a2", clipWith("m"), true),
  ];
  deepStrictEqual(toWire(buildTransformFieldBatch(selection, "x", 5)), {
    type: "batch",
    summary: "set x on 2 clips",
    commands: [
      {
        type: "update_clip_transform",
        track_id: "v1",
        clip_id: "a",
        transform: { x: 5, y: 0, scale: 1, rotation_deg: 0, opacity: 1, smart_reframe: box },
      },
      {
        type: "update_clip_transform",
        track_id: "g1",
        clip_id: "g",
        transform: { x: 5, y: 4, scale: 2, rotation_deg: 15, opacity: 0.5, smart_reframe: null },
      },
    ],
  });
});

test("buildTransformFieldBatch skips clips already at the value and refuses invalid values", () => {
  const selection = [
    selected("v1", clipWith("a")),
    selected(
      "g1",
      clipWith("g", {
        transform: { x: 0, y: 0, scale: 1, rotation_deg: 0, opacity: 0.5, smart_reframe: null },
      }),
    ),
  ];
  deepStrictEqual(toWire(buildTransformFieldBatch(selection, "opacity", 0.5)), {
    type: "batch",
    summary: "set opacity on 1 clip",
    commands: [
      {
        type: "update_clip_transform",
        track_id: "v1",
        clip_id: "a",
        transform: { x: 0, y: 0, scale: 1, rotation_deg: 0, opacity: 0.5, smart_reframe: null },
      },
    ],
  });
  strictEqual(buildTransformFieldBatch(selection, "x", 0), null);
  strictEqual(buildTransformFieldBatch(selection, "scale", 0), null);
  strictEqual(buildTransformFieldBatch(selection, "scale", -1), null);
  strictEqual(buildTransformFieldBatch(selection, "opacity", 1.5), null);
  strictEqual(buildTransformFieldBatch(selection, "y", Number.NaN), null);
});

test("buildAudioFieldBatch edits only media clips that are not locked", () => {
  const selection = [
    selected(
      "v1",
      clipWith("a", {
        audio: {
          gain_db: 0,
          ducking_enabled: true,
          duck_target_db: -18,
          fade_in_sec: 0.1,
          fade_out_sec: 0.5,
        },
      }),
    ),
    selected(
      "g1",
      clipWith("g", { payload: { kind: "gfx", gfx: { props: {} } } }),
    ),
    selected("a2", clipWith("m"), true),
    selected("a2", clipWith("n")),
  ];
  deepStrictEqual(toWire(buildAudioFieldBatch(selection, "gain_db", -3)), {
    type: "batch",
    summary: "set gain_db on 2 clips",
    commands: [
      {
        type: "set_audio_settings",
        track_id: "v1",
        clip_id: "a",
        audio: {
          gain_db: -3,
          ducking_enabled: true,
          duck_target_db: -18,
          fade_in_sec: 0.1,
          fade_out_sec: 0.5,
        },
      },
      {
        type: "set_audio_settings",
        track_id: "a2",
        clip_id: "n",
        audio: {
          gain_db: -3,
          ducking_enabled: false,
          duck_target_db: -18,
          fade_in_sec: 0,
          fade_out_sec: 0,
        },
      },
    ],
  });
});

test("buildAudioFieldBatch refuses negative fades, non-finite gain, and caption-only selections", () => {
  const selection = [selected("v1", clipWith("a"))];
  strictEqual(buildAudioFieldBatch(selection, "fade_in_sec", -0.5), null);
  strictEqual(buildAudioFieldBatch(selection, "fade_out_sec", -0.5), null);
  strictEqual(buildAudioFieldBatch(selection, "gain_db", Number.NaN), null);
  strictEqual(buildAudioFieldBatch(selection, "gain_db", 0), null);
  strictEqual(buildAudioFieldBatch([selected("t1", captionClip("c1"))], "gain_db", -3), null);
});

test("buildCaptionStyleBatch changes only the preset and keeps the words", () => {
  const selection = [
    selected("t1", captionClip("c1")),
    selected("v1", clipWith("media")),
    selected("t1", captionClip("c2"), true),
  ];
  deepStrictEqual(toWire(buildCaptionStyleBatch(selection, { preset: "karaoke" })), {
    type: "batch",
    summary: "set caption style on 1 caption",
    commands: [
      {
        type: "update_caption_clip",
        track_id: "t1",
        clip_id: "c1",
        caption: {
          words: captionWords,
          preset: "karaoke",
          font_family: "Sora",
          highlight_color: "#F7B733",
        },
      },
    ],
  });
});

test("buildCaptionStyleBatch trims the highlight color and refuses an empty one", () => {
  const selection = [selected("t1", captionClip("c1")), selected("t1", captionClip("c2"))];
  deepStrictEqual(toWire(buildCaptionStyleBatch(selection, { highlight_color: " #F05A00 " })), {
    type: "batch",
    summary: "set caption style on 2 captions",
    commands: [
      {
        type: "update_caption_clip",
        track_id: "t1",
        clip_id: "c1",
        caption: {
          words: captionWords,
          preset: "neon_glow",
          font_family: "Sora",
          highlight_color: "#F05A00",
        },
      },
      {
        type: "update_caption_clip",
        track_id: "t1",
        clip_id: "c2",
        caption: {
          words: captionWords,
          preset: "neon_glow",
          font_family: "Sora",
          highlight_color: "#F05A00",
        },
      },
    ],
  });
  strictEqual(buildCaptionStyleBatch(selection, { highlight_color: "   " }), null);
});

test("every field builder returns null for an empty selection", () => {
  strictEqual(buildTransformFieldBatch([], "x", 5), null);
  strictEqual(buildAudioFieldBatch([], "gain_db", -3), null);
  strictEqual(buildCaptionStyleBatch([], { preset: "karaoke" }), null);
});

test("buildCaptionStyleBatch returns null when the style already matches", () => {
  const selection = [selected("t1", captionClip("c1"))];
  strictEqual(buildCaptionStyleBatch(selection, { preset: "neon_glow" }), null);
  strictEqual(buildCaptionStyleBatch(selection, { highlight_color: "#F7B733" }), null);
  strictEqual(buildCaptionStyleBatch(selection, {}), null);
  const mediaOnly = [selected("v1", clipWith("media"))];
  strictEqual(buildCaptionStyleBatch(mediaOnly, { preset: "minimal" }), null);
});
