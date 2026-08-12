// Shared uPlot drawing-hook helpers (§11.1/§10.2): lightly-shaded rest
// regions with a selected-rest highlight for Plot 1, and a shaded,
// mouse-draggable regression window for Plot 2. Both draw directly on the
// canvas in the `drawClear` hook, which fires before axes/series are drawn
// each redraw, so the shading sits behind the data.

import type uPlot from "uplot";
import type { RegressionWindow } from "../types";

export interface RestBand {
  tStart: number;
  tEnd: number;
  isSelected: boolean;
}

/** Plot 1: shade each rest's [tStart, tEnd] on the (raw time) x-axis. */
export function restShadingPlugin(getBands: () => RestBand[]): uPlot.Plugin {
  return {
    hooks: {
      drawClear: [
        (u: uPlot) => {
          const bands = getBands();
          if (bands.length === 0) return;
          const ctx = u.ctx;
          const { left, top, width, height } = u.bbox;
          ctx.save();
          for (const band of bands) {
            const x0 = u.valToPos(band.tStart, "x", true);
            const x1 = u.valToPos(band.tEnd, "x", true);
            const clippedX0 = Math.max(x0, left);
            const clippedX1 = Math.min(x1, left + width);
            if (clippedX1 <= clippedX0) continue;
            ctx.fillStyle = band.isSelected ? "rgba(59,130,246,0.30)" : "rgba(148,163,184,0.18)";
            ctx.fillRect(clippedX0, top, clippedX1 - clippedX0, height);
          }
          ctx.restore();
        },
      ],
    },
  };
}

/**
 * Plot 1: shade whatever falls *outside* the detected ICI cycle (§ non-ICI
 * region detection, `analysis/ici-region.ts`) -- everything before it
 * (capacity-check cycles, OCV rests, ...) and everything after it, using
 * the chart's own currently-loaded x-data for the outer bounds so this
 * works with Plot 1's re-decimate-on-zoom behaviour without needing to
 * know the dataset's true full time range in advance. `getIciSpan`
 * returning `null` (nothing excluded -- either a pure-ICI file, or
 * segmentation hasn't loaded yet) draws nothing, so this is a no-op for
 * every file that doesn't need it.
 */
export function nonIciShadingPlugin(getIciSpan: () => { start: number; end: number } | null): uPlot.Plugin {
  return {
    hooks: {
      drawClear: [
        (u: uPlot) => {
          const span = getIciSpan();
          if (!span) return;
          const xData = u.data[0] as number[] | undefined;
          if (!xData || xData.length === 0) return;
          const dataMin = xData[0];
          const dataMax = xData[xData.length - 1];
          const ctx = u.ctx;
          const { left, top, width, height } = u.bbox;
          ctx.save();
          ctx.fillStyle = "rgba(220,38,38,0.10)";
          const drawRange = (a: number, b: number): void => {
            if (b <= a) return;
            const x0 = Math.max(u.valToPos(a, "x", true), left);
            const x1 = Math.min(u.valToPos(b, "x", true), left + width);
            if (x1 <= x0) return;
            ctx.fillRect(x0, top, x1 - x0, height);
          };
          drawRange(dataMin, span.start);
          drawRange(span.end, dataMax);
          ctx.restore();
        },
      ],
    },
  };
}

export interface WindowBoundsCallbacks {
  getWindow: () => RegressionWindow;
  onDrag: (next: RegressionWindow) => void;
}

const HIT_PX = 6;

/**
 * Plot 2: shade the [tMin, tMax] regression window and let the user drag
 * either boundary. The x-axis is √step.t, so pixel<->time conversion goes
 * through a square/sqrt (§10.2).
 */
export function regressionWindowPlugin(cb: WindowBoundsCallbacks): uPlot.Plugin {
  let dragging: "min" | "max" | null = null;

  // Canvas-pixel space (matches u.bbox / u.ctx) for drawing.
  function xPosCanvas(u: uPlot, t: number): number {
    return u.valToPos(Math.sqrt(Math.max(0, t)), "x", true);
  }

  // CSS-pixel space (matches getBoundingClientRect / MouseEvent.clientX) for
  // hit-testing and dragging -- these are NOT the same space when
  // devicePixelRatio != 1, which is why this is a separate function.
  function xPosCss(u: uPlot, t: number): number {
    return u.valToPos(Math.sqrt(Math.max(0, t)), "x", false);
  }

  function onMouseDown(u: uPlot, over: HTMLElement, e: MouseEvent): void {
    const rect = over.getBoundingClientRect();
    const px = e.clientX - rect.left;
    const { tMin, tMax } = cb.getWindow();
    const x0 = xPosCss(u, tMin);
    const x1 = xPosCss(u, tMax);
    if (Math.abs(px - x0) <= HIT_PX) dragging = "min";
    else if (Math.abs(px - x1) <= HIT_PX) dragging = "max";
    else dragging = null;
    if (dragging) e.preventDefault();
  }

  function onMouseMove(u: uPlot, over: HTMLElement, e: MouseEvent): void {
    if (!dragging) return;
    const rect = over.getBoundingClientRect();
    const px = Math.min(Math.max(e.clientX - rect.left, 0), rect.width);
    const sqrtT = u.posToVal(px, "x");
    const t = Math.max(0, sqrtT) ** 2;
    const { tMin, tMax } = cb.getWindow();
    if (dragging === "min") cb.onDrag({ tMin: Math.min(t, tMax), tMax });
    else cb.onDrag({ tMin, tMax: Math.max(t, tMin) });
  }

  function onMouseUp(): void {
    dragging = null;
  }

  return {
    hooks: {
      drawClear: [
        (u: uPlot) => {
          const { tMin, tMax } = cb.getWindow();
          const ctx = u.ctx;
          const { top, height } = u.bbox;
          const x0 = xPosCanvas(u, tMin);
          const x1 = xPosCanvas(u, tMax);
          ctx.save();
          ctx.fillStyle = "rgba(34,197,94,0.14)";
          ctx.fillRect(Math.min(x0, x1), top, Math.abs(x1 - x0), height);
          ctx.strokeStyle = "rgba(21,128,61,0.9)";
          ctx.lineWidth = 2;
          for (const x of [x0, x1]) {
            ctx.beginPath();
            ctx.moveTo(x, top);
            ctx.lineTo(x, top + height);
            ctx.stroke();
          }
          ctx.restore();
        },
      ],
      ready: [
        (u: uPlot) => {
          const over = u.over;
          const down = (e: Event) => onMouseDown(u, over, e as MouseEvent);
          const move = (e: Event) => onMouseMove(u, over, e as MouseEvent);
          const up = () => onMouseUp();
          over.addEventListener("mousedown", down);
          window.addEventListener("mousemove", move);
          window.addEventListener("mouseup", up);
          u.hooks.destroy ??= [];
          (u.hooks.destroy as uPlot.Hooks.Defs["destroy"][]).push(() => {
            over.removeEventListener("mousedown", down);
            window.removeEventListener("mousemove", move);
            window.removeEventListener("mouseup", up);
          });
        },
      ],
    },
  };
}
