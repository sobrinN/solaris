import { test } from "node:test";
import { deepStrictEqual, ok, strictEqual } from "node:assert/strict";
import { REFINEMENT_CHIPS } from "./refinementChips.ts";

test("offers the seven refinement chips in order", () => {
  deepStrictEqual(
    REFINEMENT_CHIPS.map((chip) => chip.label),
    [
      "Fix QA issues",
      "Tighten pacing",
      "Brighter grade",
      "Move GFX clear of captions",
      "Shorter captions",
      "Remove silent gaps",
      "Punchier captions",
    ],
  );
});

test("every chip has a unique kebab-case id, only id/label/prompt keys, and a non-empty prompt", () => {
  const ids = REFINEMENT_CHIPS.map((chip) => chip.id);
  strictEqual(new Set(ids).size, ids.length);
  for (const chip of REFINEMENT_CHIPS) {
    ok(/^[a-z0-9]+(-[a-z0-9]+)*$/.test(chip.id), chip.id);
    deepStrictEqual(Object.keys(chip).sort(), ["id", "label", "prompt"]);
    ok(chip.prompt.trim().length > 0, chip.id);
  }
});
