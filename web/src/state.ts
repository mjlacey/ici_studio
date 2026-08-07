// Hand-rolled observable store (playbook §1/§4 pattern: no framework, a
// tiny subscribe/notify store is enough for a single-screen app).

import { defaultAnalysisConfig, type AppState, type Dataset } from "./types";

type Listener = () => void;

class Store<T> {
  private state: T;
  private listeners = new Set<Listener>();

  constructor(initial: T) {
    this.state = initial;
  }

  get(): T {
    return this.state;
  }

  set(updater: (prev: T) => T): void {
    this.state = updater(this.state);
    for (const listener of this.listeners) listener();
  }

  subscribe(listener: Listener): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }
}

export const store = new Store<AppState>({
  datasets: [],
  activeDatasetId: null,
  analysisConfig: defaultAnalysisConfig(),
  pendingRestoredConfig: null,
});

export function activeDataset(state: AppState): Dataset | null {
  return state.datasets.find((d) => d.id === state.activeDatasetId) ?? null;
}

/** Replaces one dataset by id, leaving the rest of the array untouched. */
export function updateDataset(id: string, updater: (prev: Dataset) => Dataset): void {
  store.set((state) => ({
    ...state,
    datasets: state.datasets.map((d) => (d.id === id ? updater(d) : d)),
  }));
}
