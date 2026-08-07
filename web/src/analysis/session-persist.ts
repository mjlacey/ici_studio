// §12.5: debounced localStorage persistence of the active dataset's config
// (§12.2) -- data is never persisted, only settings. Mirrors
// stage-runner.ts's exact debounce shape (a serialized-key comparison
// gating a `clearTimeout`/`setTimeout` pair) rather than introducing a new
// debounce pattern.

import { buildConfigExport } from "../export/config";
import { activeDataset, store } from "../state";
import type { ExportedConfig } from "../types";

const SESSION_STORAGE_KEY = "ici-web:session-config";
const PERSIST_DEBOUNCE_MS = 500;

let lastPersistedKey: string | null = null;
let pendingTimer: ReturnType<typeof setTimeout> | null = null;

function currentConfigJson(): string | null {
  const state = store.get();
  const dataset = activeDataset(state);
  if (!dataset) return null;
  return JSON.stringify(buildConfigExport(dataset, state.analysisConfig));
}

export function startSessionPersistEffect(): void {
  store.subscribe(check);
  check();
}

function check(): void {
  const json = currentConfigJson();
  if (json === lastPersistedKey) return;
  lastPersistedKey = json;
  if (json === null) return;

  if (pendingTimer) clearTimeout(pendingTimer);
  pendingTimer = setTimeout(() => {
    pendingTimer = null;
    try {
      localStorage.setItem(SESSION_STORAGE_KEY, json);
    } catch {
      // localStorage unavailable/full -- not fatal, just skip persistence.
    }
  }, PERSIST_DEBOUNCE_MS);
}

export function hasPersistedConfig(): boolean {
  try {
    return localStorage.getItem(SESSION_STORAGE_KEY) !== null;
  } catch {
    return false;
  }
}

export function loadPersistedConfig(): ExportedConfig | null {
  try {
    const raw = localStorage.getItem(SESSION_STORAGE_KEY);
    if (!raw) return null;
    return JSON.parse(raw) as ExportedConfig;
  } catch {
    return null;
  }
}
