import assert from "node:assert/strict";
import { describe, it } from "node:test";
import {
  STARTER_GFX_IDS,
  checkGfxName,
  clampToViewport,
  duplicateGfxName,
  gfxPreviewGeometry,
  missingStarterGfxIds,
  nextDuplicateId,
  type GfxPreviewAspect,
} from "./gfxAssetActions.ts";

function near(actual: number, expected: number): void {
  assert.ok(Math.abs(actual - expected) < 1e-9, `expected ${actual} to be ${expected}`);
}

describe("missingStarterGfxIds", () => {
  it("returns nothing when every starter is present", () => {
    assert.deepEqual(
      missingStarterGfxIds(STARTER_GFX_IDS.map((id) => ({ id }))),
      [],
    );
  });

  it("lists the missing starters in catalog order and ignores custom ids", () => {
    const present = STARTER_GFX_IDS.filter(
      (id) => id !== "stat-card-solar" && id !== "progress-bar-solar",
    ).map((id) => ({ id }));
    assert.deepEqual(missingStarterGfxIds([...present, { id: "custom-1" }]), [
      "stat-card-solar",
      "progress-bar-solar",
    ]);
  });
});

describe("nextDuplicateId", () => {
  it("uses the -copy suffix when it is free", () => {
    assert.equal(nextDuplicateId("stat-card-solar", []), "stat-card-solar-copy");
  });

  it("moves on to -copy-2 and -copy-3 while earlier copies are taken", () => {
    assert.equal(
      nextDuplicateId("stat-card-solar", ["stat-card-solar-copy"]),
      "stat-card-solar-copy-2",
    );
    assert.equal(
      nextDuplicateId("stat-card-solar", ["stat-card-solar-copy", "stat-card-solar-copy-2"]),
      "stat-card-solar-copy-3",
    );
  });

  it("keeps the id within the 64-character server limit", () => {
    const longId = "a".repeat(64);
    const first = nextDuplicateId(longId, []);
    assert.equal(first.length, 64);
    assert.ok(first.endsWith("-copy"));
    const second = nextDuplicateId(longId, [first]);
    assert.equal(second.length, 64);
    assert.ok(second.endsWith("-copy-2"));
  });
});

describe("duplicateGfxName", () => {
  it("appends (copy) to the name", () => {
    assert.equal(duplicateGfxName("Stat Card"), "Stat Card (copy)");
  });

  it("keeps the copy name within 80 characters", () => {
    const copy = duplicateGfxName("x".repeat(80));
    assert.equal(copy.length, 80);
    assert.ok(copy.endsWith(" (copy)"));
  });
});

describe("checkGfxName", () => {
  it("trims the name", () => {
    assert.deepEqual(checkGfxName("  Hook  "), { ok: true, name: "Hook" });
  });

  it("rejects a blank name", () => {
    assert.deepEqual(checkGfxName("   "), { ok: false, message: "Enter a name." });
  });

  it("accepts 80 characters and rejects 81", () => {
    assert.equal(checkGfxName("x".repeat(80)).ok, true);
    assert.equal(checkGfxName("x".repeat(81)).ok, false);
  });
});

describe("clampToViewport", () => {
  it("keeps a menu inside the right and bottom edges", () => {
    assert.deepEqual(
      clampToViewport({ x: 900, y: 600 }, { width: 180, height: 200 }, { width: 1000, height: 700 }),
      { x: 812, y: 492 },
    );
  });

  it("keeps a menu inside the top and left edges", () => {
    assert.deepEqual(
      clampToViewport({ x: -20, y: -5 }, { width: 180, height: 200 }, { width: 1000, height: 700 }),
      { x: 8, y: 8 },
    );
  });
});

describe("gfxPreviewGeometry", () => {
  const aspects: GfxPreviewAspect[] = ["16:9", "9:16", "1:1"];

  for (const aspect of aspects) {
    it(`fits the ${aspect} canvas into the 64px preview height`, () => {
      const geometry = gfxPreviewGeometry(aspect);
      near(geometry.boxHeightPx, 64);
      near(geometry.scale * geometry.viewportHeightPx, 64);
      near(geometry.scale * geometry.viewportWidthPx, geometry.boxWidthPx);
      near(
        geometry.boxWidthPx / geometry.boxHeightPx,
        geometry.viewportWidthPx / geometry.viewportHeightPx,
      );
    });
  }

  it("uses the canonical virtual viewports", () => {
    assert.deepEqual(
      [gfxPreviewGeometry("16:9"), gfxPreviewGeometry("9:16"), gfxPreviewGeometry("1:1")].map(
        (g) => [g.viewportWidthPx, g.viewportHeightPx],
      ),
      [
        [960, 540],
        [540, 960],
        [720, 720],
      ],
    );
  });
});
