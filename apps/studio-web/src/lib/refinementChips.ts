export interface RefinementChip {
  id: string;
  label: string;
  prompt: string;
}

export const REFINEMENT_CHIPS: readonly RefinementChip[] = [
  {
    id: "fix-qa-issues",
    label: "Fix QA issues",
    prompt: "Fix the issues in the last QA report. Change only the clips those issues point to.",
  },
  {
    id: "tighten-pacing",
    label: "Tighten pacing",
    prompt: "Tighten the pacing. Trim dead air and long holds, and keep the key beats.",
  },
  {
    id: "brighter-grade",
    label: "Brighter grade",
    prompt: "Make the grade brighter and slightly warmer. Keep skin tones natural.",
  },
  {
    id: "move-gfx-clear-of-captions",
    label: "Move GFX clear of captions",
    prompt: "Move the GFX overlays so they stay clear of the captions. Keep their content.",
  },
  {
    id: "shorter-captions",
    label: "Shorter captions",
    prompt: "Shorten the captions to 2-4 words per burst. Keep the meaning.",
  },
  {
    id: "remove-silent-gaps",
    label: "Remove silent gaps",
    prompt: "Cut silences longer than 0.25 s and keep a 0.12 s breath pad around the narration.",
  },
  {
    id: "punchier-captions",
    label: "Punchier captions",
    prompt: "Make the captions punchier. Use sharper wording and keep the meaning.",
  },
];
