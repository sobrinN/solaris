import type { ColorGrade, RgbTriplet } from "@solaris/engine-client";

export interface ColorLookPreset {
  id: string;
  label: string;
  description: string;
  swatchColors: [string, string, string];
  grade: Omit<ColorGrade, "lut_path" | "lut_intensity">;
}

const ZERO_RGB: RgbTriplet = { r: 0, g: 0, b: 0 };

export const COLOR_LOOK_PRESETS: readonly ColorLookPreset[] = [
  {
    id: "natural",
    label: "Natural",
    description: "Neutral reference grade",
    swatchColors: ["#1F1F1F", "#6B6560", "#F7F5EF"],
    grade: {
      lift: { ...ZERO_RGB },
      gamma: { ...ZERO_RGB },
      gain: { ...ZERO_RGB },
      temp: 0,
      tint: 0,
      contrast: 1,
      saturation: 1,
      exposure: 0,
    },
  },
  {
    id: "solar_warm",
    label: "Solar Warm",
    description: "Warm amber highlights with rich obsidian lows",
    swatchColors: ["#1A0E0E", "#F05A00", "#F7B733"],
    grade: {
      lift: { r: -0.02, g: -0.01, b: 0.02 },
      gamma: { r: 0.03, g: 0.01, b: -0.03 },
      gain: { r: 0.06, g: 0.02, b: -0.05 },
      temp: 0.18,
      tint: 0.02,
      contrast: 1.08,
      saturation: 1.12,
      exposure: 0.04,
    },
  },
  {
    id: "obsidian_film",
    label: "Obsidian Film",
    description: "Deep contrast with subtle cool shadows",
    swatchColors: ["#080404", "#38BDF8", "#F7F5EF"],
    grade: {
      lift: { r: -0.04, g: -0.01, b: 0.04 },
      gamma: { r: -0.01, g: 0, b: 0.01 },
      gain: { r: 0.03, g: 0.02, b: 0 },
      temp: -0.08,
      tint: -0.02,
      contrast: 1.18,
      saturation: 0.94,
      exposure: -0.03,
    },
  },
  {
    id: "crisp_broadcast",
    label: "Crisp Studio",
    description: "Clean skin tones and balanced Rec.709 punch",
    swatchColors: ["#1F1F1F", "#60A5FA", "#34D399"],
    grade: {
      lift: { ...ZERO_RGB },
      gamma: { r: 0.01, g: 0.01, b: 0 },
      gain: { r: 0.02, g: 0.02, b: 0.02 },
      temp: 0.04,
      tint: 0,
      contrast: 1.06,
      saturation: 1.06,
      exposure: 0.06,
    },
  },
  {
    id: "golden_hour",
    label: "Golden Hour",
    description: "Luminous sunset warmth with lifted midtones",
    swatchColors: ["#451A03", "#F59E0B", "#FDE68A"],
    grade: {
      lift: { r: 0.02, g: 0, b: -0.02 },
      gamma: { r: 0.05, g: 0.02, b: -0.04 },
      gain: { r: 0.08, g: 0.04, b: -0.06 },
      temp: 0.28,
      tint: 0.04,
      contrast: 1.04,
      saturation: 1.18,
      exposure: 0.08,
    },
  },
  {
    id: "high_contrast",
    label: "Punchy Noir",
    description: "Bold editorial contrast with controlled chroma",
    swatchColors: ["#080404", "#C31818", "#F7F5EF"],
    grade: {
      lift: { r: -0.05, g: -0.05, b: -0.04 },
      gamma: { r: 0, g: -0.01, b: -0.01 },
      gain: { r: 0.06, g: 0.05, b: 0.04 },
      temp: -0.04,
      tint: 0,
      contrast: 1.26,
      saturation: 0.88,
      exposure: 0,
    },
  },
];

export function applyColorLook(
  current: ColorGrade,
  preset: ColorLookPreset,
): ColorGrade {
  return {
    lift: { ...preset.grade.lift },
    gamma: { ...preset.grade.gamma },
    gain: { ...preset.grade.gain },
    temp: preset.grade.temp,
    tint: preset.grade.tint,
    contrast: preset.grade.contrast,
    saturation: preset.grade.saturation,
    exposure: preset.grade.exposure,
    lut_path: current.lut_path ?? null,
    lut_intensity: current.lut_intensity,
  };
}

function approxEq(a: number, b: number): boolean {
  return Math.abs(a - b) < 0.005;
}

export function matchingColorLookId(grade: ColorGrade): string | null {
  for (const preset of COLOR_LOOK_PRESETS) {
    const g = preset.grade;
    if (
      approxEq(grade.temp, g.temp) &&
      approxEq(grade.tint, g.tint) &&
      approxEq(grade.contrast, g.contrast) &&
      approxEq(grade.saturation, g.saturation) &&
      approxEq(grade.exposure, g.exposure) &&
      approxEq(grade.lift.r, g.lift.r) &&
      approxEq(grade.lift.g, g.lift.g) &&
      approxEq(grade.lift.b, g.lift.b) &&
      approxEq(grade.gamma.r, g.gamma.r) &&
      approxEq(grade.gamma.g, g.gamma.g) &&
      approxEq(grade.gamma.b, g.gamma.b) &&
      approxEq(grade.gain.r, g.gain.r) &&
      approxEq(grade.gain.g, g.gain.g) &&
      approxEq(grade.gain.b, g.gain.b)
    ) {
      return preset.id;
    }
  }
  return null;
}

/**
 * Generates a linear-gradient CSS track fill from `neutral` to `value`
 * so NLE sliders visually communicate deviation from their resting point.
 */
export function sliderTrackGradient(
  value: number,
  min: number,
  max: number,
  neutral: number,
  fillHex = "#F05A00",
  baseTrack = "rgba(247, 245, 239, 0.14)",
): string {
  const span = max - min;
  if (!Number.isFinite(span) || span <= 0) {
    return baseTrack;
  }
  const clampPct = (v: number) =>
    Math.max(0, Math.min(100, ((v - min) / span) * 100));
  const vPct = clampPct(value);
  const nPct = clampPct(neutral);
  const lo = Math.min(vPct, nPct).toFixed(2);
  const hi = Math.max(vPct, nPct).toFixed(2);
  return `linear-gradient(to right, ${baseTrack} 0%, ${baseTrack} ${lo}%, ${fillHex} ${lo}%, ${fillHex} ${hi}%, ${baseTrack} ${hi}%, ${baseTrack} 100%)`;
}
