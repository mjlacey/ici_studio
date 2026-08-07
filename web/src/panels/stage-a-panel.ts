// §7: Stage A parameter panel. State threshold, voltage/current windows,
// edge points, and the two advanced flags (drop-unrested-reversals,
// legacy-compatibility), plus the "Fit resistances" button and a run-log
// readout (regression failures, non-physical R/k counts by state -- §7.8).
// Stage A never auto-runs (see stage-runner.ts's module doc comment).

import { runStageA } from "../analysis/stage-runner";
import { activeDataset, store, updateDataset } from "../state";
import type { Dataset, StageAConfig, StageAResult } from "../types";
import type { DataWorkerClient } from "../worker/client";

const runningDatasets = new Set<string>();

export function mountStageAPanel(container: HTMLElement, worker: DataWorkerClient): void {
  function render(): void {
    const dataset = activeDataset(store.get());
    if (!dataset?.segmentation) {
      container.innerHTML = "";
      return;
    }
    renderPanel(container, dataset, worker, render);
  }

  store.subscribe(render);
  render();
}

function windowInputHtml(field: string, value: number | null): string {
  const isNull = value === null;
  return `
    <div class="window-input">
      <input type="number" step="any" min="0" data-field="${field}" value="${isNull ? "" : value}" ${isNull ? "disabled" : ""} />
      <label class="hint"><input type="checkbox" data-field-null="${field}" ${isNull ? "checked" : ""} /> last point only</label>
    </div>
  `;
}

function renderPanel(container: HTMLElement, dataset: Dataset, worker: DataWorkerClient, rerender: () => void): void {
  const cfg = dataset.stageAConfig;
  const running = runningDatasets.has(dataset.id);

  container.innerHTML = `
    <section class="panel">
      <h2>Stage A — resistance fitting</h2>
      <table class="mapping-table">
        <tbody>
          <tr><th>State threshold (A)</th><td><input type="number" step="any" min="0" data-field="stateThreshold" value="${cfg.stateThreshold}" /></td></tr>
          <tr><th>Voltage interp. window (s)</th><td>${windowInputHtml("voltageInterpolationWindow", cfg.voltageInterpolationWindow)}</td></tr>
          <tr><th>Current avg. window (s)</th><td>${windowInputHtml("currentAverageWindow", cfg.currentAverageWindow)}</td></tr>
          <tr><th>Edge points</th><td><input type="number" step="1" min="1" data-field="edgePoints" value="${cfg.edgePoints}" /></td></tr>
        </tbody>
      </table>
      <details class="overrides">
        <summary>Advanced</summary>
        <div class="override-grid">
          <label><input type="checkbox" data-field="dropUnrestedReversals" ${cfg.dropUnrestedReversals ? "checked" : ""} /> Drop unrested reversals</label>
          <label><input type="checkbox" data-field="legacyCompatibility" ${cfg.legacyCompatibility ? "checked" : ""} /> Legacy compatibility (I = mean of all active samples)</label>
        </div>
      </details>
      <button type="button" class="btn" id="fit-btn" ${running ? "disabled" : ""}>${running ? "Fitting…" : "Fit resistances"}</button>
      ${dataset.stageAError ? `<p class="status status-error">${escapeHtml(dataset.stageAError)}</p>` : ""}
      ${dataset.stageAResult ? renderRunLog(dataset.stageAResult) : ""}
    </section>
  `;

  wireInputs(container, dataset);

  container.querySelector<HTMLButtonElement>("#fit-btn")!.addEventListener("click", () => {
    runningDatasets.add(dataset.id);
    rerender();
    void runStageA(dataset.id, worker).finally(() => {
      runningDatasets.delete(dataset.id);
      rerender();
    });
  });
}

function wireInputs(container: HTMLElement, dataset: Dataset): void {
  const setField = <K extends keyof StageAConfig>(field: K, value: StageAConfig[K]): void => {
    updateDataset(dataset.id, (d) => ({ ...d, stageAConfig: { ...d.stageAConfig, [field]: value } }));
  };

  container.querySelector<HTMLInputElement>('[data-field="stateThreshold"]')?.addEventListener("change", (e) => {
    const v = Number((e.target as HTMLInputElement).value);
    if (Number.isFinite(v) && v >= 0) setField("stateThreshold", v);
  });
  container.querySelector<HTMLInputElement>('[data-field="edgePoints"]')?.addEventListener("change", (e) => {
    const v = Math.max(1, Math.round(Number((e.target as HTMLInputElement).value)));
    if (Number.isFinite(v)) setField("edgePoints", v);
  });

  for (const field of ["voltageInterpolationWindow", "currentAverageWindow"] as const) {
    container.querySelector<HTMLInputElement>(`[data-field="${field}"]`)?.addEventListener("change", (e) => {
      const v = Number((e.target as HTMLInputElement).value);
      if (Number.isFinite(v) && v >= 0) setField(field, v);
    });
    container.querySelector<HTMLInputElement>(`[data-field-null="${field}"]`)?.addEventListener("change", (e) => {
      setField(field, (e.target as HTMLInputElement).checked ? null : 10);
    });
  }

  container.querySelector<HTMLInputElement>('[data-field="dropUnrestedReversals"]')?.addEventListener("change", (e) => {
    setField("dropUnrestedReversals", (e.target as HTMLInputElement).checked);
  });
  container.querySelector<HTMLInputElement>('[data-field="legacyCompatibility"]')?.addEventListener("change", (e) => {
    setField("legacyCompatibility", (e.target as HTMLInputElement).checked);
  });
}

function renderRunLog(result: StageAResult): string {
  const failCount = result.regressionFailures.length;
  const totalRests = result.segmentation.totalRests;
  const fittedCount = totalRests - failCount;

  const nonphysicalNote = (label: string, byState: Record<string, number>): string => {
    const entries = Object.entries(byState).filter(([, n]) => n > 0);
    if (entries.length === 0) return "";
    const total = entries.reduce((sum, [, n]) => sum + n, 0);
    const clustered = entries.length === 1;
    return `<li class="issue issue-warning">${total} non-physical ${label} value(s) set to NA (${entries.map(([s, n]) => `${s}: ${n}`).join(", ")})${
      clustered ? " — clustered in one state, possibly a sign/orientation issue" : ""
    }</li>`;
  };

  const failuresHtml =
    failCount > 0
      ? `<details><summary>${failCount} regression failure(s)</summary><ul class="issues">${result.regressionFailures
          .slice(0, 50)
          .map((f) => `<li>rest ${f.rest} (group ${f.groupId}): ${escapeHtml(f.reason)}</li>`)
          .join("")}${failCount > 50 ? `<li>… and ${failCount - 50} more</li>` : ""}</ul></details>`
      : "";

  const nonphysicalHtml = nonphysicalNote("R", result.nonphysicalReport.rByState) + nonphysicalNote("k", result.nonphysicalReport.kByState);

  return `
    <div class="run-log">
      <p class="hint">${fittedCount} of ${totalRests} rests fitted successfully.</p>
      ${failuresHtml}
      ${nonphysicalHtml ? `<ul class="issues">${nonphysicalHtml}</ul>` : ""}
    </div>
  `;
}

function escapeHtml(s: string): string {
  const div = document.createElement("div");
  div.textContent = s;
  return div.innerHTML;
}
