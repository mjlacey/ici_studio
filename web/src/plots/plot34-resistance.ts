// §11.3: Plots 3 and 4 -- R vs x and k vs x, available once Stage A has
// run. Shared factory; x-column selectable (any built-in numeric analysis
// column -- no extras, §7.9 is deferred), state-coloured scatter, error
// bars (§7.7's R_err/k_err), one smoothing line per Stage B smoothing
// group, §9's flagged (muted) / excluded (hollow) point styling, and
// click-to-select-rest (loads the clicked point into Plot 2).

import uPlot from "uplot";
import "uplot/dist/uPlot.min.css";
import { normalizeValue, normalizedLabel } from "../analysis/area-normalization";
import { applyAbsValue, absValueLabel } from "../analysis/absolute-value-display";
import { computeRowFlags, isRowExcluded, isRowFlagged } from "../analysis/qc";
import { activeDataset, store, updateDataset } from "../state";
import type { AnalysisTable, Dataset, StageAResult, StageBResult } from "../types";
import { applyAxisRange, axisControlsHtml, defaultAxisRange, robustRange, setAxisInputs, wireAxisControls } from "./axis-controls";
import { errorBarPlugin, type ErrorBarPoint } from "./error-bar-plugin";
import { naturalOrderPathBuilder } from "./natural-order-path";
import { pngExportControlHtml, wirePngExportControl } from "./png-export";

interface XColumn {
  key: string;
  label: string;
  get: (t: AnalysisTable) => number[];
}

const X_COLUMNS: XColumn[] = [
  { key: "q", label: "Q", get: (t) => t.q },
  { key: "t", label: "t", get: (t) => t.t },
  { key: "stepT", label: "step.t", get: (t) => t.stepT },
  { key: "e", label: "E", get: (t) => t.e },
  { key: "i", label: "I", get: (t) => t.i },
  { key: "e0", label: "E0", get: (t) => t.e0 },
  { key: "s", label: "s", get: (t) => t.s },
  { key: "i0", label: "I0", get: (t) => t.i0 },
  { key: "cycN", label: "cyc.n", get: (t) => t.cycN },
];

const GROUP_PALETTE = ["#16a34a", "#7c3aed", "#0891b2", "#ea580c", "#db2777", "#65a30d", "#0284c7", "#c026d3"];
const CHARGE_COLOR = "#2563eb";
const DISCHARGE_COLOR = "#f97316";
const SURFACE_COLOR = "#ffffff"; // matches --bg -- "hollow" = fill the marker with the page background.
const CLICK_HIT_RADIUS_PX = 10;

function nullIfNotFinite(v: number | null): number | null {
  return v !== null && Number.isFinite(v) ? v : null;
}

function mutedColor(hex: string): string {
  const n = parseInt(hex.slice(1), 16);
  const r = (n >> 16) & 255;
  const g = (n >> 8) & 255;
  const b = n & 255;
  return `rgba(${r},${g},${b},0.35)`;
}

type RowCategory = "normal" | "flagged" | "excluded";

function classifyRow(dataset: Dataset, table: AnalysisTable, i: number): RowCategory {
  if (isRowExcluded(table, i, dataset.qcConfig, dataset.manualExclusions)) return "excluded";
  if (isRowFlagged(computeRowFlags(table, i, dataset.qcConfig))) return "flagged";
  return "normal";
}

interface ClickableRow {
  x: number;
  y: number;
  groupId: number;
  rest: number;
}

/**
 * §9 click-through: uPlot has no built-in per-point click for scatter series, so this hit-tests
 * the nearest row in CSS-pixel space (not canvas-pixel -- the exact mismatch milestone 7's drag
 * plugin hit once). Listens in the *capture* phase on `u.root` (an ancestor of `.u-over`), not a
 * plain bubble-phase listener on `.u-over` itself: uPlot's own cursor/drag handling binds its own
 * "click" listener directly on `.u-over` (its default `cursor.drag.click` calls
 * `e.stopImmediatePropagation()`, to swallow the spurious click a drag-to-zoom gesture produces),
 * and that listener is registered before this plugin's -- since same-target listeners run in
 * registration order regardless of capture flag, a listener added on `.u-over` itself would never
 * fire. A capture-phase listener on an ancestor runs during the top-down capture pass, before the
 * event reaches `.u-over`'s target phase, so it's unaffected by that later stopImmediatePropagation.
 */
