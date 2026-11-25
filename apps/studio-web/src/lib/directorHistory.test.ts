import { test } from "node:test";
import { deepStrictEqual, ok, strictEqual } from "node:assert/strict";
import {
  appendDirectorTurn,
  conversationHistory,
  directorHistoryKey,
  loadDirectorHistory,
  parseDirectorHistory,
  saveDirectorHistory,
  type DirectorHistoryStorage,
  type DirectorTurn,
} from "./directorHistory.ts";

function turns(count: number): DirectorTurn[] {
  return Array.from({ length: count }, (_, index): DirectorTurn => ({
    role: index % 2 === 0 ? "user" : "assistant",
    text: `turn ${index + 1}`,
  }));
}

function memoryStorage(seed: Record<string, string> = {}) {
  const items = new Map(Object.entries(seed));
  return {
    items,
    getItem: (key: string) => items.get(key) ?? null,
    setItem: (key: string, value: string) => {
      items.set(key, value);
    },
  };
}

const blockedStorage: DirectorHistoryStorage = {
  getItem() {
    throw new Error("SecurityError: storage is disabled");
  },
  setItem() {
    throw new Error("QuotaExceededError");
  },
};

const PROJECT_KEY = "solaris:director-history:p";

test("directorHistoryKey scopes the history to one project", () => {
  strictEqual(directorHistoryKey("proj-1"), "solaris:director-history:proj-1");
});

test("parseDirectorHistory returns nothing for values that are not an array", () => {
  deepStrictEqual(parseDirectorHistory({ turns: [] }), []);
  deepStrictEqual(parseDirectorHistory("user: hi"), []);
  deepStrictEqual(parseDirectorHistory(null), []);
  deepStrictEqual(parseDirectorHistory(42), []);
});

test("parseDirectorHistory drops malformed entries and strips fields that are not role or text", () => {
  const raw = [
    { role: "user", text: "make the hook punchier" },
    { role: "system", text: "wrong role" },
    { role: "assistant", text: 3 },
    null,
    "assistant: plain string",
    ["user", "nested"],
    { role: "assistant", text: "Tightened the intro.", apiKey: "sk-or-secret" },
  ];
  deepStrictEqual(parseDirectorHistory(raw), [
    { role: "user", text: "make the hook punchier" },
    { role: "assistant", text: "Tightened the intro." },
  ]);
});

test("parseDirectorHistory clamps each text to 2000 characters without splitting an emoji", () => {
  const [turn] = parseDirectorHistory([{ role: "user", text: "😀".repeat(2001) }]);
  strictEqual(turn.text, "😀".repeat(2000));
});

test("appendDirectorTurn keeps only the newest 20 turns", () => {
  let history: DirectorTurn[] = [];
  for (let index = 1; index <= 25; index++) {
    history = appendDirectorTurn(history, { role: "user", text: `prompt ${index}` });
  }
  strictEqual(history.length, 20);
  strictEqual(history[0].text, "prompt 6");
  strictEqual(history.at(-1)?.text, "prompt 25");
});

test("conversationHistory sends only the last 12 turns", () => {
  const history = turns(20);
  deepStrictEqual(conversationHistory(history), history.slice(8));
  deepStrictEqual(conversationHistory(turns(5)), turns(5));
});

test("saveDirectorHistory writes at most 20 turns under the project key and load reads them back", () => {
  const storage = memoryStorage();
  saveDirectorHistory(storage, "p", turns(25));
  const saved = storage.items.get(PROJECT_KEY);
  ok(saved);
  strictEqual(JSON.parse(saved).length, 20);
  deepStrictEqual(loadDirectorHistory(storage, "p"), turns(25).slice(5));
  deepStrictEqual(loadDirectorHistory(storage, "other"), []);
});

test("saveDirectorHistory persists only role and text", () => {
  const storage = memoryStorage();
  saveDirectorHistory(storage, "p", [
    { role: "user", text: "tighten", token: "secret-token" } as DirectorTurn,
  ]);
  strictEqual(storage.items.get(PROJECT_KEY), '[{"role":"user","text":"tighten"}]');
});

test("loadDirectorHistory treats malformed or non-array JSON as an empty history", () => {
  deepStrictEqual(loadDirectorHistory(memoryStorage({ [PROJECT_KEY]: "{not json" }), "p"), []);
  deepStrictEqual(loadDirectorHistory(memoryStorage({ [PROJECT_KEY]: '{"turns":[]}' }), "p"), []);
});

test("loadDirectorHistory keeps the valid turns from a partly corrupt entry", () => {
  const raw = JSON.stringify([
    { role: "user", text: "tighten" },
    { role: "bot", text: "nope" },
  ]);
  deepStrictEqual(loadDirectorHistory(memoryStorage({ [PROJECT_KEY]: raw }), "p"), [
    { role: "user", text: "tighten" },
  ]);
});

test("history survives storage that throws on read or write", () => {
  deepStrictEqual(loadDirectorHistory(blockedStorage, "p"), []);
  saveDirectorHistory(blockedStorage, "p", turns(3));
});

test("history helpers do nothing when no storage is available", () => {
  deepStrictEqual(loadDirectorHistory(null, "p"), []);
  saveDirectorHistory(null, "p", turns(2));
});
