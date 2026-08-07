// §11.1: raw E-vs-t time series, LTTB-decimated to ~4,000 points, with a
// current (I) toggle, rest shading, and a selected-rest highlight.
// Re-decimates on zoom (drag-to-select) so detail increases without ever
// shipping the full series to the DOM.

import uPlot from "uplot";
import "uplot/dist/uPlot.min.css";
import { activeDataset, store } from "../state";
import type { DataWorkerClient } from "../worker/client";
import { applyAxisRange, axisControlsHtml, defaultAxisRange, setAxisInputs, wireAxisControls } from "./axis-controls";
import { restShadingPlugin, type RestBand } from "./band-plugin";
import { pngExportControlHtml, wirePngExportControl } from "./png-export";

const TARGET_POINTS = 4000;

export function mountPlot1(container: HTMLElement, worker: DataWorkerClient): void {
  let chart: uPlot | null = null;
  let currentDatasetId: string | null = null;
  let lastSegmentation: unknown = null;
  let showCurrent = true;
  let xRange = defaultAxisRange();
  let yRange = defaultAxisRange();

  const wrapper = document.createElement("section");
  wrapper.className = "panel plot-panel";
  wrapper.style.display = "none";
  wrapper.innerHTML = `
    <h2>
      <span>Raw time series</span>
      <label class="toggle"><input type="checkbox" data-toggle-current checked /> show current (I)</label>
      ${pngExportControlHtml()}
    </h2>
    <div class="plot-controls">
      ${axisControlsHtml("x", xRange)}
      ${axisControlsHtml("y", yRange)}
    </div>
    <div class="plot-host"></div>
  `;
  container.appendChild(wrapper);

  const host = wrapper.querySelector<HTMLDivElement>(".plot-host")!;
  const toggle = wrapper.querySelector<HTMLInputElement>("[data-toggle-current]")!;
  toggle.addEventListener("change", () => {
    showCurrent = toggle.checked;
    chart?.setSeries(2, { show: showCurrent });
  });

  wirePngExportControl(
    wrapper,
    () => chart,
    () => `${activeDataset(store.get())?.filename ?? "export"}_E_vs_t.png`,
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
        if (axis === "x") void applyXRange();
        else if (chart) applyAxisRange(chart, "y", yRange);
      },
      onLogChange: () => void rebuild(),
    },
  );

  function bands(): RestBand[] {
    const dataset = activeDataset(store.get());
    const seg = dataset?.segmentation;
    if (!dataset || !seg) return [];
    return seg.restBoundaries.map((b) => ({
      tStart: b.tStart,
      tEnd: b.tEnd,
      isSelected: b.groupId === dataset.selectedGroupId && b.restId === dataset.selectedRestId,
    }));
  }

  async function redecimate(datasetId: string, xMin: number, xMax: number): Promise<void> {
    const series = await worker.getDecimatedSeries(datasetId, xMin, xMax, TARGET_POINTS);
    if (!series || !chart) return;
    chart.setData([series.t, series.e, series.i], true);
    // Only override the natural auto-fit when both bounds are finite (a drag or a fully-typed
    // range) -- an infinite bound (one axis-control input left blank) already gets the right
    // visible range for free from setData's own auto-fit over the fetched (partially-bounded) series.
    if (Number.isFinite(xMin) && Number.isFinite(xMax)) chart.setScale("x", { min: xMin, max: xMax });
    applyAxisRange(chart, "y", yRange);
  }

  /** Resolves a partial x-range (only min or only max typed) against the chart's current auto-fit before re-decimating -- avoids ever passing a literal ±Infinity into setScale. */
  async function applyXRange(): Promise<void> {
    const dataset = activeDataset(store.get());
    if (!dataset) return;
    if (xRange.min === null && xRange.max === null) {
      await loadFull(dataset.id);
      return;
    }
    const auto = chart?.scales.x;
    const min = xRange.min ?? auto?.min ?? -Infinity;
    const max = xRange.max ?? auto?.max ?? Infinity;
    await redecimate(dataset.id, min, max);
  }

  function buildChart(width: number): uPlot {
    const opts: uPlot.Options = {
      width,
      height: 300,
      scales: { x: { time: false, distr: xRange.log ? 3 : 1 }, y: { distr: yRange.log ? 3 : 1 }, i: {} },
      axes: [{ label: "t (s)" }, { label: "E (V)", scale: "y" }, { label: "I (A)", scale: "i", side: 1, grid: { show: false } }],
      series: [
        { label: "t" },
        { label: "E", stroke: "#2563eb", scale: "y", width: 1.5, points: { show: false } },
        { label: "I", stroke: "#f97316", scale: "i", width: 1, points: { show: false }, show: showCurrent },
      ],
      cursor: { drag: { x: true, y: false, uni: 20 } },
      hooks: {
        setSelect: [
          (u: uPlot) => {
            const dataset = activeDataset(store.get());
            if (!dataset || u.select.width < 4) return;
            const min = u.posToVal(u.select.left, "x");
            const max = u.posToVal(u.select.left + u.select.width, "x");
            u.select.width = 0;
            u.select.height = 0;
            xRange = { ...xRange, min, max };
            setAxisInputs(wrapper, "x", xRange);
            void redecimate(dataset.id, min, max);
          },
        ],
      },
      plugins: [restShadingPlugin(bands)],
    };
    return new uPlot(opts, [[], [], []], host);
  }

  async function loadFull(datasetId: string): Promise<void> {
    const series = await worker.getDecimatedSeries(datasetId, -Infinity, Infinity, TARGET_POINTS);
    if (!series) return;
    chart ??= buildChart(host.clientWidth || 800);
    chart.setData([series.t, series.e, series.i], true);
    applyAxisRange(chart, "y", yRange);
  }

  async function rebuild(): Promise<void> {
    const dataset = activeDataset(store.get());
    if (!dataset) return;
    chart?.destroy();
    chart = buildChart(host.clientWidth || 800);
    if (xRange.min !== null || xRange.max !== null) await applyXRange();
    else await loadFull(dataset.id);
  }

  function render(): void {
    const dataset = activeDataset(store.get());
    if (!dataset?.segmentation) {
      wrapper.style.display = "none";
      currentDatasetId = null;
      lastSegmentation = null;
      return;
    }
    wrapper.style.display = "";
    // Reload whenever segmentation was actually recomputed (e.g. the state
    // threshold changed, including via the §7.1 suggestion banner), not just
    // when switching datasets -- otherwise a fix that turns an empty
    // segmentation into a real one leaves the chart showing its earlier,
    // empty fetch until an unrelated zoom/drag happens to trigger a refetch.
    if (dataset.id !== currentDatasetId || dataset.segmentation !== lastSegmentation) {
      currentDatasetId = dataset.id;
      lastSegmentation = dataset.segmentation;
      void loadFull(dataset.id);
    } else {
      chart?.redraw();
    }
  }

  store.subscribe(render);
  render();
}
