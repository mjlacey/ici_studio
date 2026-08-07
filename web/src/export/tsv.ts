// §12.1: tab-delimited analysed-data export. Reuses `analysis/qc.ts`'s
// existing flag/exclusion functions (no new QC logic) and the worker's
// `groupKeyColumnsJson` lookup for each row's actual grouping-column
// string values (the `AnalysisTable.groupId` int alone can't reconstruct
// them). "extras" (§7.9) is omitted entirely -- an established out-of-
// scope precedent elsewhere in this codebase (e.g. plot34-resistance.ts's
// own X_COLUMNS comment), there's no data for it since the feature was
// never implemented.

import { computeRowFlags, isRowExcluded, type RowFlags } from "../analysis/qc";
import type { Dataset, GroupKeyColumns } from "../types";

export interface TsvOptions {
  precision: "full" | "6sig";
  missing: "empty" | "NA" | "NaN";
  includeUnitComment: boolean;
  /** The secondary "per-rest regression table" export is this table minus the Stage B smoothed columns. */
  includeSmoothed: boolean;
}

export function defaultTsvOptions(): TsvOptions {
  return { precision: "full", missing: "empty", includeUnitComment: false, includeSmoothed: true };
}

function formatValue(v: number | null | undefined, options: TsvOptions): string {
  if (v === null || v === undefined || !Number.isFinite(v)) {
    return options.missing === "empty" ? "" : options.missing === "NA" ? "NA" : "NaN";
  }
  return options.precision === "full" ? v.toString() : v.toPrecision(6);
}

const FLAG_KEYS: (keyof RowFlags)[] = ["poorFit", "tooFewPoints", "edgeCurvature", "edgeImbalance", "nonphysical", "degenerateDeltaI"];

export function buildAnalysisTsv(dataset: Dataset, groupKeyColumns: GroupKeyColumns | null, options: TsvOptions): string {
  const result = dataset.stageAResult;
  if (!result) return "";
  const table = result.analysisTable;
  const stageB = dataset.stageBResult;
  const n = table.rest.length;

  const groupingNames = groupKeyColumns?.groupingColumnNames ?? [];
  const groupValues = (i: number): string[] => {
    if (!groupKeyColumns) return [];
    return groupKeyColumns.values[String(table.groupId[i])] ?? groupingNames.map(() => "");
  };

  const fmt = (v: number | null | undefined): string => formatValue(v, options);

  const smoothedHeaders = options.includeSmoothed ? ["E0_smooth", "R_smooth", "k_smooth", "dVdQ", "dQdV"] : [];
  const headers = [
    ...groupingNames,
    "cyc.n",
    "state",
    "rest",
    "t",
    "step.t",
    "E",
    "I",
    "Q",
    "E0",
    "E0_err",
    "s",
    "s_err",
    "I0",
    "n_pts",
    "r2",
    "adj_r2",
    "rmse",
    "edge_mae_ratio",
    "edge_max_z",
    "R",
    "R_err",
    "k",
    "k_err",
    ...smoothedHeaders,
    "flags",
    "excluded",
  ];

  const lines: string[] = [];
  if (options.includeUnitComment) {
    lines.push("# internal units: t/step.t [s], E [V], I [A], Q [Ah], R [Ω], k [Ω·s^-1/2]");
  }
  lines.push(headers.join("\t"));

  for (let i = 0; i < n; i++) {
    const flags = computeRowFlags(table, i, dataset.qcConfig);
    const flagNames = FLAG_KEYS.filter((k) => flags[k]);
    const excluded = isRowExcluded(table, i, dataset.qcConfig, dataset.manualExclusions);

    const smoothedValues = options.includeSmoothed ? [fmt(stageB?.e0Smooth[i]), fmt(stageB?.rSmooth[i]), fmt(stageB?.kSmooth[i]), fmt(stageB?.dVdQ[i]), fmt(stageB?.dQdV[i])] : [];

    const row = [
      ...groupValues(i),
      fmt(table.cycN[i]),
      table.state[i],
      String(table.rest[i]),
      fmt(table.t[i]),
      fmt(table.stepT[i]),
      fmt(table.e[i]),
      fmt(table.i[i]),
      fmt(table.q[i]),
      fmt(table.e0[i]),
      fmt(table.e0Err[i]),
      fmt(table.s[i]),
      fmt(table.sErr[i]),
      fmt(table.i0[i]),
      String(table.nPts[i]),
      fmt(table.r2[i]),
      fmt(table.adjR2[i]),
      fmt(table.rmse[i]),
      fmt(table.edgeMaeRatio[i]),
      fmt(table.edgeMaxZ[i]),
      fmt(table.r[i]),
      fmt(table.rErr[i]),
      fmt(table.k[i]),
      fmt(table.kErr[i]),
      ...smoothedValues,
      flagNames.join(","),
      String(excluded),
    ];
    lines.push(row.join("\t"));
  }

  return lines.join("\n") + "\n";
}
