// §11.3: error bars for Plots 3/4, drawn as a `draw` hook (fires after
// series are drawn, unlike band-plugin's `drawClear` background shading)
// so bars sit on top of the points. Coordinates are canvas-pixel space
// (`u.valToPos(..., true)`), matching band-plugin's drawing convention --
// this hook only draws, it never hit-tests, so there's no CSS-vs-canvas
// pixel-space hazard here.

import type uPlot from "uplot";

export interface ErrorBarPoint {
  x: number;
  y: number;
  err: number;
}

export interface ErrorBarOptions {
  getPoints: () => ErrorBarPoint[];
  getEnabled: () => boolean;
  /** Bars shorter than this many CSS px are skipped so dense plots stay readable (§11.3). */
  getMinPx: () => number;
  color?: string;
}

const CAP_HALF_WIDTH_PX = 3;

export function errorBarPlugin(opts: ErrorBarOptions): uPlot.Plugin {
  return {
    hooks: {
      draw: [
        (u: uPlot) => {
          if (!opts.getEnabled()) return;
          const points = opts.getPoints();
          if (points.length === 0) return;

          const minPx = opts.getMinPx();
          const dpr = u.bbox.width / u.over.getBoundingClientRect().width;
          const ctx = u.ctx;
          ctx.save();
          ctx.strokeStyle = opts.color ?? "rgba(71,85,105,0.65)";
          ctx.lineWidth = 1;
          for (const p of points) {
            if (!Number.isFinite(p.y) || !Number.isFinite(p.err) || p.err <= 0) continue;
            const xPx = u.valToPos(p.x, "x", true);
            const yLowPx = u.valToPos(p.y - p.err, "y", true);
            const yHighPx = u.valToPos(p.y + p.err, "y", true);
            if (Math.abs(yHighPx - yLowPx) < minPx * dpr) continue;

            ctx.beginPath();
            ctx.moveTo(xPx, yLowPx);
            ctx.lineTo(xPx, yHighPx);
            const cap = CAP_HALF_WIDTH_PX * dpr;
            ctx.moveTo(xPx - cap, yLowPx);
            ctx.lineTo(xPx + cap, yLowPx);
            ctx.moveTo(xPx - cap, yHighPx);
            ctx.lineTo(xPx + cap, yHighPx);
            ctx.stroke();
          }
          ctx.restore();
        },
      ],
    },
  };
}
