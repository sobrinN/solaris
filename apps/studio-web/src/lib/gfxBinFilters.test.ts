import assert from "node:assert/strict";
import { describe, it } from "node:test";
import type { SavedCodeAsset } from "@solaris/engine-client";
import {
  ALL_GFX_CATEGORIES,
  filterGfxAssets,
  gfxCategories,
  resolveGfxCategory,
} from "./gfxBinFilters.ts";

function asset(id: string, name: string, category: string, tags: string[]): SavedCodeAsset {
  return {
    id,
    name,
    category,
    tags,
    props_schema: {},
    default_props: {},
    html_source: '<div class="clip"></div>',
    default_duration_sec: 2.5,
    created_at: "2026-01-01T00:00:00Z",
  };
}

const LIBRARY = [
  asset("chapter-title-solar", "Chapter Title", "titles", ["title", "hook", "chapter"]),
  asset("stat-card-solar", "Stat Card", "cards", ["stat", "metric", "callout"]),
  asset("custom-171", "Lower Third Blue", "titles", ["brand"]),
];

function idsFor(query: string, category = ALL_GFX_CATEGORIES): string[] {
  return filterGfxAssets(LIBRARY, { query, category }).map((item) => item.id);
}

describe("filterGfxAssets", () => {
  it("matches the asset name", () => {
    assert.deepEqual(idsFor("third"), ["custom-171"]);
  });

  it("matches the asset id", () => {
    assert.deepEqual(idsFor("custom"), ["custom-171"]);
  });

  it("matches a tag", () => {
    assert.deepEqual(idsFor("metric"), ["stat-card-solar"]);
  });

  it("ignores letter case in the query and the asset fields", () => {
    assert.deepEqual(idsFor("CHAPTER"), ["chapter-title-solar"]);
    assert.deepEqual(idsFor("MeTrIc"), ["stat-card-solar"]);
  });

  it("returns every asset for an empty or whitespace-only query", () => {
    assert.deepEqual(idsFor(""), ["chapter-title-solar", "stat-card-solar", "custom-171"]);
    assert.deepEqual(idsFor("   "), ["chapter-title-solar", "stat-card-solar", "custom-171"]);
  });

  it("returns an empty list when nothing matches", () => {
    assert.deepEqual(idsFor("zzz"), []);
  });

  it("requires every query term to match some field", () => {
    assert.deepEqual(idsFor("stat metric"), ["stat-card-solar"]);
    assert.deepEqual(idsFor("stat hook"), []);
  });

  it("keeps only the selected category", () => {
    assert.deepEqual(idsFor("", "titles"), ["chapter-title-solar", "custom-171"]);
  });

  it("combines the category filter with the search", () => {
    assert.deepEqual(idsFor("card", "titles"), []);
    assert.deepEqual(idsFor("card", "cards"), ["stat-card-solar"]);
  });
});

describe("gfxCategories", () => {
  it("lists each present category once with its count, sorted by name", () => {
    assert.deepEqual(gfxCategories(LIBRARY), [
      { name: "cards", count: 1 },
      { name: "titles", count: 2 },
    ]);
  });

  it("ignores blank categories", () => {
    const withBlank = [...LIBRARY, asset("blank-one", "Blank", "   ", [])];
    assert.deepEqual(gfxCategories(withBlank), gfxCategories(LIBRARY));
  });
});

describe("resolveGfxCategory", () => {
  const categories = gfxCategories(LIBRARY);

  it("keeps a selected category that is still present", () => {
    assert.equal(resolveGfxCategory(categories, "titles"), "titles");
  });

  it("falls back to All when the selected category no longer exists", () => {
    assert.equal(resolveGfxCategory(categories, "overlays"), ALL_GFX_CATEGORIES);
  });

  it("keeps All selected", () => {
    assert.equal(resolveGfxCategory(categories, ALL_GFX_CATEGORIES), ALL_GFX_CATEGORIES);
  });
});