function clickSelectPlugin(getRows: () => ClickableRow[], onSelect: (groupId: number, rest: number) => void): uPlot.Plugin {
  let cleanup: (() => void) | null = null;
  return {
    hooks: {
      ready: [
        (u: uPlot) => {
          const over = u.over;
          const handler = (e: MouseEvent): void => {
            const rect = over.getBoundingClientRect();
            const px = e.clientX - rect.left;
            const py = e.clientY - rect.top;
            let best: ClickableRow | null = null;
            let bestDist = Infinity;
            for (const row of getRows()) {
              const rx = u.valToPos(row.x, "x", false);
              const ry = u.valToPos(row.y, "y", false);
              const dist = Math.hypot(rx - px, ry - py);
              if (dist < bestDist) {
                bestDist = dist;
                best = row;
              }
            }
            if (best && bestDist <= CLICK_HIT_RADIUS_PX) onSelect(best.groupId, best.rest);
          };
          u.root.addEventListener("click", handler, { capture: true });
          cleanup = () => u.root.removeEventListener("click", handler, { capture: true });
        },
      ],
      destroy: [
        () => {
          cleanup?.();
          cleanup = null;
        },
      ],
    },
  };
}

export interface ResistancePlotOptions {
  valueColumn: "r" | "k";
  title: string;
  /** Unscaled unit symbol, e.g. "Ω" or "Ω·s⁻¹ᐟ²" -- a prefix (m, µ) gets
   * inserted directly before this when the user picks a display scale, so
   * §11.5's area-normalised "Ω cm²"/"Ω cm²·s⁻¹ᐟ²" composes correctly too
   * (e.g. "mΩ cm²"). */
  baseUnit: string;
}

/** R and k are typically ~1e-4-1e-3 in SI units -- small enough that
 * uPlot's default tick formatter rounds most ticks to the same "0.001"
 * (observed directly: repeated identical tick labels), not because the
 * data lacks resolution. A plain SI-prefix display-scale multiplier (not
 * persisted -- a view preference, same category as xKey/minPx) fixes
 * this without touching the underlying values used for QC/export/fits. */
const UNIT_SCALES: { label: string; prefix: string; factor: number }[] = [
  { label: "×1", prefix: "", factor: 1 },
  { label: "×1e3 (m)", prefix: "m", factor: 1e3 },
  { label: "×1e6 (µ)", prefix: "µ", factor: 1e6 },
];

