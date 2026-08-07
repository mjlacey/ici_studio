// Message protocol between the main thread and the single data worker
// (§2.1). Every message carries `datasetId` (§4.5: "worker protocol
// messages carry a datasetId"). Requests that expect a reply also carry a
// `requestId` so multiple in-flight requests (e.g. rapid raw-table
// scrolling) can be matched to their responses.

import type {
  ColumnInfo,
  ColumnStats,
  DecimatedSeries,
  GroupKeyColumns,
  OptimalWindowResult,
  Page,
  ParseErrorInfo,
  ParseOverrides,
  ParseReport,
  RestPoints,
  SegmentationSummary,
  SmootherConfig,
  SmoothingKeyConfig,
  StageAResult,
  StageBResult,
} from "../types";

export type { DecimatedSeries, RestPoints, SegmentationSummary } from "../types";

export interface Monotonicity {
  isMonotonic: boolean;
  firstOffendingRow: number | null;
}

/** Shared by `runSegmentation` and `runStageA` -- both run the identical §7.1-7.5/§6 segmentation pass. */
export interface SegmentationInput {
  timeColumn: string;
  timeScale: number;
  cycleColumn: string;
  currentColumn: string;
  currentScale: number;
  voltageColumn: string;
  voltageScale: number;
  chargeColumn: string;
  chargeScale: number;
  groupingColumns: string[];
  stateThreshold: number;
  dropUnrestedReversals: boolean;
  chargeAnchor: "start" | "end";
  dischargeAnchor: "start" | "end";
}

export type WorkerRequest =
  | { type: "parseFile"; datasetId: string; bytes: ArrayBuffer; overrides: ParseOverrides }
  | { type: "reparse"; datasetId: string; overrides: ParseOverrides }
  | { type: "getPage"; datasetId: string; requestId: string; offset: number; limit: number }
  | { type: "getColumnStats"; datasetId: string; requestId: string; column: string }
  | { type: "checkTimeMonotonic"; datasetId: string; requestId: string; column: string }
  | { type: "dropNonFiniteRows"; datasetId: string; requestId: string; column: string }
  | { type: "dropDecreasingRows"; datasetId: string; requestId: string; column: string }
  | { type: "sortByColumn"; datasetId: string; requestId: string; column: string }
  | {
      type: "detectPrenormalizedCharge";
      datasetId: string;
      requestId: string;
      charge: string;
      cycle: string;
      current: string;
      stateThreshold: number;
    }
  | ({ type: "runSegmentation"; datasetId: string; requestId: string } & SegmentationInput)
  | ({
      type: "runStageA";
      datasetId: string;
      requestId: string;
      tMin: number;
      tMax: number;
      voltageInterpWindow: number | null;
      currentAvgWindow: number | null;
      edgePoints: number;
      legacyCompatibility: boolean;
    } & SegmentationInput)
  | {
      type: "runStageB";
      datasetId: string;
      requestId: string;
      smoothingKey: SmoothingKeyConfig;
      e0Smoothing: SmootherConfig;
      kSmoothing: SmootherConfig;
      rInheritsK: boolean;
      rSmoothing: SmootherConfig;
      derivativeWindow: number;
      derivativeDegree: number;
      /** §9: rows QC excludes from smoothing -- indices into the cached `StageAResult.analysisTable`. */
      excludedRowIndices: number[];
    }
  | { type: "getRestPoints"; datasetId: string; requestId: string; groupId: number; restId: number }
  | { type: "getDecimatedSeries"; datasetId: string; requestId: string; xMin: number; xMax: number; targetPoints: number }
  | {
      type: "estimateOptimalWindow";
      datasetId: string;
      requestId: string;
      n: number;
      lMin: number;
      tMinLowerBound: number;
      edgePoints: number;
    }
  | { type: "cancelOptimalWindow"; datasetId: string; requestId: string }
  | { type: "getGroupKeyColumns"; datasetId: string; requestId: string };

export type WorkerResponse =
  | { type: "parseResult"; datasetId: string; ok: true; report: ParseReport; columns: ColumnInfo[]; sha256: string | null }
  | { type: "parseResult"; datasetId: string; ok: false; error: ParseErrorInfo }
  | { type: "pageResult"; datasetId: string; requestId: string; page: Page }
  | { type: "columnStatsResult"; datasetId: string; requestId: string; stats: ColumnStats | null }
  | { type: "monotonicResult"; datasetId: string; requestId: string; result: Monotonicity }
  | { type: "mutateResult"; datasetId: string; requestId: string; report: ParseReport; columns: ColumnInfo[] }
  | { type: "prenormalizedResult"; datasetId: string; requestId: string; looksPrenormalized: boolean }
  | { type: "segmentationResult"; datasetId: string; requestId: string; ok: true; summary: SegmentationSummary }
  | { type: "segmentationResult"; datasetId: string; requestId: string; ok: false; error: string }
  | { type: "restPointsResult"; datasetId: string; requestId: string; points: RestPoints | null }
  | { type: "decimatedSeriesResult"; datasetId: string; requestId: string; series: DecimatedSeries | null }
  | { type: "stageAResult"; datasetId: string; requestId: string; ok: true; result: StageAResult }
  | { type: "stageAResult"; datasetId: string; requestId: string; ok: false; error: string }
  | { type: "stageBResult"; datasetId: string; requestId: string; ok: true; result: StageBResult }
  | { type: "stageBResult"; datasetId: string; requestId: string; ok: false; error: string }
  | { type: "optimalWindowProgress"; datasetId: string; requestId: string; completed: number; total: number }
  | { type: "optimalWindowResult"; datasetId: string; requestId: string; ok: true; result: OptimalWindowResult }
  | { type: "optimalWindowResult"; datasetId: string; requestId: string; ok: false; error: string }
  | { type: "groupKeyColumnsResult"; datasetId: string; requestId: string; result: GroupKeyColumns | null };
