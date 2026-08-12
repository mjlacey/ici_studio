// Non-ICI rows (a capacity-check cycle, an OCV rest, a DCIR leg, ...) are
// now dropped during segmentation itself (`core::segment`'s
// `IciDetectionConfig`, driven by `StageAConfig.nonIci*`) -- before Q
// anchoring and Stage A ever see them, not just hidden from the results
// afterward. `restBoundaries` therefore only ever contains ICI rests; the
// one thing still needed here is the time span they cover, so Plot 1 (which
// still shows the *complete* raw series, independently of what segmentation
// dropped -- see wasm's `raw_series` cache) can shade everything outside it.

import type { RestBoundary } from "../types";

/** The time span covered by `restBoundaries` (all of which are ICI-only by construction) -- `null` when nothing was actually dropped, so a file that doesn't need it gets no shading at all. */
export function iciTimeSpan(restBoundaries: RestBoundary[], nonIciRowsDropped: number): { start: number; end: number } | null {
  if (nonIciRowsDropped === 0 || restBoundaries.length === 0) return null;
  let start = Infinity;
  let end = -Infinity;
  for (const b of restBoundaries) {
    if (b.tStart < start) start = b.tStart;
    if (b.tEnd > end) end = b.tEnd;
  }
  return { start, end };
}
