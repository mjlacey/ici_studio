// §11.4: the column registry additional plots pick x/y/error columns from
// -- every numeric AnalysisTable field (everything but `state`) plus the 5
// Stage B-derived columns. Deliberately a separate, broader registry from
// plot34-resistance.ts's own narrower X_COLUMNS (base columns only, no
// smoothed columns) -- not refactoring that already-shipped file for a
// cosmetic dedup. Labels match results-table.ts's BASE_COLUMNS/SMOOTH_COLUMNS.

import type { AnalysisTable, PlotColumnKey, StageBResult } from "../types";

export interface PlotColumnDef {
  key: PlotColumnKey;
  label: string;
  get: (t: AnalysisTable, b: StageBResult | null) => (number | null)[];
}

function nullIfNotFinite(values: number[]): (number | null)[] {
  return values.map((v) => (Number.isFinite(v) ? v : null));
}

function smoothed(get: (b: StageBResult) => (number | null)[]): (t: AnalysisTable, b: StageBResult | null) => (number | null)[] {
  return (_t, b) => (b ? get(b) : []);
}

export const PLOT_COLUMNS: PlotColumnDef[] = [
  { key: "groupId", label: "group", get: (t) => nullIfNotFinite(t.groupId) },
  { key: "cycN", label: "cyc.n", get: (t) => nullIfNotFinite(t.cycN) },
  { key: "rest", label: "rest", get: (t) => nullIfNotFinite(t.rest) },
  { key: "t", label: "t", get: (t) => nullIfNotFinite(t.t) },
  { key: "stepT", label: "step.t", get: (t) => nullIfNotFinite(t.stepT) },
  { key: "e", label: "E", get: (t) => nullIfNotFinite(t.e) },
  { key: "i", label: "I", get: (t) => nullIfNotFinite(t.i) },
  { key: "q", label: "Q", get: (t) => nullIfNotFinite(t.q) },
  { key: "e0", label: "E0", get: (t) => nullIfNotFinite(t.e0) },
  { key: "e0Err", label: "E0_err", get: (t) => nullIfNotFinite(t.e0Err) },
  { key: "s", label: "s", get: (t) => nullIfNotFinite(t.s) },
  { key: "sErr", label: "s_err", get: (t) => nullIfNotFinite(t.sErr) },
  { key: "i0", label: "I0", get: (t) => nullIfNotFinite(t.i0) },
  { key: "nPts", label: "n_pts", get: (t) => nullIfNotFinite(t.nPts) },
  { key: "r2", label: "r2", get: (t) => nullIfNotFinite(t.r2) },
  { key: "adjR2", label: "adj_r2", get: (t) => nullIfNotFinite(t.adjR2) },
  { key: "rmse", label: "rmse", get: (t) => nullIfNotFinite(t.rmse) },
  { key: "edgeMaeRatio", label: "edge_mae_ratio", get: (t) => nullIfNotFinite(t.edgeMaeRatio) },
  { key: "edgeMaxZ", label: "edge_max_z", get: (t) => nullIfNotFinite(t.edgeMaxZ) },
  { key: "r", label: "R", get: (t) => nullIfNotFinite(t.r) },
  { key: "rErr", label: "R_err", get: (t) => nullIfNotFinite(t.rErr) },
  { key: "k", label: "k", get: (t) => nullIfNotFinite(t.k) },
  { key: "kErr", label: "k_err", get: (t) => nullIfNotFinite(t.kErr) },
  { key: "e0Smooth", label: "E0_smooth", get: smoothed((b) => b.e0Smooth) },
  { key: "kSmooth", label: "k_smooth", get: smoothed((b) => b.kSmooth) },
  { key: "rSmooth", label: "R_smooth", get: smoothed((b) => b.rSmooth) },
  { key: "dVdQ", label: "dV/dQ", get: smoothed((b) => b.dVdQ) },
  { key: "dQdV", label: "dQ/dV", get: smoothed((b) => b.dQdV) },
];

export function plotColumnLabel(key: PlotColumnKey): string {
  return PLOT_COLUMNS.find((c) => c.key === key)?.label ?? key;
}
