// §10.2: the E-vs-√step.t diagnostic panel -- rest navigator, scatter with
// fitted line + greyed excluded points, and the live R/k/adjR²/n/RMSE
// annotation. The fit itself comes from the main-thread live-fit WASM
// instance (`live-fit-client`), not the worker, so dragging the window
// bounds gets an immediate answer (§2.1/§10.2).

import uPlot from "uplot";
import "uplot/dist/uPlot.min.css";
import { activeDataset, store, updateDataset } from "../state";
import { fitRestPreviewLive } from "../preview/live-fit-client";
import type { RestPoints, RestPreview } from "../types";
import type { DataWorkerClient } from "../worker/client";
import { applyAxisRange, axisControlsHtml, defaultAxisRange, wireAxisControls } from "./axis-controls";
import { regressionWindowPlugin } from "./band-plugin";
import { pngExportControlHtml, wirePngExportControl } from "./png-export";
import { chartGridColor, chartTextColor, onThemeChange } from "../theme";

export function mountPlot2(container: HTMLElement, worker: DataWorkerClient): void {
  let chart: uPlot | null = null;
  let currentDatasetId: string | null = null;
  let pointsKey: string | null = null;
  let points: RestPoints | null = null;
  let fitToken = 0;
  let xRange = defaultAxisRange();
  let yRange = defaultAxisRange();

  const wrapper = document.createElement("section");
  wrapper.className = "panel plot-panel";
  wrapper.style.display = "none";
  wrapper.innerHTML = `
    <h2><span>Regression diagnostic (E vs √t)</span>${pngExportControlHtml()}</h2>
    <div class="rest-navigator">
      <button type="button" data-nav="prev" title="Previous rest (←)">◀</button>
      <span data-rest-label>Rest — of —</span>
      <button type="button" data-nav="next" title="Next rest (→)">▶</button>
      <label>Jump to <input type="number" data-jump min="1" step="1" /></label>
    </div>
    <div class="plot-controls">
      ${axisControlsHtml("x", xRange)}
      ${axisControlsHtml("y", yRange)}
    </div>
    <div class="plot-host"></div>
    <div class="fit-annotation" data-annotation></div>
  `;
  container.appendChild(wrapper);

  const host = wrapper.querySelector<HTMLDivElement>(".plot-host")!;
  const label = wrapper.querySelector<HTMLSpanElement>("[data-rest-label]")!;
  const jumpInput = wrapper.querySelector<HTMLInputElement>("[data-jump]")!;
  const annotationEl = wrapper.querySelector<HTMLDivElement>("[data-annotation]")!;

  wirePngExportControl(
    wrapper,
    () => chart,
    () => `${activeDataset(store.get())?.filename ?? "export"}_E_vs_sqrt_stepT.png`,
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
      onLogChange: () => rebuild(),
    },
  );

  function boundaries() {
    return activeDataset(store.get())?.segmentation?.restBoundaries ?? [];
  }

  function currentPosition(): number {
    const dataset = activeDataset(store.get());
    if (!dataset) return -1;
    return boundaries().findIndex((b) => b.groupId === dataset.selectedGroupId && b.restId === dataset.selectedRestId);
  }

  function selectByPosition(pos: number): void {
    const bs = boundaries();
    if (bs.length === 0) return;
    const clamped = Math.max(0, Math.min(bs.length - 1, pos));
    const dataset = activeDataset(store.get());
    if (!dataset) return;
    updateDataset(dataset.id, (d) => ({ ...d, selectedGroupId: bs[clamped].groupId, selectedRestId: bs[clamped].restId }));
  }

  wrapper.querySelector<HTMLButtonElement>('[data-nav="prev"]')!.addEventListener("click", () => selectByPosition(currentPosition() - 1));
  wrapper.querySelector<HTMLButtonElement>('[data-nav="next"]')!.addEventListener("click", () => selectByPosition(currentPosition() + 1));
  jumpInput.addEventListener("change", () => {
    const n = Number(jumpInput.value);
    if (Number.isFinite(n)) selectByPosition(n - 1);
  });

  function isTypingTarget(el: EventTarget | null): boolean {
    return el instanceof HTMLElement && ["INPUT", "SELECT", "TEXTAREA"].includes(el.tagName);
  }
  window.addEventListener("keydown", (e: KeyboardEvent) => {
    if (wrapper.style.display === "none" || isTypingTarget(e.target)) return;
    if (e.key === "ArrowLeft") selectByPosition(currentPosition() - 1);
    else if (e.key === "ArrowRight") selectByPosition(currentPosition() + 1);
  });

  function buildChart(width: number): uPlot {
    const opts: uPlot.Options = {
      width,
      height: 300,
      scales: { x: { time: false, distr: xRange.log ? 3 : 1 }, y: { distr: yRange.log ? 3 : 1 } },
      axes: [
        { label: "√(step.t) [√s]", stroke: chartTextColor, grid: { stroke: chartGridColor }, ticks: { stroke: chartGridColor } },
        { label: "E (V)", stroke: chartTextColor, grid: { stroke: chartGridColor }, ticks: { stroke: chartGridColor } },
      ],
      series: [
        { label: "√(step.t)" },
        { label: "in window", scale: "y", paths: () => null, points: { show: true, size: 6, fill: "#2563eb", stroke: "#2563eb" } },
        { label: "excluded", scale: "y", paths: () => null, points: { show: true, size: 6, fill: "#94a3b8", stroke: "#94a3b8" } },
        { label: "fit", scale: "y", stroke: "#16a34a", width: 2, points: { show: false } },
      ],
      plugins: [
        regressionWindowPlugin({
          getWindow: () => activeDataset(store.get())?.regressionWindow ?? { tMin: 0, tMax: 0 },
          onDrag: (next) => {
            const dataset = activeDataset(store.get());
            if (dataset) updateDataset(dataset.id, (d) => ({ ...d, regressionWindow: next }));
          },
        }),
      ],
    };
    return new uPlot(opts, [[], [], [], []], host);
  }

  function renderScatterAndFit(pts: RestPoints, fit: RestPreview | null, tMin: number, tMax: number): void {
    if (!chart) return;
    const pairs = pts.restStepT.map((t, idx) => ({ t, e: pts.restVoltage[idx] })).sort((a, b) => a.t - b.t);
    const xs = pairs.map((p) => Math.sqrt(Math.max(0, p.t)));
    const inWindow = pairs.map((p) => (p.t >= tMin && p.t <= tMax ? p.e : null));
    const excluded = pairs.map((p) => (p.t >= tMin && p.t <= tMax ? null : p.e));
    const canDrawFit = !!fit?.ok && fit.e0 !== null && fit.s !== null;
    const sqrtMin = Math.sqrt(Math.max(0, tMin));
    const sqrtMax = Math.sqrt(Math.max(0, tMax));
    const fitLine = xs.map((x) => (canDrawFit && x >= sqrtMin && x <= sqrtMax ? fit!.e0! + fit!.s! * x : null));
    chart.setData([xs, inWindow, excluded, fitLine], true);
    applyAxisRange(chart, "x", xRange);
    applyAxisRange(chart, "y", yRange);
  }

  function renderAnnotation(fit: RestPreview | null): void {
    if (!fit) {
      annotationEl.innerHTML = "";
      return;
    }
    if (!fit.ok) {
      annotationEl.innerHTML = `<div class="too-few-points">Too few points in window (${fit.nPointsInWindow}) -- widen the regression window.</div>`;
      return;
    }
    const fmt = (v: number | null, digits = 4) => (v === null ? "—" : v.toPrecision(digits));
    annotationEl.innerHTML = `
      <div><span class="label">R</span> ${fmt(fit.r)} Ω</div>
      <div><span class="label">k</span> ${fmt(fit.k)} Ω·s⁻¹ᐟ²</div>
      <div><span class="label">n</span> ${fit.nPointsInWindow}</div>
      <div><span class="label">adj. R²</span> ${fmt(fit.adjR2)}</div>
      <div><span class="label">RMSE</span> ${fmt(fit.rmse)}</div>
      <div><span class="label">edge max |z|</span> ${fmt(fit.edgeMaxZ)}</div>
    `;
  }

  async function ensurePoints(datasetId: string, groupId: number, restId: number): Promise<RestPoints | null> {
    const key = `${datasetId}:${groupId}:${restId}`;
    if (key === pointsKey) return points;
    const fetched = await worker.getRestPoints(datasetId, groupId, restId);
    pointsKey = key;
    points = fetched;
    return fetched;
  }

  async function refresh(): Promise<void> {
    const dataset = activeDataset(store.get());
    if (!dataset?.segmentation || dataset.selectedGroupId === null || dataset.selectedRestId === null) return;
    const bs = dataset.segmentation.restBoundaries;
    const boundary = bs.find((b) => b.groupId === dataset.selectedGroupId && b.restId === dataset.selectedRestId);
    if (!boundary) return;

    label.textContent = `Rest ${boundary.index + 1} of ${dataset.segmentation.totalRests}`;
    if (document.activeElement !== jumpInput) jumpInput.value = String(boundary.index + 1);
    jumpInput.max = String(dataset.segmentation.totalRests);

    const pts = await ensurePoints(dataset.id, boundary.groupId, boundary.restId);
    if (!pts || !chart) return;

    const token = ++fitToken;
    const { tMin, tMax } = dataset.regressionWindow;
    const fit = await fitRestPreviewLive(pts, tMin, tMax, dataset.stageAConfig);
    if (token !== fitToken) return; // a newer request superseded this one

    renderScatterAndFit(pts, fit, tMin, tMax);
    renderAnnotation(fit);
  }

  function rebuild(): void {
    chart?.destroy();
    chart = buildChart(host.clientWidth || 800);
    void refresh();
  }

  function render(): void {
    const dataset = activeDataset(store.get());
    if (!dataset?.segmentation) {
      wrapper.style.display = "none";
      currentDatasetId = null;
      return;
    }
    wrapper.style.display = "";
    if (dataset.id !== currentDatasetId) {
      currentDatasetId = dataset.id;
      pointsKey = null;
      chart ??= buildChart(host.clientWidth || 800);
    }
    void refresh();
  }

  store.subscribe(render);
  render();
  onThemeChange(() => chart?.redraw(true, true));
}
