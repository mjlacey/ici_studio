// §11.4: one user-defined additional plot panel. Self-subscribing to the
// store exactly like mountResistancePlot -- looks up its own config by id
// on every tick, dedupes against a config key, only rebuilds on real
// change -- so an unrelated keystroke elsewhere in the app doesn't force
// every additional plot to reconsider a remount. No state/QC colour-split,
// no click-to-select -- §11.4's text is plain compared to §11.3's itemized
// list, those are named explicitly for Plots 3/4 only. Any of its x/y/
// error columns can be r/k/q/rSmooth/kSmooth/dVdQ/dQdV, so §11.5's area
// normalisation applies here exactly as it does on Plot3/4.

import uPlot from "uplot";
import "uplot/dist/uPlot.min.css";
import { applyColumnNormalization, normalizedLabel } from "../analysis/area-normalization";
import { applyColumnAbsValue, absValueLabel } from "../analysis/absolute-value-display";
import { activeDataset, store, updateDataset } from "../state";
import type { AdditionalPlotConfig, AnalysisTable, Dataset, PlotColumnKey, StageBResult } from "../types";
import { applyAxisRange, axisControlsHtml, defaultAxisRange, robustRange, setAxisInputs, wireAxisControls } from "./axis-controls";
import { errorBarPlugin, type ErrorBarPoint } from "./error-bar-plugin";
import { naturalOrderPathBuilder } from "./natural-order-path";
import { PLOT_COLUMNS } from "./plot-columns";
import { pngExportControlHtml, wirePngExportControl } from "./png-export";
import { chartGridColor, chartTextColor, onThemeChange } from "../theme";

const SERIES_COLOR = "#2563eb";
const GROUP_PALETTE = ["#2563eb", "#f97316", "#16a34a", "#7c3aed", "#0891b2", "#ea580c", "#db2777", "#65a30d"];

function nullIfNotFinite(v: number | null): number | null {
  return v !== null && Number.isFinite(v) ? v : null;
}

/** Mirrors Stage B's own smoothing grouping when available (cyc.n + state,
 * matching Plot 3/4's own line grouping) so a line series never has to
 * connect points across groups -- falls back to a plain charge/discharge
 * split when Stage B hasn't run yet. */
function lineGroupsFor(table: AnalysisTable, stageB: StageBResult | null): { key: string; label: string }[] {
  if (stageB) return stageB.groups.map((g) => ({ key: g.key, label: g.label }));
  const seen = new Set<string>();
  const groups: { key: string; label: string }[] = [];
  for (const state of table.state) {
    if (!seen.has(state)) {
      seen.add(state);
      groups.push({ key: state, label: state });
    }
  }
  return groups;
}

function lineGroupKeyFor(table: AnalysisTable, stageB: StageBResult | null, i: number): string {
  return stageB ? stageB.rowGroupKey[i] : table.state[i];
}

function columnOptionsHtml(selected: string, includeNone: boolean): string {
  const none = includeNone ? `<option value="" ${selected === "" ? "selected" : ""}>none</option>` : "";
  return none + PLOT_COLUMNS.map((c) => `<option value="${c.key}" ${c.key === selected ? "selected" : ""}>${c.label}</option>`).join("");
}

export interface AdditionalPlotCallbacks {
  onRemove: () => void;
  onDragStart: () => void;
}

