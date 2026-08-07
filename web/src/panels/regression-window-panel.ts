// §10.1: manual t_min/t_max entry with live point-count feedback, for the
// selected rest and the median across all rests. Bidirectionally in sync
// with Plot 2's draggable window bounds via the shared `regressionWindow`
// store field.
//
// §10.3 "Estimate optimal window" lives in this same panel -- the spec
// places its button "below the two inputs", i.e. inside this section, not
// as a separate one.

import { activeDataset, store, updateDataset } from "../state";
import type { CandidateScore, Dataset, OptimalWindowResult } from "../types";
import type { DataWorkerClient } from "../worker/client";

function countInWindow(restStepTs: number[], tMin: number, tMax: number): number {
  let count = 0;
  for (const t of restStepTs) if (t >= tMin && t <= tMax) count++;
  return count;
}

function median(values: number[]): number {
  if (values.length === 0) return 0;
  const sorted = [...values].sort((a, b) => a - b);
  const mid = Math.floor(sorted.length / 2);
  return sorted.length % 2 === 0 ? (sorted[mid - 1] + sorted[mid]) / 2 : sorted[mid];
}

interface RunningState {
  completed: number;
  total: number;
  cancel: () => void;
}

const runningByDataset = new Map<string, RunningState>();

export function mountRegressionWindowPanel(container: HTMLElement, worker: DataWorkerClient): void {
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

function renderPanel(container: HTMLElement, dataset: Dataset, worker: DataWorkerClient, rerender: () => void): void {
  const { tMin, tMax } = dataset.regressionWindow;
  const boundaries = dataset.segmentation?.restBoundaries ?? [];
  const selected = boundaries.find((b) => b.groupId === dataset.selectedGroupId && b.restId === dataset.selectedRestId);
  const selectedCount = selected ? countInWindow(selected.restStepTs, tMin, tMax) : null;
  const medianCount = median(boundaries.map((b) => countInWindow(b.restStepTs, tMin, tMax)));

  const owCfg = dataset.optimalWindowConfig;
  const running = runningByDataset.get(dataset.id) ?? null;

  container.innerHTML = `
    <section class="panel">
      <h2>Regression window</h2>
      <table class="mapping-table">
        <tbody>
          <tr><th>t_min (s)</th><td><input type="number" step="any" data-window="tMin" value="${tMin}" /></td></tr>
          <tr><th>t_max (s)</th><td><input type="number" step="any" data-window="tMax" value="${tMax}" /></td></tr>
        </tbody>
      </table>
      <p class="hint">
        Selected rest: ${selectedCount === null ? "—" : `${selectedCount} point(s)`}${
          selectedCount !== null && selectedCount < 3 ? ' <span class="too-few-points">too few</span>' : ""
        }<br />
        Median across all ${boundaries.length} rests: ${medianCount} point(s)
      </p>

      <h3>Estimate optimal window</h3>
      <table class="mapping-table">
        <tbody>
          <tr><th>N (rests sampled)</th><td><input type="number" min="5" max="100" step="1" data-owfield="n" value="${owCfg.n}" /></td></tr>
        </tbody>
      </table>
      <details class="overrides">
        <summary>Advanced</summary>
        <div class="override-grid">
          <label>L_min (s) <input type="number" min="1" step="1" data-owfield="lMin" value="${owCfg.lMin}" /></label>
          <label>t_min lower bound (s) <input type="number" min="0" step="1" data-owfield="tMinLowerBound" value="${owCfg.tMinLowerBound}" /></label>
        </div>
      </details>
      <button type="button" class="btn" id="estimate-btn" ${running ? "disabled" : ""}>
        ${running ? "Estimating…" : "Estimate optimal window"}
      </button>
      ${
        running
          ? `<div class="ow-progress">
               <progress max="${Math.max(running.total, 1)}" value="${running.completed}"></progress>
               <span class="hint">${running.completed} / ${running.total}</span>
               <button type="button" class="btn-small" id="cancel-btn">Cancel</button>
             </div>`
          : ""
      }
      ${dataset.optimalWindowError ? `<p class="status status-error">${escapeHtml(dataset.optimalWindowError)}</p>` : ""}
      ${dataset.optimalWindowResult ? renderOptimalWindowResults(dataset.optimalWindowResult) : ""}
    </section>
  `;

  wireInputs(container, dataset, worker, rerender);
}

function wireInputs(container: HTMLElement, dataset: Dataset, worker: DataWorkerClient, rerender: () => void): void {
  const minInput = container.querySelector<HTMLInputElement>('[data-window="tMin"]')!;
  const maxInput = container.querySelector<HTMLInputElement>('[data-window="tMax"]')!;
  minInput.addEventListener("change", () => {
    const v = Number(minInput.value);
    if (Number.isFinite(v)) updateDataset(dataset.id, (d) => ({ ...d, regressionWindow: { ...d.regressionWindow, tMin: v } }));
  });
  maxInput.addEventListener("change", () => {
    const v = Number(maxInput.value);
    if (Number.isFinite(v)) updateDataset(dataset.id, (d) => ({ ...d, regressionWindow: { ...d.regressionWindow, tMax: v } }));
  });

  container.querySelector<HTMLInputElement>('[data-owfield="n"]')?.addEventListener("change", (e) => {
    const v = Math.round(Number((e.target as HTMLInputElement).value));
    if (Number.isFinite(v)) {
      const clamped = Math.min(100, Math.max(5, v));
      updateDataset(dataset.id, (d) => ({ ...d, optimalWindowConfig: { ...d.optimalWindowConfig, n: clamped } }));
    }
  });
  container.querySelector<HTMLInputElement>('[data-owfield="lMin"]')?.addEventListener("change", (e) => {
    const v = Math.max(1, Math.round(Number((e.target as HTMLInputElement).value)));
    if (Number.isFinite(v)) updateDataset(dataset.id, (d) => ({ ...d, optimalWindowConfig: { ...d.optimalWindowConfig, lMin: v } }));
  });
  container.querySelector<HTMLInputElement>('[data-owfield="tMinLowerBound"]')?.addEventListener("change", (e) => {
    const v = Math.max(0, Math.round(Number((e.target as HTMLInputElement).value)));
    if (Number.isFinite(v)) updateDataset(dataset.id, (d) => ({ ...d, optimalWindowConfig: { ...d.optimalWindowConfig, tMinLowerBound: v } }));
  });

  container.querySelector<HTMLButtonElement>("#estimate-btn")?.addEventListener("click", () => {
    const cfg = dataset.optimalWindowConfig;
    updateDataset(dataset.id, (d) => ({ ...d, optimalWindowError: null }));
    const startedAt = performance.now();
    const { promise, cancel } = worker.estimateOptimalWindow(
      dataset.id,
      { n: cfg.n, lMin: cfg.lMin, tMinLowerBound: cfg.tMinLowerBound, edgePoints: dataset.stageAConfig.edgePoints },
      (completed, total) => {
        runningByDataset.set(dataset.id, { completed, total, cancel });
        rerender();
      },
    );
    runningByDataset.set(dataset.id, { completed: 0, total: 0, cancel });
    rerender();

    void promise.then((outcome) => {
      runningByDataset.delete(dataset.id);
      if (outcome.ok) {
        updateDataset(dataset.id, (d) => ({
          ...d,
          optimalWindowResult: outcome.result,
          optimalWindowError: null,
          optimalWindowCompletedAt: Date.now(),
          optimalWindowTimingMs: performance.now() - startedAt,
        }));
      } else if (outcome.error !== "cancelled") {
        updateDataset(dataset.id, (d) => ({ ...d, optimalWindowError: outcome.error }));
      } else {
        rerender();
      }
    });
  });

  container.querySelector<HTMLButtonElement>("#cancel-btn")?.addEventListener("click", () => {
    runningByDataset.get(dataset.id)?.cancel();
  });

  for (const btn of container.querySelectorAll<HTMLButtonElement>("[data-apply-tmin]")) {
    btn.addEventListener("click", () => {
      const tMin = Number(btn.dataset.applyTmin);
      const tMax = Number(btn.dataset.applyTmax);
      updateDataset(dataset.id, (d) => ({ ...d, regressionWindow: { tMin, tMax } }));
    });
  }
}

/** Exported so §12.3's run-log uses the identical ranking, not a reimplementation. */
export function rankCandidates(scores: CandidateScore[]): CandidateScore[] {
  return scores
    .filter((s) => !s.rejected)
    .sort((a, b) => {
      if (Math.abs(a.meanAdjR2 - b.meanAdjR2) > 1e-6) return b.meanAdjR2 - a.meanAdjR2;
      const lenA = a.tMax - a.tMin;
      const lenB = b.tMax - b.tMin;
      if (lenA !== lenB) return lenB - lenA; // tie -> longer window wins
      return a.tMin - b.tMin; // tie -> smaller t_min wins
    });
}

function renderOptimalWindowResults(result: OptimalWindowResult): string {
  const top10 = rankCandidates(result.scores).slice(0, 10);
  const sampledList = result.sampledRests.map((r) => `#${r.restId} (${r.state}, Q=${r.q.toPrecision(4)})`).join(", ");

  const rowsHtml = top10
    .map(
      (c) => `
        <tr>
          <td><button type="button" class="btn-small" data-apply-tmin="${c.tMin}" data-apply-tmax="${c.tMax}">Apply</button></td>
          <td>${c.tMin}</td>
          <td>${c.tMax}</td>
          <td>${c.meanAdjR2.toPrecision(4)}</td>
          <td>${c.medianAdjR2.toPrecision(4)}</td>
          <td>${c.nValid}/${c.nSampled}</td>
          <td>${c.medianNPts}</td>
          <td>${fmtOrDash(c.medianEdgeMaxZ)}</td>
        </tr>`,
    )
    .join("");

  return `
    <div class="ow-results">
      <p class="hint ow-caveat">Adjusted R² compares fits over <em>different</em> point subsets across candidates -- treat this as a heuristic ranking, not a formal comparison. Check median adj R² and median edge_max_z before accepting.</p>
      ${result.heterogeneousLengths ? `<p class="hint">Rest lengths vary -- using the 5th percentile of per-rest max step.t as the candidate grid's upper bound.</p>` : ""}
      <p class="hint">Sampled ${result.sampledRests.length} rest(s): ${escapeHtml(sampledList)}</p>
      ${
        top10.length === 0
          ? `<p class="hint">No candidate window passed the 80% fit-rate threshold.</p>`
          : `<div class="table-scroll-x">
               <table class="kv-table ow-table">
                 <thead>
                   <tr><th></th><th>t_min</th><th>t_max</th><th>mean adj R²</th><th>median adj R²</th><th>n valid</th><th>median n_pts</th><th>median edge_max_z</th></tr>
                 </thead>
                 <tbody>${rowsHtml}</tbody>
               </table>
             </div>`
      }
      ${renderHeatmap(result.scores)}
    </div>
  `;
}

// Sequential single-hue (blue) ramp, light→dark, for continuous magnitude
// encoding (mean adj R²) -- matches the app's existing accent blue.
const SEQUENTIAL_RAMP = [
  "#cde2fb",
  "#b7d3f6",
  "#9ec5f4",
  "#86b6ef",
  "#6da7ec",
  "#5598e7",
  "#3987e5",
  "#2a78d6",
  "#256abf",
  "#1c5cab",
  "#184f95",
  "#104281",
  "#0d366b",
];

function hexToRgb(hex: string): [number, number, number] {
  const n = parseInt(hex.slice(1), 16);
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
}

function sequentialColor(t: number): string {
  const clamped = Math.max(0, Math.min(1, t));
  const scaled = clamped * (SEQUENTIAL_RAMP.length - 1);
  const i = Math.floor(scaled);
  const frac = scaled - i;
  const [r0, g0, b0] = hexToRgb(SEQUENTIAL_RAMP[i]);
  const [r1, g1, b1] = hexToRgb(SEQUENTIAL_RAMP[Math.min(i + 1, SEQUENTIAL_RAMP.length - 1)]);
  const r = Math.round(r0 + (r1 - r0) * frac);
  const g = Math.round(g0 + (g1 - g0) * frac);
  const b = Math.round(b0 + (b1 - b0) * frac);
  return `rgb(${r},${g},${b})`;
}

function renderHeatmap(scores: CandidateScore[]): string {
  if (scores.length === 0) return "";
  const tMins = [...new Set(scores.map((s) => s.tMin))].sort((a, b) => a - b);
  const tMaxs = [...new Set(scores.map((s) => s.tMax))].sort((a, b) => a - b);
  const byKey = new Map(scores.map((s) => [`${s.tMin}:${s.tMax}`, s]));

  const nonRejected = scores.filter((s) => !s.rejected && Number.isFinite(s.meanAdjR2));
  const domainMin = nonRejected.length ? Math.min(...nonRejected.map((s) => s.meanAdjR2)) : 0;
  const domainMax = nonRejected.length ? Math.max(...nonRejected.map((s) => s.meanAdjR2)) : 1;

  const cells = tMins.flatMap((tMinValue) =>
    tMaxs.map((tMaxValue) => {
      const s = byKey.get(`${tMinValue}:${tMaxValue}`);
      if (!s) return `<div class="ow-heat-cell ow-heat-empty"></div>`;
      if (s.rejected) {
        return `<div class="ow-heat-cell ow-heat-rejected" title="t_min=${tMinValue}, t_max=${tMaxValue}: rejected (fewer than 80% of sampled rests fitted)"></div>`;
      }
      const t = domainMax > domainMin ? (s.meanAdjR2 - domainMin) / (domainMax - domainMin) : 1;
      return `<div class="ow-heat-cell" style="background:${sequentialColor(t)}" title="t_min=${tMinValue}, t_max=${tMaxValue}: mean adj R²=${s.meanAdjR2.toPrecision(4)}"></div>`;
    }),
  );

  return `
    <div class="ow-heatmap">
      <p class="hint">Mean adj R² over the candidate grid (rows: t_min ${tMins[0]}–${tMins[tMins.length - 1]} s top→bottom; columns: t_max ${tMaxs[0]}–${tMaxs[tMaxs.length - 1]} s left→right; hatched = rejected)</p>
      <div class="ow-heat-grid" style="grid-template-columns: repeat(${tMaxs.length}, 1fr);">${cells.join("")}</div>
    </div>
  `;
}

function fmtOrDash(v: number): string {
  return Number.isFinite(v) ? v.toPrecision(3) : "—";
}

function escapeHtml(s: string): string {
  const div = document.createElement("div");
  div.textContent = s;
  return div.innerHTML;
}
