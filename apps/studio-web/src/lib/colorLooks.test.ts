import assert from "node:assert/strict";
import { describe, it } from "node:test";
import type { ColorGrade } from "@solaris/engine-client";
import {
  applyColorLook,
  COLOR_LOOK_PRESETS,
  matchingColorLookId,
  sliderTrackGradient,
} from "./colorLooks.ts";

const BASE_GRADE: ColorGrade = {
  lift: { r: 0.1, g: 0, b: -0.05 },
  gamma: { r: 0, g: 0.02, b: 0 },
  gain: { r: 0, g: 0, b: 0 },
  temp: 0.25,
  tint: -0.1,
  contrast: 1.15,
  saturation: 1.2,
  exposure: 0.1,
  lut_path: "brands/demo/luts/film.cube",
  lut_intensity: 0.65,
};

describe("colorLooks", () => {
  it("applies preset while preserving lut_path and lut_intensity", () => {
    const solar = COLOR_LOOK_PRESETS.find((p) => p.id === "solar_warm");
    assert.ok(solar);
    const next = applyColorLook(BASE_GRADE, solar);
    assert.equal(next.lut_path, "brands/demo/luts/film.cube");
    assert.equal(next.lut_intensity, 0.65);
    assert.equal(next.temp, solar.grade.temp);
    assert.equal(next.contrast, solar.grade.contrast);
    assert.equal(matchingColorLookId(next), "solar_warm");
  });

  it("returns null when grade does not match any preset", () => {
    assert.equal(matchingColorLookId(BASE_GRADE), null);
  });

  it("renders centered bipolar gradient from neutral 0 to positive/negative values", () => {
    const pos = sliderTrackGradient(0.5, -1, 1, 0, "#F05A00");
    assert.ok(pos.includes("50.00%"));
    assert.ok(pos.includes("75.00%"));

    const neg = sliderTrackGradient(-0.5, -1, 1, 0, "#F05A00");
    assert.ok(neg.includes("25.00%"));
    assert.ok(neg.includes("50.00%"));
  });

  it("clamps out-of-range values and handles off-center neutral", () => {
    const contrast = sliderTrackGradient(2.5, 0.5, 1.6, 1, "#F05A00");
    assert.ok(contrast.includes("100.00%"));
    assert.ok(contrast.includes("45.45%"));
  });

  it("centers contrast neutral 1.00 at 50.00% on symmetric [0.4, 1.6] range", () => {
    const neutral = sliderTrackGradient(1, 0.4, 1.6, 1, "#F05A00");
    assert.ok(neutral.includes("50.00%"));

    const clampedMax = sliderTrackGradient(2.5, 0.4, 1.6, 1, "#F05A00");
    assert.ok(clampedMax.includes("50.00%"));
    assert.ok(clampedMax.includes("100.00%"));
  });
});
