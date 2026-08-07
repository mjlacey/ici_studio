// §8: Stage B parameter panel. The read-only grouping-summary card (§8.1),
// the editable smoothing-key chip list, the E0/k/R smoother sub-panels
// (R defaulting to "inherit from k"), derivative window/degree, the
// "Smooth & derive" button, and a compact per-group diagnostics readout.

import { runStageB } from "../analysis/stage-runner";
import { activeDataset, store, updateDataset } from "../state";
import type { Dataset, SmootherConfig, SplineDiagnostics, StageBResult } from "../types";
import type { DataWorkerClient } from "../worker/client";

const DIRECTIONS = ["automatic", "increasing", "decreasing"] as const;
const runningDatasets = new Set<string>();

export function mountStageBPanel(container: HTMLElement, worker: DataWorkerClient): void {
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

function joinParts(...parts: string[]): string {
  return parts.filter(Boolean).join(", ");
}

function smoothingKeyLabel(dataset: Dataset): string {
  const { useCycN, useState, groupingColumns } = dataset.stageBConfig.smoothingKey;
  const label = joinParts(groupingColumns.join(", "), useCycN ? "cyc.n" : "", useState ? "state" : "");
  return label || "(none — all rows smoothed together)";
}

function renderGroupingSummary(dataset: Dataset): string {
  const g = dataset.groupingColumns.join(", ");
  return `
    <table class="kv-table grouping-summary">
      <tr><th>Segmentation &amp; rest indexing</th><td>${escapeHtml(g || "(none)")} (whole file if none)</td></tr>
      <tr><th>Q anchoring</th><td>${escapeHtml(joinParts(g, "cyc.n", "state"))}</td></tr>
      <tr><th>Interruption summary</th><td>${escapeHtml(joinParts(g, "cyc.n", "state", "rest"))}</td></tr>
      <tr><th>Rest regression</th><td>${escapeHtml(joinParts(g, "rest"))}</td></tr>
      <tr><th>Smoothing &amp; derivatives</th><td>${escapeHtml(smoothingKeyLabel(dataset))}</td></tr>
    </table>
  `;
}

function renderChips(dataset: Dataset): string {
  const key = dataset.stageBConfig.smoothingKey;
  const chips: { label: string; active: boolean; field: string }[] = [
    { label: "cyc.n", active: key.useCycN, field: "cycN" },
    { label: "state", active: key.useState, field: "state" },
    ...dataset.groupingColumns.map((c) => ({ label: c, active: key.groupingColumns.includes(c), field: c })),
  ];
  return chips.map((c) => `<button type="button" class="chip ${c.active ? "chip-active" : ""}" data-chip="${escapeAttr(c.field)}">${escapeHtml(c.label)}</button>`).join("");
}

function smootherFieldsetHtml(legend: string, prefix: "e0" | "k" | "r", cfg: SmootherConfig, disabled: boolean): string {
  return `
    <fieldset class="smoother" ${disabled ? "disabled" : ""}>
      <legend>${legend}</legend>
      <label><input type="checkbox" data-smoother="${prefix}.monotonic" ${cfg.monotonic ? "checked" : ""} /> Monotonic</label>
      <label>Direction
        <select data-smoother="${prefix}.direction">
          ${DIRECTIONS.map((d) => `<option value="${d}" ${cfg.direction === d ? "selected" : ""}>${d}</option>`).join("")}
        </select>
      </label>
      <label>k <input type="number" min="4" step="1" data-smoother="${prefix}.k" value="${cfg.k}" /></label>
      <label>m <input type="number" min="1" step="1" data-smoother="${prefix}.m" value="${cfg.m}" /></label>
    </fieldset>
  `;
}

function renderPanel(container: HTMLElement, dataset: Dataset, worker: DataWorkerClient, rerender: () => void): void {
  const cfg = dataset.stageBConfig;
  const running = runningDatasets.has(dataset.id);
  const canRun = !!dataset.stageAResult;

  container.innerHTML = `
    <section class="panel">
      <h2>Stage B — smoothing &amp; derivatives</h2>
      ${renderGroupingSummary(dataset)}
      <h3>Smoothing key</h3>
      <div class="chip-row">${renderChips(dataset)}</div>
      ${smootherFieldsetHtml("E0 smoothing", "e0", cfg.e0Smoothing, false)}
      ${smootherFieldsetHtml("k smoothing", "k", cfg.kSmoothing, false)}
      <label class="hint"><input type="checkbox" id="r-inherits-k" ${cfg.rInheritsK ? "checked" : ""} /> R smoothing inherits k smoothing</label>
      ${smootherFieldsetHtml("R smoothing", "r", cfg.rSmoothing, cfg.rInheritsK)}
      <table class="mapping-table">
        <tbody>
          <tr><th>Derivative window (odd)</th><td><input type="number" min="3" step="2" data-field="derivativeWindow" value="${cfg.derivativeWindow}" /></td></tr>
          <tr><th>Derivative degree</th><td><input type="number" min="1" step="1" data-field="derivativeDegree" value="${cfg.derivativeDegree}" /></td></tr>
        </tbody>
      </table>
      <button type="button" class="btn" id="smooth-btn" ${running || !canRun ? "disabled" : ""}>${running ? "Smoothing…" : "Smooth & derive"}</button>
      ${!canRun ? `<p class="hint">Run Stage A first.</p>` : ""}
      ${dataset.stageBResult ? renderDiagnostics(dataset.stageBResult) : ""}
    </section>
  `;

  wireInputs(container, dataset);

  container.querySelector<HTMLButtonElement>("#smooth-btn")!.addEventListener("click", () => {
    runningDatasets.add(dataset.id);
    rerender();
    void runStageB(dataset.id, worker).finally(() => {
      runningDatasets.delete(dataset.id);
      rerender();
    });
  });
}

function wireInputs(container: HTMLElement, dataset: Dataset): void {
  for (const btn of container.querySelectorAll<HTMLButtonElement>("[data-chip]")) {
    btn.addEventListener("click", () => {
      const field = btn.dataset.chip!;
      updateDataset(dataset.id, (d) => {
        const key = d.stageBConfig.smoothingKey;
        let nextKey = key;
        if (field === "cycN") nextKey = { ...key, useCycN: !key.useCycN };
        else if (field === "state") nextKey = { ...key, useState: !key.useState };
        else {
          const has = key.groupingColumns.includes(field);
          nextKey = { ...key, groupingColumns: has ? key.groupingColumns.filter((c) => c !== field) : [...key.groupingColumns, field] };
        }
        return { ...d, stageBConfig: { ...d.stageBConfig, smoothingKey: nextKey } };
      });
    });
  }

  for (const el of container.querySelectorAll<HTMLInputElement | HTMLSelectElement>("[data-smoother]")) {
    el.addEventListener("change", () => {
      const [which, field] = el.dataset.smoother!.split(".");
      const configKey = (which === "e0" ? "e0Smoothing" : which === "k" ? "kSmoothing" : "rSmoothing") as "e0Smoothing" | "kSmoothing" | "rSmoothing";
      updateDataset(dataset.id, (d) => {
        const smoother = { ...d.stageBConfig[configKey] };
        if (field === "monotonic") smoother.monotonic = (el as HTMLInputElement).checked;
        else if (field === "direction") smoother.direction = (el as HTMLSelectElement).value as SmootherConfig["direction"];
        else if (field === "k") smoother.k = Math.max(4, Math.round(Number((el as HTMLInputElement).value)));
        else if (field === "m") smoother.m = Math.max(1, Math.round(Number((el as HTMLInputElement).value)));
        return { ...d, stageBConfig: { ...d.stageBConfig, [configKey]: smoother } };
      });
    });
  }

  container.querySelector<HTMLInputElement>("#r-inherits-k")?.addEventListener("change", (e) => {
    const checked = (e.target as HTMLInputElement).checked;
    updateDataset(dataset.id, (d) => ({ ...d, stageBConfig: { ...d.stageBConfig, rInheritsK: checked } }));
  });

  container.querySelector<HTMLInputElement>('[data-field="derivativeWindow"]')?.addEventListener("change", (e) => {
    let v = Math.round(Number((e.target as HTMLInputElement).value));
    if (v % 2 === 0) v += 1; // §8.3: window must be odd
    v = Math.max(3, v);
    updateDataset(dataset.id, (d) => ({ ...d, stageBConfig: { ...d.stageBConfig, derivativeWindow: v } }));
  });
  container.querySelector<HTMLInputElement>('[data-field="derivativeDegree"]')?.addEventListener("change", (e) => {
    const v = Math.max(1, Math.round(Number((e.target as HTMLInputElement).value)));
    updateDataset(dataset.id, (d) => ({ ...d, stageBConfig: { ...d.stageBConfig, derivativeDegree: v } }));
  });
}

function renderDiagnostics(result: StageBResult): string {
  if (result.groups.length === 0) return "";
  const fmt = (d: SplineDiagnostics | null): string => (d ? `k=${d.kEffective}, λ=${d.lambda.toPrecision(3)}, edf=${d.edf.toFixed(1)}` : "NA (&lt;8 distinct x)");
  const rows = result.groups
    .slice(0, 30)
    .map((g) => `<tr><th>${escapeHtml(g.label)} (n=${g.n})</th><td>${fmt(g.e0)}</td><td>${fmt(g.k)}</td><td>${fmt(g.r)}</td></tr>`)
    .join("");
  return `
    <details class="run-log">
      <summary>Smoothing diagnostics (${result.groups.length} group(s))</summary>
      <table class="kv-table">
        <thead><tr><th>Group</th><th>E0</th><th>k</th><th>R</th></tr></thead>
        <tbody>${rows}</tbody>
      </table>
      ${result.groups.length > 30 ? `<p class="hint">… and ${result.groups.length - 30} more</p>` : ""}
    </details>
  `;
}

function escapeHtml(s: string): string {
  const div = document.createElement("div");
  div.textContent = s;
  return div.innerHTML;
}

function escapeAttr(s: string): string {
  return escapeHtml(s).replace(/"/g, "&quot;");
}
