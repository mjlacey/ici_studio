// §11.5: shared axis min/max/log controls, used by every plot (built-in
// and additional). Range/log state itself stays a local closure inside
// each plot's own mount function -- not persisted on Dataset -- matching
// the existing precedent (xKey/showErrorBars/minPx/showCurrent are already
// per-instance closures). `distr` (log vs linear) can only be set at
// `new uPlot(...)` construction time (confirmed against the installed
// uplot package: axis tick/split/filter functions are derived once from
// `sc.distr` during axis init, `setScale` never revisits them) -- so a
// log-checkbox change always requires the caller to rebuild the chart,
// while a min/max change never does.

import type uPlot from "uplot";

export interface AxisRange {
  min: number | null;
  max: number | null;
  log: boolean;
}

export function defaultAxisRange(): AxisRange {
  return { min: null, max: null, log: false };
}

/** A handful of outlier rows (e.g. boundary rests right after a state-
 * threshold or Q-anchoring change) can carry a genuinely real value many
 * times the typical magnitude. uPlot's native auto-fit is a plain
 * min/max, so a single such outlier stretches the whole axis and
 * visually flattens the otherwise-informative bulk of a curve into a
 * sliver near zero -- not a rendering failure, just a bad default range.
 * Computes a robust (5th-95th percentile, with margin) range instead,
 * for the *default* view only -- the "Auto" button still means "true
 * full-data auto-fit", unchanged, for anyone who wants the real extremes.
 */
export function robustRange(values: number[], opts: { clampMinToZero?: boolean } = {}): { min: number; max: number } | null {
  const finite = values.filter((v) => Number.isFinite(v)).sort((a, b) => a - b);
  if (finite.length < 8) return null;
  const pct = (p: number): number => finite[Math.min(finite.length - 1, Math.max(0, Math.round(p * (finite.length - 1))))];
  const lo = pct(0.05);
  const hi = pct(0.95);
  if (!(hi > lo)) return null;
  const margin = (hi - lo) * 0.15;
  const min = lo - margin;
  return { min: opts.clampMinToZero ? Math.max(0, min) : min, max: hi + margin };
}

export function axisControlsHtml(prefix: "x" | "y", range: AxisRange): string {
  return `
    <span class="axis-controls">
      ${prefix}:
      <input type="number" step="any" placeholder="min" data-axis="${prefix}.min" value="${range.min ?? ""}" />
      <input type="number" step="any" placeholder="max" data-axis="${prefix}.max" value="${range.max ?? ""}" />
      <label class="toggle"><input type="checkbox" data-axis="${prefix}.log" ${range.log ? "checked" : ""} /> log</label>
      <button type="button" class="btn-small" data-axis-auto="${prefix}">Auto</button>
    </span>
  `;
}

export function wireAxisControls(
  container: HTMLElement,
  getRange: (axis: "x" | "y") => AxisRange,
  setRange: (axis: "x" | "y", next: AxisRange) => void,
  opts: { onRangeChange: (axis: "x" | "y") => void; onLogChange: (axis: "x" | "y") => void },
): void {
  for (const el of container.querySelectorAll<HTMLInputElement>("[data-axis]")) {
    el.addEventListener("change", () => {
      const [axis, field] = el.dataset.axis!.split(".") as ["x" | "y", "min" | "max" | "log"];
      const range = { ...getRange(axis) };
      if (field === "log") {
        range.log = el.checked;
        setRange(axis, range);
        setAxisInputs(container, axis, range);
        opts.onLogChange(axis);
      } else {
        range[field] = el.value === "" ? null : Number(el.value);
        setRange(axis, range);
        setAxisInputs(container, axis, range);
        opts.onRangeChange(axis);
      }
    });
  }

  for (const btn of container.querySelectorAll<HTMLButtonElement>("[data-axis-auto]")) {
    btn.addEventListener("click", () => {
      const axis = btn.dataset.axisAuto as "x" | "y";
      const range = { ...getRange(axis), min: null, max: null };
      setRange(axis, range);
      setAxisInputs(container, axis, range);
      opts.onRangeChange(axis);
    });
  }
}

/** Reflects `range` back into the min/max inputs -- e.g. after a drag-zoom updates the range programmatically. Skips a field the user is actively typing into. */
export function setAxisInputs(container: HTMLElement, prefix: "x" | "y", range: AxisRange): void {
  const minEl = container.querySelector<HTMLInputElement>(`[data-axis="${prefix}.min"]`);
  const maxEl = container.querySelector<HTMLInputElement>(`[data-axis="${prefix}.max"]`);
  if (minEl && document.activeElement !== minEl) minEl.value = range.min === null ? "" : String(range.min);
  if (maxEl && document.activeElement !== maxEl) maxEl.value = range.max === null ? "" : String(range.max);
}

/** The one call site every plot uses, right after `setData(data, true)` or `new uPlot(...)` -- both already auto-fit at that point, so this only overrides when the user has set a custom bound (playbook §6: always auto-fit, then override). */
export function applyAxisRange(chart: uPlot, axis: "x" | "y", range: AxisRange): void {
  if (range.min === null && range.max === null) return;
  const auto = chart.scales[axis];
  const min = range.min ?? auto.min;
  const max = range.max ?? auto.max;
  if (min == null || max == null) return;
  chart.setScale(axis, { min, max });
}