export function mountAdditionalPlot(container: HTMLElement, plotId: string, callbacks: AdditionalPlotCallbacks): { destroy: () => void } {
  let chart: uPlot | null = null;
  let errorPoints: ErrorBarPoint[] = [];
  let xRange = defaultAxisRange();
  let yRange = defaultAxisRange();
  let lastXColumn: PlotColumnKey | null = null;
  let lastYColumn: PlotColumnKey | null = null;
  let pendingRangeReset = false;

  let lastConfigKey = "";
  let lastStageAResult: unknown = null;
  let lastStageBResult: unknown = null;
  let lastRangeKey = "";
  let lastNormKey = "";

  const wrapper = document.createElement("section");
  wrapper.className = "panel plot-panel additional-plot-panel";
  container.appendChild(wrapper);

  function config(dataset: Dataset): AdditionalPlotConfig | null {
    return dataset.additionalPlots.find((p) => p.id === plotId) ?? null;
  }

  function renderHeader(cfg: AdditionalPlotConfig): void {
    wrapper.innerHTML = `
      <h2>
        <span class="drag-handle" draggable="true" title="Drag to reorder">⠿</span>
        <label>x <select data-field="xColumn">${columnOptionsHtml(cfg.xColumn, false)}</select></label>
        <label>y <select data-field="yColumn">${columnOptionsHtml(cfg.yColumn, false)}</select></label>
        <label>mode
          <select data-field="mode">
            <option value="points" ${cfg.mode === "points" ? "selected" : ""}>points</option>
            <option value="line" ${cfg.mode === "line" ? "selected" : ""}>line</option>
            <option value="both" ${cfg.mode === "both" ? "selected" : ""}>both</option>
          </select>
        </label>
        <label>error <select data-field="errorColumn">${columnOptionsHtml(cfg.errorColumn ?? "", true)}</select></label>
        <button type="button" class="btn-small" data-remove title="Remove plot">✕</button>
        ${pngExportControlHtml()}
      </h2>
      <div class="plot-controls">
        ${axisControlsHtml("x", xRange)}
        ${axisControlsHtml("y", yRange)}
      </div>
      <div class="plot-host"></div>
    `;

    wrapper.querySelector<HTMLElement>(".drag-handle")!.addEventListener("dragstart", (e) => {
      e.dataTransfer?.setData("text/plain", plotId);
      if (e.dataTransfer) e.dataTransfer.effectAllowed = "move";
      callbacks.onDragStart();
    });
    wrapper.querySelector<HTMLButtonElement>("[data-remove]")!.addEventListener("click", callbacks.onRemove);

    function updateConfig(updater: (p: AdditionalPlotConfig) => AdditionalPlotConfig): void {
      const dataset = activeDataset(store.get());
      if (!dataset) return;
      updateDataset(dataset.id, (d) => ({
        ...d,
        additionalPlots: d.additionalPlots.map((p) => (p.id === plotId ? updater(p) : p)),
      }));
    }

    wrapper.querySelector<HTMLSelectElement>('[data-field="xColumn"]')!.addEventListener("change", (e) => {
      const value = (e.target as HTMLSelectElement).value as PlotColumnKey;
      updateConfig((p) => ({ ...p, xColumn: value }));
    });
    wrapper.querySelector<HTMLSelectElement>('[data-field="yColumn"]')!.addEventListener("change", (e) => {
      const value = (e.target as HTMLSelectElement).value as PlotColumnKey;
      updateConfig((p) => ({ ...p, yColumn: value }));
    });
    wrapper.querySelector<HTMLSelectElement>('[data-field="mode"]')!.addEventListener("change", (e) => {
      const value = (e.target as HTMLSelectElement).value as AdditionalPlotConfig["mode"];
      updateConfig((p) => ({ ...p, mode: value }));
    });
    wrapper.querySelector<HTMLSelectElement>('[data-field="errorColumn"]')!.addEventListener("change", (e) => {
      const raw = (e.target as HTMLSelectElement).value;
      updateConfig((p) => ({ ...p, errorColumn: raw === "" ? null : (raw as PlotColumnKey) }));
    });

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

    wirePngExportControl(
      wrapper,
      () => chart,
      () => `${activeDataset(store.get())?.filename ?? "export"}_${cfg.yColumn}_vs_${cfg.xColumn}.png`,
    );
  }

  function buildChart(dataset: Dataset, cfg: AdditionalPlotConfig, table: AnalysisTable, stageB: StageBResult | null): void {
    // A manual axis range set up for the *previous* column selection is
    // meaningless (often actively misleading -- e.g. a y-range left over
    // from a ±400 dQ/dV plot silently squashing a newly-selected E column's
    // 2.8-4.3 values into an invisible flat line) once x or y actually
    // changes to a different column. Reset before rendering the header so
    // the min/max inputs themselves reflect the reset too.
    if (cfg.xColumn !== lastXColumn || cfg.yColumn !== lastYColumn) {
      // Ranges are recomputed below, once the actual column values are in
      // hand, to a robust (outlier-resistant) range rather than plain
      // full auto-fit -- see `robustRange`.
      pendingRangeReset = true;
      lastXColumn = cfg.xColumn;
      lastYColumn = cfg.yColumn;
    }
    renderHeader(cfg);
    chart?.destroy();
    const host = wrapper.querySelector<HTMLDivElement>(".plot-host")!;

    const xCol = PLOT_COLUMNS.find((c) => c.key === cfg.xColumn)!;
    const yCol = PLOT_COLUMNS.find((c) => c.key === cfg.yColumn)!;
    const xRaw = applyColumnAbsValue(cfg.xColumn, applyColumnNormalization(cfg.xColumn, xCol.get(table, stageB), dataset), dataset);
    const yRaw = applyColumnAbsValue(cfg.yColumn, applyColumnNormalization(cfg.yColumn, yCol.get(table, stageB), dataset), dataset);

    const n = table.rest.length;
    const order = Array.from({ length: n }, (_, i) => i)
      .filter((i) => xRaw[i] !== null && Number.isFinite(xRaw[i]))
      .sort((a, b) => (xRaw[a] as number) - (xRaw[b] as number));
    const x = order.map((i) => xRaw[i] as number);
    const y = order.map((i) => yRaw[i]);

    if (pendingRangeReset) {
      pendingRangeReset = false;
      const robustX = robustRange(x);
      const robustY = robustRange(y.filter((v): v is number => v !== null));
      xRange = robustX ? { min: robustX.min, max: robustX.max, log: false } : defaultAxisRange();
      yRange = robustY ? { min: robustY.min, max: robustY.max, log: false } : defaultAxisRange();
      setAxisInputs(wrapper, "x", xRange);
      setAxisInputs(wrapper, "y", yRange);
    }

    errorPoints = [];
    if (cfg.errorColumn) {
      const errCol = PLOT_COLUMNS.find((c) => c.key === cfg.errorColumn);
      const errRaw = errCol ? applyColumnAbsValue(cfg.errorColumn, applyColumnNormalization(cfg.errorColumn, errCol.get(table, stageB), dataset), dataset) : [];
      errorPoints = order
        .map((i, idx) => ({ x: x[idx], y: y[idx], err: errRaw[i] }))
        .filter((p): p is ErrorBarPoint => p.y !== null && Number.isFinite(p.y) && p.err !== null && Number.isFinite(p.err));
    }

    const showLine = cfg.mode !== "points";
    const showPoints = cfg.mode !== "line";
    const xLabel = absValueLabel(cfg.xColumn, normalizedLabel(cfg.xColumn, xCol.label, dataset), dataset);
    const yLabel = absValueLabel(cfg.yColumn, normalizedLabel(cfg.yColumn, yCol.label, dataset), dataset);

    const data: (number | null)[][] = [x];
    const series: uPlot.Series[] = [{ label: xLabel }];

    if (showLine) {
      // A single line series connecting points in x order zigzags wherever
      // x isn't a genuine function of the underlying data -- e.g. Q shared
      // between charge and discharge branches. Split into one line series
      // per Stage B smoothing group (cyc.n + state, matching Plot 3/4)
      // instead, each a null-elsewhere subset of the same x-sorted `order`
      // -- a subsequence of a sorted sequence is still sorted, so each
      // group's own line stays monotonic in x with no zigzag.
      const groups = lineGroupsFor(table, stageB);
      for (const [idx, group] of groups.entries()) {
        const color = GROUP_PALETTE[idx % GROUP_PALETTE.length];
        const groupY = order.map((i) => (lineGroupKeyFor(table, stageB, i) === group.key ? nullIfNotFinite(yRaw[i]) : null));
        data.push(groupY);
        const qOrderedPositions = order
          .map((i, k) => (lineGroupKeyFor(table, stageB, i) === group.key && groupY[k] !== null ? k : -1))
          .filter((k) => k !== -1)
          .sort((a, b) => table.q[order[a]] - table.q[order[b]]);
        series.push({
          label: groups.length > 1 ? group.label : yLabel,
          scale: "y",
          stroke: color,
          width: 2,
          paths: naturalOrderPathBuilder(qOrderedPositions, x, groupY),
          points: { show: showPoints, size: 5, fill: color, stroke: color },
        });
      }
    } else {
      data.push(y);
      series.push({
        label: yLabel,
        scale: "y",
        stroke: SERIES_COLOR,
        width: 2,
        paths: () => null,
        points: { show: true, size: 5, fill: SERIES_COLOR, stroke: SERIES_COLOR },
      });
    }

    const uOpts: uPlot.Options = {
      width: host.clientWidth || 800,
      height: 260,
      scales: { x: { time: false, distr: xRange.log ? 3 : 1 }, y: { distr: yRange.log ? 3 : 1 } },
      axes: [
        { label: xLabel, stroke: chartTextColor, grid: { stroke: chartGridColor }, ticks: { stroke: chartGridColor } },
        { label: yLabel, stroke: chartTextColor, grid: { stroke: chartGridColor }, ticks: { stroke: chartGridColor } },
      ],
      series,
      plugins: cfg.errorColumn ? [errorBarPlugin({ getEnabled: () => true, getMinPx: () => 2, getPoints: () => errorPoints })] : [],
    };
    chart = new uPlot(uOpts, data as unknown as uPlot.AlignedData, host);
    applyAxisRange(chart, "x", xRange);
    applyAxisRange(chart, "y", yRange);
  }

  function render(): void {
    const dataset = activeDataset(store.get());
    const cfg = dataset ? config(dataset) : null;
    const result = dataset?.stageAResult ?? null;
    if (!dataset || !cfg || !result) return;

    const stageB = dataset.stageBResult;
    const configKey = JSON.stringify(cfg);
    const rangeKey = `${xRange.log}:${yRange.log}`;
    const normKey = `${dataset.normalizeToArea}:${dataset.electrodeAreaCm2}:${dataset.absoluteQ}:${dataset.absoluteDVDQ}`;
    if (
      result === lastStageAResult &&
      stageB === lastStageBResult &&
      configKey === lastConfigKey &&
      rangeKey === lastRangeKey &&
      normKey === lastNormKey
    )
      return;
    lastStageAResult = result;
    lastStageBResult = stageB;
    lastConfigKey = configKey;
    lastRangeKey = rangeKey;
    lastNormKey = normKey;

    buildChart(dataset, cfg, result.analysisTable, stageB);
  }

  const unsubscribe = store.subscribe(render);
  render();
  const unsubscribeTheme = onThemeChange(() => chart?.redraw(true, true));

  return {
    destroy: () => {
      unsubscribe();
      unsubscribeTheme();
      chart?.destroy();
      wrapper.remove();
    },
  };
}
