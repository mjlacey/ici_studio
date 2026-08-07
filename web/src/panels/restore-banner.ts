// §12.5: on boot, if a stored session config exists, offer to restore it.
// Restore just sets `pendingRestoredConfig`, silently applied the moment
// the next file is dropped (`import-panel.ts`) -- narrowed scope, per the
// user's explicit choice, from the literal spec text's "populate every
// parameter panel before a file is loaded" (which would need a second
// render path threaded through all six left-column config panels for a
// fairly marginal win over this).

import { hasPersistedConfig, loadPersistedConfig } from "../analysis/session-persist";
import { store } from "../state";

export function mountRestoreBanner(container: HTMLElement): void {
  if (!hasPersistedConfig()) {
    container.innerHTML = "";
    return;
  }

  container.innerHTML = `
    <div class="info-note restore-banner">
      Restore your last session's settings? (data must be re-imported)
      <button type="button" class="btn-small" id="restore-btn">Restore</button>
      <button type="button" class="btn-small" id="dismiss-btn">Dismiss</button>
    </div>
  `;

  container.querySelector<HTMLButtonElement>("#restore-btn")!.addEventListener("click", () => {
    const config = loadPersistedConfig();
    if (config) {
      store.set((s) => ({ ...s, pendingRestoredConfig: config }));
    }
    container.innerHTML = "";
  });
  container.querySelector<HTMLButtonElement>("#dismiss-btn")!.addEventListener("click", () => {
    container.innerHTML = "";
  });
}
