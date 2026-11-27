import assert from "node:assert/strict";
import { describe, it } from "node:test";
import type { MediaAsset } from "@solaris/engine-client";
import {
  canvasConfigForPreset,
  inferAspectFromDimensions,
  inferCanvasFromVideo,
  snapStandardFps,
} from "./canvasMatch.ts";

function makeMedia(overrides: Partial<MediaAsset> = {}): MediaAsset {
  return {
    id: "m1",
    name: "clip.mp4",
    kind: "video",
    role_hint: "a_roll",
    path: "/tmp/clip.mp4",
    added_at: "2026-11-27T00:00:00Z",
    intelligence: {
      analyzed: true,
      duration_sec: 12,
      has_audio: true,
      width: 1920,
      height: 1080,
      fps: 29.97,
      is_log_footage: false,
      waveform_peaks: [],
      beats: [],
      scene_cuts: [],
      silences: [],
      transcript: [],
    },
    ...overrides,
  };
}

describe("canvasMatch", () => {
  it("snaps standard cinema and broadcast frame rates", () => {
    assert.equal(snapStandardFps(23.976), 24);
    assert.equal(snapStandardFps(29.97), 30);
    assert.equal(snapStandardFps(59.94), 60);
    assert.equal(snapStandardFps(null), 30);
    assert.equal(snapStandardFps(0), 30);
    assert.equal(snapStandardFps(Number.NaN), 30);
  });

  it("infers aspect preset from dimensions", () => {
    assert.equal(inferAspectFromDimensions(1920, 1080), "landscape16x9");
    assert.equal(inferAspectFromDimensions(1080, 1920), "vertical9x16");
    assert.equal(inferAspectFromDimensions(1080, 1080), "square1x1");
  });

  it("builds canvas config for each preset", () => {
    assert.deepEqual(canvasConfigForPreset("landscape16x9", 24), {
      width: 1920,
      height: 1080,
      fps_num: 24,
      fps_den: 1,
      aspect_preset: "landscape16x9",
    });
    assert.deepEqual(canvasConfigForPreset("vertical9x16", 60), {
      width: 1080,
      height: 1920,
      fps_num: 60,
      fps_den: 1,
      aspect_preset: "vertical9x16",
    });
    assert.deepEqual(canvasConfigForPreset("square1x1"), {
      width: 1080,
      height: 1080,
      fps_num: 30,
      fps_den: 1,
      aspect_preset: "square1x1",
    });
  });

  it("infers canvas from analyzed video and returns null for invalid/unanalyzed assets", () => {
    assert.deepEqual(inferCanvasFromVideo(makeMedia()), {
      width: 1920,
      height: 1080,
      fps_num: 30,
      fps_den: 1,
      aspect_preset: "landscape16x9",
    });

    assert.equal(
      inferCanvasFromVideo(
        makeMedia({
          kind: "image",
        }),
      ),
      null,
    );

    const base = makeMedia();
    assert.equal(
      inferCanvasFromVideo(
        makeMedia({
          intelligence: { ...base.intelligence, analyzed: false },
        }),
      ),
      null,
    );

    assert.equal(
      inferCanvasFromVideo(
        makeMedia({
          intelligence: { ...base.intelligence, width: 8, height: 8 },
        }),
      ),
      null,
    );
  });
});
