import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { computeStudioGridLayout } from "./studioLayout.ts";

describe("studioLayout", () => {
  it("returns null below 1024px", () => {
    assert.equal(
      computeStudioGridLayout({
        width: 1023,
        height: 560,
        canvasAspect: 9 / 16,
        scopesOpen: true,
        viewerMode: "program",
      }),
      null,
    );
  });

  it("sums columns + gaps to exact container width across aspects and viewports", () => {
    const widths = [1024, 1280, 1440, 1920];
    const aspects = [9 / 16, 1, 16 / 9];
    for (const width of widths) {
      for (const canvasAspect of aspects) {
        for (const scopesOpen of [true, false]) {
          for (const viewerMode of ["program", "source"] as const) {
            const layout = computeStudioGridLayout({
              width,
              height: 580,
              canvasAspect,
              scopesOpen,
              viewerMode,
            });
            assert.ok(layout);
            const showScopes = width >= 1280 && scopesOpen;
            const gaps = showScopes ? 24 : 16;
            assert.equal(
              layout.directorPx +
                layout.programPx +
                layout.scopesPx +
                layout.inspectorPx +
                gaps,
              width,
            );
            assert.ok(layout.scopesPx <= 216);
          }
        }
      }
    }
  });

  it("scales program width monotonically 9:16 < 1:1 < 16:9", () => {
    for (const width of [1440, 1920]) {
      const portrait = computeStudioGridLayout({
        width,
        height: 580,
        canvasAspect: 9 / 16,
        scopesOpen: true,
        viewerMode: "program",
      })!;
      const square = computeStudioGridLayout({
        width,
        height: 580,
        canvasAspect: 1,
        scopesOpen: true,
        viewerMode: "program",
      })!;
      const landscape = computeStudioGridLayout({
        width,
        height: 580,
        canvasAspect: 16 / 9,
        scopesOpen: true,
        viewerMode: "program",
      })!;
      assert.ok(portrait.programPx < square.programPx);
      assert.ok(square.programPx < landscape.programPx);
    }
  });

  it("centers the Program monitor horizontally on 1920px when side minimums allow", () => {
    for (const [canvasAspect, height] of [
      [9 / 16, 600],
      [1, 600],
      [16 / 9, 460],
    ] as const) {
      const layout = computeStudioGridLayout({
        width: 1920,
        height,
        canvasAspect,
        scopesOpen: true,
        viewerMode: "program",
      })!;
      const leftWidth = layout.directorPx;
      const rightWidth = layout.scopesPx + 8 + layout.inspectorPx;
      assert.ok(
        Math.abs(leftWidth - rightWidth) <= 2,
        `aspect ${canvasAspect}: left=${leftWidth}, right=${rightWidth}`,
      );
    }
  });
});
