import assert from "node:assert/strict";
import { describe, it } from "node:test";
import type { ColorGrade } from "@solaris/engine-client";
import {
  applyColorLook,
  COLOR_LOOK_PRESETS,
  composeEffectiveGrade,
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
    assert.equal(next.brightness, 0);
    assert.equal(matchingColorLookId(next), "solar_warm");
    assert.equal(matchingColorLookId({ ...next, brightness: 0.15 }), null);
  });

  it("composes FX adjustment-layer grades onto base grade with LUT override rules", () => {
    assert.equal(composeEffectiveGrade(BASE_GRADE, null), BASE_GRADE);

    const fx: ColorGrade = {
      lift: { r: 0.05, g: -0.02, b: 0.01 },
      gamma: { r: -0.01, g: 0.03, b: 0.02 },
      gain: { r: 0.04, g: 0.01, b: -0.03 },
      temp: 0.1,
      tint: 0.05,
      contrast: 1.2,
      saturation: 0.8,
      exposure: 0.25,
      brightness: 0.1,
      lut_path: null,
      lut_intensity: 0.5,
    };

    const stacked = composeEffectiveGrade(BASE_GRADE, fx);
    assert.ok(Math.abs(stacked.lift.r - 0.15) < 1e-9);
    assert.ok(Math.abs(stacked.gamma.g - 0.05) < 1e-9);
    assert.ok(Math.abs(stacked.gain.b - -0.03) < 1e-9);
    assert.ok(Math.abs(stacked.temp - 0.35) < 1e-9);
    assert.ok(Math.abs(stacked.tint - -0.05) < 1e-9);
    assert.ok(Math.abs(stacked.contrast - 1.38) < 1e-9);
    assert.ok(Math.abs(stacked.saturation - 0.96) < 1e-9);
    assert.ok(Math.abs(stacked.exposure - 0.35) < 1e-9);
    assert.ok(Math.abs((stacked.brightness ?? 0) - 0.1) < 1e-9);
    assert.equal(stacked.lut_path, "brands/demo/luts/film.cube");
    assert.equal(stacked.lut_intensity, 0.65);

    const withFxLut = composeEffectiveGrade(BASE_GRADE, {
      ...fx,
      lut_path: "brands/demo/luts/custom.cube",
      lut_intensity: 0.42,
    });
    assert.equal(withFxLut.lut_path, "brands/demo/luts/custom.cube");
    assert.equal(withFxLut.lut_intensity, 0.42);
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
