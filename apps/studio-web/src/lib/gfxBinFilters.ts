import type { SavedCodeAsset } from "@solaris/engine-client";

// Blank categories are dropped below, so the empty string can never name a real category.
export const ALL_GFX_CATEGORIES = "";

export interface GfxCategory {
  name: string;
  count: number;
}

export function gfxCategories(assets: readonly SavedCodeAsset[]): GfxCategory[] {
  const counts = new Map<string, number>();
  for (const asset of assets) {
    const name = asset.category.trim();
    if (name) counts.set(name, (counts.get(name) ?? 0) + 1);
  }
  return [...counts]
    .map(([name, count]) => ({ name, count }))
    .sort((a, b) => a.name.localeCompare(b.name));
}

export function resolveGfxCategory(categories: readonly GfxCategory[], selected: string): string {
  if (selected === ALL_GFX_CATEGORIES) return selected;
  return categories.some((category) => category.name === selected)
    ? selected
    : ALL_GFX_CATEGORIES;
}

function parseGfxQuery(query: string): string[] {
  return query.trim().toLowerCase().split(/\s+/).filter(Boolean);
}

// Every whitespace-separated term must match the name, id, or a tag (case-insensitive).
export function filterGfxAssets(
  assets: readonly SavedCodeAsset[],
  { query, category }: { query: string; category: string },
): SavedCodeAsset[] {
  const terms = parseGfxQuery(query);
  return assets.filter((asset) => {
    if (category !== ALL_GFX_CATEGORIES && asset.category.trim() !== category) return false;
    const fields = [asset.name, asset.id, ...asset.tags].map((field) => field.toLowerCase());
    return terms.every((term) => fields.some((field) => field.includes(term)));
  });
}
