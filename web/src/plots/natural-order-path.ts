// uPlot's shared x-array must be sorted in *displayed* x order for
// cursor/legend lookups to work at all, so the default line renderer
// always connects a series' points in that order too. That's fine when
// the displayed x column is itself the series' natural/independent
// parameter (Q) and its own points are contiguous in the shared array --
// but once another series is heavily interleaved with it in that shared
// array (e.g. two smoothing groups whose Q domains overlap after an
// "end" Q anchor, or a secondary display column like E that isn't
// perfectly monotonic-with-Q), the default per-series gap/gap-gap
// handling can end up drawing far less of the line than the data
// actually supports. This builds the stroke path explicitly instead --
// still uses the public `u.valToPos` API so it stays in sync with
// whatever scale/zoom uPlot is currently showing, but walks the given
// positions (indices into the shared arrays, already sorted in whatever
// order the caller considers "natural" for this series) directly rather
// than delegating to uPlot's own path builder.
import type uPlot from "uplot";

export function naturalOrderPathBuilder(orderedPositions: number[], xArr: number[], yArr: (number | null)[]): uPlot.Series.PathBuilder {
  return (u: uPlot) => {
    const path = new Path2D();
    let started = false;
    for (const k of orderedPositions) {
      const yv = yArr[k];
      if (yv === null || !Number.isFinite(yv)) {
        started = false;
        continue;
      }
      const px = u.valToPos(xArr[k], "x", true);
      const py = u.valToPos(yv, "y", true);
      if (!started) {
        path.moveTo(px, py);
        started = true;
      } else {
        path.lineTo(px, py);
      }
    }
    return { stroke: path, fill: null };
  };
}
