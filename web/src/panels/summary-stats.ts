// §11.6: summary statistics, per smoothing group -- n, median/IQR of R and
// k, median adj_r2, median n_pts, R/k at the start/mid/end of the group's Q
// range, Q range, and the group's fitted λ/EDF per smoother. Regression
// failures (§7.7) are reported once at the dataset level, not attributed
// per smoothing group -- Stage A's failures are keyed by segmentation
// group/rest, not the (possibly coarser or differently-scoped) smoothing
// key, so a per-group breakdown isn't cleanly derivable client-side. This
// is a distinct count from §9's QC flagged/excluded (the qc-panel) --
// rests that never produced a fit at all, vs. fitted rests QC considers
// questionable. The actual per-group numbers come from `computeSummaryStats`
// (analysis/summary-stats.ts), shared with §12.3's run-report export.

import { computeSummaryStats } from "../analysis/summary-stats";
import { normalizeValue, normalizedLabel } from "../analysis/area-normalization";
import { applyAbsValueRange, absValueLabel } from "../analysis/absolute-value-display";
import { activeDataset, store } from "../state";
import type { Dataset, SplineDiagnostics, StageAResult, StageBResult, SummaryStatsRow } from "../types";

export function mountSummaryStats(container: HTMLElement): void {
  let lastResult: StageAResult | null = null;
  let lastStageB: StageBResult | null = null;
  let lastNormKey = "";

  function render(): void {
    const dataset = activeDataset(store.get());
    const result = dataset?.stageAResult ?? null;
    if (!dataset || !result) {
      container.innerHTML = "";
      lastResult = null;
      return;
    }

    const stageB = dataset.stageBResult;
    const normKey = `${dataset.normalizeToArea}:${dataset.electrodeAreaCm2}:${dataset.absoluteQ}`;
    if (result === lastResult && stageB === lastStageB && normKey === lastNormKey) return;
    lastResult = result;
    lastStageB = stageB;
    lastNormKey = normKey;

    container.innerHTML = renderPanel(dataset, result, stageB);
  }

  store.subscribe(render);
  render();
}

function renderPanel(dataset: Dataset, result: StageAResult, stageB: StageBResult | null): string {
  const totalRests = result.segmentation.totalRests;
  const failCount = result.regressionFailures.length;
  const fittedCount = totalRests - failCount;

  const header = `
    <h2>Summary statistics</h2>
    <p class="hint">${fittedCount} of ${totalRests} rests fitted; ${failCount} excluded (regression failure -- see the quality control panel for flagged/excluded counts).</p>
  `;

  if (!stageB) {
    return `<section class="panel">${header}<p class="hint">Waiting for Stage B (smoothing) to finish…</p></section>`;
  }

  const rLabel = normalizedLabel("r", "R", dataset);
  const kLabel = normalizedLabel("k", "k", dataset);
  const qLabel = absValueLabel("q", normalizedLabel("q", "Q", dataset), dataset);

  const statsRows = computeSummaryStats(dataset);
  const rows = statsRows.map((row: SummaryStatsRow) => {
    const normR = (v: number): number => normalizeValue("r", v, dataset);
    const normK = (v: number): number => normalizeValue("k", v, dataset);

    return `
      <tr>
        <th>${escapeHtml(row.groupLabel)}</th>
        <td>${row.n}</td>
        <td>${fmt(normR(row.rMedian))} [${fmt(normR(row.rQ1))}, ${fmt(normR(row.rQ3))}]</td>
        <td>${fmt(normK(row.kMedian))} [${fmt(normK(row.kQ1))}, ${fmt(normK(row.kQ3))}]</td>
        <td>${fmt(row.medianAdjR2)}</td>
        <td>${fmt(row.medianNPts, 0)}</td>
        <td>${fmtTriple(normR(row.rStart), normR(row.rMid), normR(row.rEnd))}</td>
        <td>${fmtTriple(normK(row.kStart), normK(row.kMid), normK(row.kEnd))}</td>
        <td>${row.qMin !== null && row.qMax !== null ? formatQRange(row, dataset) : "—"}</td>
        <td>${splineFmt(row.e0Spline)}</td>
        <td>${splineFmt(row.kSpline)}</td>
        <td>${splineFmt(row.rSpline)}</td>
      </tr>
    `;
  });

  return `
    <section class="panel summary-stats-panel">
      ${header}
      <div class="table-scroll-x">
        <table class="kv-table summary-table">
          <thead>
            <tr>
              <th>Group</th><th>n</th><th>${escapeHtml(rLabel)} (median [IQR])</th><th>${escapeHtml(kLabel)} (median [IQR])</th>
              <th>median adj.R²</th><th>median n_pts</th><th>${escapeHtml(rLabel)} (start/mid/end)</th><th>${escapeHtml(kLabel)} (start/mid/end)</th>
              <th>${escapeHtml(qLabel)} range</th><th>E0 λ/edf</th><th>k λ/edf</th><th>R λ/edf</th>
            </tr>
          </thead>
          <tbody>${rows.join("")}</tbody>
        </table>
      </div>
    </section>
  `;
}

function fmt(v: number, digits = 4): string {
  if (!Number.isFinite(v)) return "—";
  return digits === 0 ? String(Math.round(v)) : v.toPrecision(digits);
}

function fmtTriple(start: number, mid: number, end: number): string {
  return `${fmt(start, 3)} / ${fmt(mid, 3)} / ${fmt(end, 3)}`;
}

function formatQRange(row: SummaryStatsRow, dataset: Dataset): string {
  if (row.qMin === null || row.qMax === null) return "—";
  const areaMin = normalizeValue("q", row.qMin, dataset);
  const areaMax = normalizeValue("q", row.qMax, dataset);
  const [lo, hi] = applyAbsValueRange("q", areaMin, areaMax, dataset);
  return `${fmt(lo)} – ${fmt(hi)}`;
}

function splineFmt(d: SplineDiagnostics | null): string {
  return d ? `${d.lambda.toPrecision(3)} / ${d.edf.toFixed(1)}` : "NA";
}

function escapeHtml(s: string): string {
  const div = document.createElement("div");
  div.textContent = s;
  return div.innerHTML;
}
