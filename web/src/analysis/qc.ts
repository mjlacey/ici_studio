// §9 Quality control. Every field a flag needs already exists in
// `AnalysisTable` (adjR2/nPts/edgeMaxZ/edgeMaeRatio/r/k/i/i0 -- §7.8's
// non-physical NaN'ing is already baked into r/k from Stage A), and
// thresholds are user-editable, so flags are computed here in TS rather
// than in core/wasm -- baking them into the wasm result would mean
// re-running Stage A just to move a threshold slider.

import type { AnalysisTable, ManualExclusions, QcConfig } from "../types";

export interface RowFlags {
  poorFit: boolean;
  tooFewPoints: boolean;
  edgeCurvature: boolean;
  edgeImbalance: boolean;
  nonphysical: boolean;
  degenerateDeltaI: boolean;
}

const FLAG_KEYS: (keyof RowFlags)[] = ["poorFit", "tooFewPoints", "edgeCurvature", "edgeImbalance", "nonphysical", "degenerateDeltaI"];

/** `ΔI = i - i0`, the exact formula `derive.rs` already uses internally for `R`/`k`. */
export function computeRowFlags(table: AnalysisTable, i: number, cfg: QcConfig): RowFlags {
  const deltaI = table.i[i] - table.i0[i];
  return {
    poorFit: cfg.poorFit.enabled && table.adjR2[i] < cfg.poorFit.threshold,
    tooFewPoints: cfg.tooFewPoints.enabled && table.nPts[i] < cfg.tooFewPoints.threshold,
    edgeCurvature: cfg.edgeCurvature.enabled && table.edgeMaxZ[i] > cfg.edgeCurvature.threshold,
    edgeImbalance: cfg.edgeImbalance.enabled && table.edgeMaeRatio[i] > cfg.edgeImbalance.threshold,
    nonphysical: cfg.nonphysical.enabled && (!Number.isFinite(table.r[i]) || !Number.isFinite(table.k[i])),
    degenerateDeltaI: cfg.degenerateDeltaI.enabled && Math.abs(deltaI) < cfg.degenerateDeltaI.threshold,
  };
}

export function isRowFlagged(flags: RowFlags): boolean {
  return FLAG_KEYS.some((key) => flags[key]);
}

function matchesExcludingFlag(flags: RowFlags, cfg: QcConfig): boolean {
  return FLAG_KEYS.some((key) => flags[key] && cfg[key].excludeFromSmoothing);
}

/** §9: "keyed by group + rest id". */
export function rowExclusionKey(table: AnalysisTable, i: number): string {
  return `${table.groupId[i]}:${table.rest[i]}`;
}

/** A manual override wins outright; absent one, exclusion falls back to "does any enabled, exclude-from-smoothing flag match this row". */
export function isRowExcluded(table: AnalysisTable, i: number, cfg: QcConfig, manualExclusions: ManualExclusions): boolean {
  const manual = manualExclusions[rowExclusionKey(table, i)];
  if (manual === "include") return false;
  if (manual === "exclude") return true;
  return matchesExcludingFlag(computeRowFlags(table, i, cfg), cfg);
}

/** Row indices to withhold from Stage B's smoothing input (`runStageB`'s `excludedRowIndices`). */
export function computeExcludedIndices(table: AnalysisTable, cfg: QcConfig, manualExclusions: ManualExclusions): number[] {
  const n = table.rest.length;
  const out: number[] = [];
  for (let i = 0; i < n; i++) {
    if (isRowExcluded(table, i, cfg, manualExclusions)) out.push(i);
  }
  return out;
}

export interface QcSummaryCounts {
  total: number;
  flagged: number;
  excluded: number;
}

export function qcSummary(table: AnalysisTable, cfg: QcConfig, manualExclusions: ManualExclusions): QcSummaryCounts {
  const n = table.rest.length;
  let flagged = 0;
  let excluded = 0;
  for (let i = 0; i < n; i++) {
    if (isRowFlagged(computeRowFlags(table, i, cfg))) flagged++;
    if (isRowExcluded(table, i, cfg, manualExclusions)) excluded++;
  }
  return { total: n, flagged, excluded };
}