export function mountResistancePlot(container: HTMLElement, opts: ResistancePlotOptions): void {
  let chart: uPlot | null = null;
  let xKey = "q";
  let showErrorBars = true;
  let minPx = 2;
  let unitScale = UNIT_SCALES[0];
  let errorPoints: ErrorBarPoint[] = [];
  let clickableRows: ClickableRow[] = [];
  let xRange = defaultAxisRange();
  let yRange = defaultAxisRange();
  let pendingRangeReset = false;

  let lastStageAResult: StageAResult | null = null;
  let lastStageBResult: StageBResult | null = null;
  let lastXKey = "";
  let lastQcKey = "";
  let lastRangeKey = "";
  let lastNormKey = "";

  const wrapper = document.createElement("section");
  wrapper.className = "panel plot-panel";
  wrapper.style.display = "none";
  wrapper.innerHTML = `
    <h2>
      <span>${opts.title}</span>
      ${pngExportControlHtml()}
    </h2>
    <div class="plot-controls">
      <label>x-axis <select data-x-col></select></label>
      <label class="toggle"><input type="checkbox" data-error-toggle checked /> error bars</label>
      <label>min px <input type="number" min="0" step="1" data-min-px value="2" /></label>
      <label>units <select data-unit-scale>${UNIT_SCALES.map((u) => `<option value="${u.factor}">${u.label}</option>`).join("")}</select></label>
      ${axisControlsHtml("x", xRange)}
      ${axisControlsHtml("y", yRange)}
    </div>
    <div class="plot-host"></div>
  `;
  container.appendChild(wrapper);

  const host = wrapper.querySelector<HTMLDivElement>(".plot-host")!;
  const xSelect = wrapper.querySelector<HTMLSelectElement>("[data-x-col]")!;
  xSelect.innerHTML = X_COLUMNS.map((c) => `<option value="${c.key}">${c.label}</option>`).join("");
  xSelect.value = xKey;
  xSelect.addEventListener("change", () => {
    xKey = xSelect.value;
    render();
  });

  const errorToggle = wrapper.querySelector<HTMLInputElement>("[data-error-toggle]")!;
  errorToggle.addEventListener("change", () => {
    showErrorBars = errorToggle.checked;
    chart?.redraw();
  });

  const unitScaleSelect = wrapper.querySelector<HTMLSelectElement>("[data-unit-scale]")!;
  unitScaleSelect.addEventListener("change", () => {
    const next = UNIT_SCALES.find((u) => u.factor === Number(unitScaleSelect.value)) ?? UNIT_SCALES[0];
    // The y-range (whether robust-auto-computed or user-typed) was set in
    // terms of the *previous* display scale's values -- carry it forward by
    // the same ratio rather than leaving it stale, or almost every point
    // ends up clipped outside the old (now-mismatched) bounds.
    const ratio = next.factor / unitScale.factor;
    yRange = { ...yRange, min: yRange.min === null ? null : yRange.min * ratio, max: yRange.max === null ? null : yRange.max * ratio };
    unitScale = next;
    setAxisInputs(wrapper, "y", yRange);
    render();
  });

  const minPxInput = wrapper.querySelector<HTMLInputElement>("[data-min-px]")!;
  minPxInput.addEventListener("change", () => {
    minPx = Math.max(0, Number(minPxInput.value) || 0);
    chart?.redraw();
  });

  wirePngExportControl(
    wrapper,
    () => chart,
    () => `${activeDataset(store.get())?.filename ?? "export"}_${opts.valueColumn}_vs_${xKey}.png`,
  );

  wireAxisControls(
    wrapper,
    (axis) => (axis === "x" ? xRange : yRange),
    (axis, next) => {
      if (axis === "x") xRange = next;
      else yRange = next;
    },
    {
      onRangeChange: (axis) => {
        if (chart) applyAxisRange(chart, axis, axis === "x" ? xRange : yRange);
      },
      onLogChange: () => render(),
    },
  );

  function buildChart(dataset: Dataset, table: AnalysisTable, stageB: StageBResult | null): void {
    chart?.destroy();

    const xGetter = X_COLUMNS.find((c) => c.key === xKey)!.get;
    // §11.5: normalize *before* sorting/deriving anything downstream (errorPoints,
    // clickableRows, the per-state/class series) so a click's hit-test position
    // always matches what's actually drawn.
    const xRaw = xGetter(table).map((v) => applyAbsValue(xKey, normalizeValue(xKey, v, dataset), dataset));
    const n = xRaw.length;
    const order = Array.from({ length: n }, (_, i) => i).sort((a, b) => xRaw[a] - xRaw[b]);
    const x = order.map((i) => xRaw[i]);

    const columnKey = opts.valueColumn; // "r" | "k"
    const errColumnKey = opts.valueColumn === "r" ? "rErr" : "kErr";
    const smoothColumnKey = opts.valueColumn === "r" ? "rSmooth" : "kSmooth";
    const valueArr = (opts.valueColumn === "r" ? table.r : table.k).map((v) => normalizeValue(columnKey, v, dataset) * unitScale.factor);
    const errArr = (opts.valueColumn === "r" ? table.rErr : table.kErr).map((v) => normalizeValue(errColumnKey, v, dataset) * unitScale.factor);
    const smoothArrRaw = stageB ? (opts.valueColumn === "r" ? stageB.rSmooth : stageB.kSmooth) : null;
    const smoothArr = smoothArrRaw ? smoothArrRaw.map((v) => (v === null ? null : normalizeValue(smoothColumnKey, v, dataset) * unitScale.factor)) : null;

    if (pendingRangeReset) {
      pendingRangeReset = false;
      const robust = robustRange([...valueArr, ...(smoothArr ?? [])].filter((v): v is number => v !== null), { clampMinToZero: true });
      yRange = robust ? { min: robust.min, max: robust.max, log: false } : defaultAxisRange();
      setAxisInputs(wrapper, "y", yRange);
    }

    const rowClass = order.map((i) => classifyRow(dataset, table, i));
    const byStateAndClass = (state: "charge" | "discharge", cls: RowCategory): (number | null)[] =>
      order.map((i, idx) => (table.state[i] === state && rowClass[idx] === cls ? nullIfNotFinite(valueArr[i]) : null));

    errorPoints = order
      .map((i, idx) => ({ x: x[idx], y: valueArr[i], err: errArr[i] }))
      .filter((p) => Number.isFinite(p.y) && Number.isFinite(p.err));

    clickableRows = order
      .map((i, idx) => ({ x: x[idx], y: valueArr[i], groupId: table.groupId[i], rest: table.rest[i] }))
      .filter((p) => Number.isFinite(p.y));

    const data: (number | null)[][] = [
      x,
      byStateAndClass("charge", "normal"),
      byStateAndClass("charge", "flagged"),
      byStateAndClass("charge", "excluded"),
      byStateAndClass("discharge", "normal"),
      byStateAndClass("discharge", "flagged"),
      byStateAndClass("discharge", "excluded"),
    ];
    const pointSeries = (label: string, color: string, hollow: boolean): uPlot.Series => ({
      label,
      scale: "y",
      paths: () => null,
      points: { show: true, size: 5, fill: hollow ? SURFACE_COLOR : color, stroke: color },
    });
    const xColLabel = absValueLabel(xKey, normalizedLabel(xKey, X_COLUMNS.find((c) => c.key === xKey)!.label, dataset), dataset);
    const baseUnit = dataset.normalizeToArea ? (opts.valueColumn === "r" ? "Ω cm²" : "Ω cm²·s⁻¹ᐟ²") : opts.baseUnit;
    const yAxisLabel = `${opts.valueColumn === "r" ? "R" : "k"} (${unitScale.prefix}${baseUnit})`;

    const series: uPlot.Series[] = [
      { label: xColLabel },
      pointSeries("charge", CHARGE_COLOR, false),
      pointSeries("charge (flagged)", mutedColor(CHARGE_COLOR), false),
      pointSeries("charge (excluded)", CHARGE_COLOR, true),
      pointSeries("discharge", DISCHARGE_COLOR, false),
      pointSeries("discharge (flagged)", mutedColor(DISCHARGE_COLOR), false),
      pointSeries("discharge (excluded)", DISCHARGE_COLOR, true),
    ];

    if (stageB && smoothArr) {
      for (const [groupIdx, group] of stageB.groups.entries()) {
        const color = GROUP_PALETTE[groupIdx % GROUP_PALETTE.length];
        const y = order.map((i) => (stageB.rowGroupKey[i] === group.key ? nullIfNotFinite(smoothArr[i]) : null));
        data.push(y);
        // Stage B always smooths against Q, regardless of which column is
        // currently displayed on the x-axis -- draw the stroke by walking
        // the group's own points in Q order (its true independent
        // variable), not in whatever order they land in the shared,
        // display-x-sorted array. Two smoothing groups whose Q domains
        // overlap (e.g. after an "end" Q anchor) interleave heavily in
        // that shared array; uPlot's default path builder handles the
        // resulting long runs of alternating nulls far more sparsely than
        // the data actually supports (confirmed: <1% of the expected
        // stroke pixels), even though every point is present and finite.
        const qOrderedPositions = order
          .map((i, k) => (stageB.rowGroupKey[i] === group.key && y[k] !== null ? k : -1))
          .filter((k) => k !== -1)
          .sort((a, b) => table.q[order[a]] - table.q[order[b]]);
        // Color alone isn't enough to tell two smoothing-group lines apart
        // when they're nearly coincident in value (e.g. charge and
        // discharge k(Q) can be almost identical) -- especially once an
        // "end"-anchored Q makes their domains overlap instead of sitting
        // on opposite sides of zero, one line can otherwise completely
        // paint over the other. Alternate solid/dashed by group index so
        // an occluded line still shows through the gaps.
        const dash = groupIdx % 2 === 1 ? [6, 4] : undefined;
        series.push({
          label: group.label,
          scale: "y",
          stroke: color,
          width: 2,
          dash,
          paths: naturalOrderPathBuilder(qOrderedPositions, x, y),
          points: { show: false },
        });
      }
    }
    const uOpts: uPlot.Options = {
      width: host.clientWidth || 800,
      height: 280,
      scales: { x: { time: false, distr: xRange.log ? 3 : 1 }, y: { distr: yRange.log ? 3 : 1 } },
      axes: [{ label: xColLabel }, { label: yAxisLabel }],
      series,
      plugins: [
        errorBarPlugin({
          getEnabled: () => showErrorBars,
          getMinPx: () => minPx,
          getPoints: () => errorPoints,
        }),
        clickSelectPlugin(
          () => clickableRows,
          (groupId, rest) => updateDataset(dataset.id, (d) => ({ ...d, selectedGroupId: groupId, selectedRestId: rest })),
        ),
      ],
    };
    // uPlot's AlignedData type is a tuple (x: number[], ...y: (number|null)[][])
    // that TS only recognises from an inline literal, not a pre-typed
    // variable -- `data` is built incrementally (group lines pushed in a
    // loop) so it can't be inlined; the runtime shape is exactly right.
    chart = new uPlot(uOpts, data as unknown as uPlot.AlignedData, host);
    applyAxisRange(chart, "x", xRange);
    applyAxisRange(chart, "y", yRange);
  }

  function render(): void {
    const dataset = activeDataset(store.get());
    const result = dataset?.stageAResult ?? null;
    if (!dataset || !result) {
      wrapper.style.display = "none";
      lastStageAResult = null;
      return;
    }
    wrapper.style.display = "";

    const stageB = dataset.stageBResult;
    const qcKey = `${JSON.stringify(dataset.qcConfig)}:${JSON.stringify(dataset.manualExclusions)}`;
    const rangeKey = `${xRange.log}:${yRange.log}`;
    const normKey = `${dataset.normalizeToArea}:${dataset.electrodeAreaCm2}:${dataset.absoluteQ}:${unitScale.factor}`;
    if (
      result === lastStageAResult &&
      stageB === lastStageBResult &&
      xKey === lastXKey &&
      qcKey === lastQcKey &&
      rangeKey === lastRangeKey &&
      normKey === lastNormKey
    )
      return;
    // A genuinely new Stage A run (not just an unrelated re-render) means
    // whatever range the user set earlier is describing stale data --
    // e.g. re-fitting after applying the §7.1 threshold-suggestion banner
    // can go from "almost nothing survived" to the full Q range, and a
    // manual zoom from investigating the former would otherwise silently
    // clip the latter, looking like the data itself is still broken.
    if (result !== lastStageAResult) {
      xRange = defaultAxisRange();
      // yRange is recomputed inside buildChart, once the actual R/k values
      // are in hand, to a robust (outlier-resistant) range rather than
      // plain full auto-fit -- see `robustRange`.
      pendingRangeReset = true;
      setAxisInputs(wrapper, "x", xRange);
    }
    lastStageAResult = result;
    lastStageBResult = stageB;
    lastXKey = xKey;
    lastQcKey = qcKey;
    lastRangeKey = rangeKey;
    lastNormKey = normKey;

    buildChart(dataset, result.analysisTable, stageB);
  }

  store.subscribe(render);
  render();
}
