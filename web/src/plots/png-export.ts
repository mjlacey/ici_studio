// §12.4: per-panel PNG export at a user-selected scale factor. Confirmed
// via direct read of the installed uPlot source (`uplot.iife.js`):
// `setSize()` schedules its redraw via `queueMicrotask` (`commit()` ->
// `queueMicrotask(_commit)`), it does *not* draw synchronously --
// capturing the canvas immediately after `setSize()` with no `await`
// would grab stale/pre-resize pixels, a real correctness bug, not just a
// cosmetic flash. Fix: push a one-off callback onto uPlot's own mutable
// `chart.hooks.draw` array ("fires after everything is drawn," so it runs
// synchronously at the end of the real redraw), capture there. Since
// microtasks flush before the browser's next paint, there's still no
// visible flash despite the async round trip.
//
// Restoring the original size needs a *second* deferral, confirmed by
// reading `_commit()`'s own body: `queuedCommit` (the guard `commit()`
// checks before scheduling a new microtask) isn't reset to `false` until
// the very end of `_commit()`, *after* `fire("draw")` -- i.e. after our
// hook already ran. Calling `setSize()` synchronously from inside the
// `draw` hook therefore calls `commit()` while `queuedCommit` is still
// `true`, so the restore's redraw is silently never scheduled (verified:
// without this extra deferral, the on-screen chart is left at the scaled-
// up size after export). Queuing the restore with another `queueMicrotask`
// lets the current `_commit()` finish (and reset the flag) first.
//
// uPlot's own `pxRatio` (device-pixel-ratio scaling) already applies to
// the canvas backing store independently of `scale` -- the two compound
// (e.g. scale=2 on a 2x-DPR screen yields a 4x-CSS-size PNG). Intentional:
// sharper output, not a bug to work around.

import type uPlot from "uplot";

/** A small scale-select + download button, meant for a plot panel's `<h2>` row. */
export function pngExportControlHtml(): string {
  return `
    <span class="png-export">
      <select data-png-scale title="Export scale">
        <option value="1">1×</option>
        <option value="2" selected>2×</option>
        <option value="3">3×</option>
      </select>
      <button type="button" class="btn-small" data-png-download title="Download PNG">⬇ PNG</button>
    </span>
  `;
}

/** Wires up `pngExportControlHtml`'s markup -- `getChart` may return `null` if the chart hasn't built yet. */
export function wirePngExportControl(container: HTMLElement, getChart: () => uPlot | null, filenameFor: () => string): void {
  const scaleSelect = container.querySelector<HTMLSelectElement>("[data-png-scale]");
  const btn = container.querySelector<HTMLButtonElement>("[data-png-download]");
  if (!scaleSelect || !btn) return;
  btn.addEventListener("click", () => {
    const chart = getChart();
    if (!chart) return;
    const scale = Number(scaleSelect.value) as 1 | 2 | 3;
    void exportPng(chart, scale).then((dataUrl) => {
      const a = document.createElement("a");
      a.href = dataUrl;
      a.download = filenameFor();
      a.click();
    });
  });
}

export function exportPng(chart: uPlot, scale: 1 | 2 | 3): Promise<string> {
  const width = chart.width;
  const height = chart.height;

  return new Promise((resolve) => {
    const onDraw = (u: uPlot): void => {
      const dataUrl = u.ctx.canvas.toDataURL("image/png");
      const hooks = u.hooks.draw;
      if (hooks) {
        const idx = hooks.indexOf(onDraw);
        if (idx !== -1) hooks.splice(idx, 1);
      }
      queueMicrotask(() => u.setSize({ width, height }));
      resolve(dataUrl);
    };
    (chart.hooks.draw ??= []).push(onDraw);
    chart.setSize({ width: width * scale, height: height * scale });
  });
}
