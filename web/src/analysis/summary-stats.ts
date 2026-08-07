// §11.6: per-smoothing-group summary statistics, pure computation shared by
// the summary-stats panel and §12.3's run-report ("key statistics"). Raw/
// un-normalized -- §11.5's area normalisation is presentation-only, applied
// by callers at render/format time (same pattern as results-table.ts).

import type { Dataset, SummaryStatsRow } from "../types";

function median(values: number[]): number {
  if (values.length === 0) return NaN;
  const sorted = [...values].sort((a, b) => a - b);
  const mid = Math.floor(sorted.length / 2);
  return sorted.length % 2 === 0 ? (sorted[mid - 1] + sorted[mid]) / 2 : sorted[mid];
}

function quartile(sorted: number[], q: number): number {
  if (sorted.length === 0) return NaN;
  const pos = (sorted.length - 1) * q;
  const base = Math.floor(pos);
  const rest = pos - base;
  return base + 1 < sorted.length ? sorted[base] + rest * (sorted[base + 1] - sorted[base]) : sorted[base];
}

function iqr(values: number[]): [number, number] {
  const sorted = [...values].sort((a, b) => a - b);
  return [quartile(sorted, 0.25), quartile(sorted, 0.75)];
}

export function computeSummaryStats(dataset: Dataset): SummaryStatsRow[] {
  const result = dataset.stageAResult;
  const stageB = dataset.stageBResult;
  if (!result || !stageB) return [];
  const table = result.analysisTable;

  return stageB.groups.map((group) => {
    const idx = table.rest.map((_, i) => i).filter((i) => stageB.rowGroupKey[i] === group.key);
    const rValues = idx.map((i) => table.r[i]).filter(Number.isFinite);
    const kValues = idx.map((i) => table.k[i]).filter(Number.isFinite);
    const adjR2Values = idx.map((i) => table.adjR2[i]).filter(Number.isFinite);
    const nPtsValues = idx.map((i) => table.nPts[i]);
    const qValues = idx.map((i) => table.q[i]).filter(Number.isFinite);

    const qSortedIdx = [...idx].sort((a, b) => table.q[a] - table.q[b]);
    const startIdx = qSortedIdx[0];
    const midIdx = qSortedIdx[Math.floor(qSortedIdx.length / 2)];
    const endIdx = qSortedIdx[qSortedIdx.length - 1];

    const [rQ1, rQ3] = iqr(rValues);
    const [kQ1, kQ3] = iqr(kValues);

    return {
      groupLabel: group.label,
      n: group.n,
      rMedian: median(rValues),
      rQ1,
      rQ3,
      kMedian: median(kValues),
      kQ1,
      kQ3,
      medianAdjR2: median(adjR2Values),
      medianNPts: median(nPtsValues),
      rStart: table.r[startIdx],
      rMid: table.r[midIdx],
      rEnd: table.r[endIdx],
      kStart: table.k[startIdx],
      kMid: table.k[midIdx],
      kEnd: table.k[endIdx],
      qMin: qValues.length ? Math.min(...qValues) : null,
      qMax: qValues.length ? Math.max(...qValues) : null,
      e0Spline: group.e0,
      kSpline: group.k,
      rSpline: group.r,
    };
  });
}
