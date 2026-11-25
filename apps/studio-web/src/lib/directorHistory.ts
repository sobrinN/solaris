export interface DirectorTurn {
  role: "user" | "assistant";
  text: string;
}

export interface DirectorHistoryStorage {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
}

const DIRECTOR_HISTORY_LIMIT = 20;
const CONVERSATION_HISTORY_LIMIT = 12;
const DIRECTOR_TURN_TEXT_LIMIT = 2000;

export function directorHistoryKey(projectId: string): string {
  return `solaris:director-history:${projectId}`;
}

/** Rebuilds each valid turn as `{ role, text }`, so no other field can ever be persisted. */
export function parseDirectorHistory(value: unknown): DirectorTurn[] {
  if (!Array.isArray(value)) return [];
  const turns = value.flatMap((item): DirectorTurn[] => {
    if (typeof item !== "object" || item === null) return [];
    const { role, text } = item as { role?: unknown; text?: unknown };
    if ((role !== "user" && role !== "assistant") || typeof text !== "string") return [];
    return [{ role, text: [...text].slice(0, DIRECTOR_TURN_TEXT_LIMIT).join("") }];
  });
  return turns.slice(-DIRECTOR_HISTORY_LIMIT);
}

export function appendDirectorTurn(
  turns: readonly DirectorTurn[],
  turn: DirectorTurn,
): DirectorTurn[] {
  return parseDirectorHistory([...turns, turn]);
}

export function conversationHistory(turns: readonly DirectorTurn[]): DirectorTurn[] {
  return turns.slice(-CONVERSATION_HISTORY_LIMIT);
}

export function loadDirectorHistory(
  storage: DirectorHistoryStorage | null,
  projectId: string,
): DirectorTurn[] {
  if (!storage) return [];
  try {
    const raw = storage.getItem(directorHistoryKey(projectId));
    return raw === null ? [] : parseDirectorHistory(JSON.parse(raw));
  } catch {
    return [];
  }
}

export function saveDirectorHistory(
  storage: DirectorHistoryStorage | null,
  projectId: string,
  turns: readonly DirectorTurn[],
): void {
  if (!storage) return;
  try {
    storage.setItem(directorHistoryKey(projectId), JSON.stringify(parseDirectorHistory(turns)));
  } catch {
    // Storage can be full or blocked. History is best-effort, so the save is skipped.
  }
}
