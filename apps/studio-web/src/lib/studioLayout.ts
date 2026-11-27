export interface StudioGridInput {
  width: number;
  height: number;
  canvasAspect: number;
  scopesOpen: boolean;
  viewerMode: "program" | "source";
}

export interface StudioGridLayout {
  gridTemplateColumns: string;
  directorPx: number;
  programPx: number;
  scopesPx: number;
  inspectorPx: number;
}

/**
 * Computes exact pixel column tracks for the desktop Studio grid across
 * 9:16, 1:1, 16:9, and Source Monitor modes so Program stays horizontally
 * centered, Scopes stay compact, and columns never overflow the container.
 */
export function computeStudioGridLayout(
  input: StudioGridInput,
): StudioGridLayout | null {
  const { width, height, canvasAspect, scopesOpen, viewerMode } = input;
  if (!Number.isFinite(width) || !Number.isFinite(height) || width < 1024 || height <= 0) {
    return null;
  }

  const isXl = width >= 1280;
  const showScopes = isXl && scopesOpen;
  const gapPx = 8;
  const gapsTotal = showScopes ? 24 : 16;
  const scopesPx = showScopes
    ? width >= 1600
      ? 216
      : width >= 1400
        ? 204
        : 196
    : 0;

  const minDirector = isXl ? 300 : 260;
  const minInspector = isXl ? 272 : 252;

  const safeAspect =
    Number.isFinite(canvasAspect) && canvasAspect > 0 ? canvasAspect : 9 / 16;
  const effectiveAspect =
    viewerMode === "source" ? Math.max(safeAspect, 16 / 9) : safeAspect;

  const stageInnerH = Math.max(240, height - 52);
  const idealProgramPx = Math.round(stageInnerH * effectiveAspect + 20);
  const minProgramPx =
    effectiveAspect < 0.85
      ? 268
      : effectiveAspect <= 1.2
        ? 340
        : isXl
          ? 440
          : 360;

  const maxProgramPx = Math.max(
    minProgramPx,
    width - minDirector - minInspector - scopesPx - gapsTotal,
  );
  const programPx = Math.round(
    Math.max(minProgramPx, Math.min(idealProgramPx, maxProgramPx)),
  );

  const remaining = Math.max(
    minDirector + minInspector,
    width - programPx - scopesPx - gapsTotal,
  );

  let directorPx: number;
  let inspectorPx: number;

  if (showScopes) {
    const idealInspector = Math.round((remaining - scopesPx - gapPx) / 2);
    inspectorPx = Math.max(
      minInspector,
      Math.min(remaining - minDirector, idealInspector),
    );
    directorPx = remaining - inspectorPx;
  } else {
    directorPx = Math.max(
      minDirector,
      Math.min(remaining - minInspector, Math.round(remaining / 2)),
    );
    inspectorPx = remaining - directorPx;
  }

  const gridTemplateColumns = showScopes
    ? `${directorPx}px ${programPx}px ${scopesPx}px ${inspectorPx}px`
    : `${directorPx}px ${programPx}px ${inspectorPx}px`;

  return {
    gridTemplateColumns,
    directorPx,
    programPx,
    scopesPx,
    inspectorPx,
  };
}
