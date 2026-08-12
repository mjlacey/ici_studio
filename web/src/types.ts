// Mirrors the JSON DTOs in wasm/src/lib.rs. Keep in sync by hand -- the
// wasm crate is the source of truth (see its module doc comment).

export type Encoding = "Utf8" | "Cp1252";
export type Delimiter = "Tab" | "Comma" | "Semicolon" | "Pipe";
export type DecimalSeparator = "Dot" | "Comma";

export interface CoercionFailure {
  column: string;
  count: number;
  firstRows: number[];
}

export interface ParseReport {
  encoding: Encoding;
  delimiter: Delimiter;
  decimalSeparator: DecimalSeparator;
  preambleLinesSkipped: number;
  headerPresent: boolean;
  headerSynthesized: boolean;
  nRows: number;
  nColumns: number;
  trailingColumnDropped: boolean;
  raggedRowsDropped: number;
  raggedRowLineNumbers: number[];
  coercionFailures: CoercionFailure[];
  warnings: string[];
}

export interface ColumnInfo {
  name: string;
  isNumeric: boolean;
}

export interface ColumnStats {
  isNumeric: boolean;
  nFinite: number;
  min: number | null;
  median: number | null;
  max: number | null;
  distinctCount: number | null;
}

/** Every field `undefined` (or omitted) means "use the sniffer's own choice". */
export interface ParseOverrides {
  encoding?: Encoding;
  delimiter?: Delimiter;
  decimalSeparator?: DecimalSeparator;
  skipLines?: number;
  headerPresent?: boolean;
}

export interface Page {
  columnNames: string[];
  rows: (number | string | null)[][];
  totalRows: number;
}

export interface ParseErrorInfo {
  message: string;
  delimitersTried?: Delimiter[];
}

// ---------------------------------------------------------------------
// App-level types (§4.5: state shaped for multi-file even though v1 UI is
// single-file; column *mapping* is per-dataset, analysis config is top-level).
// ---------------------------------------------------------------------

export type RequiredField = "time" | "cycle" | "current" | "voltage" | "charge";

export const REQUIRED_FIELDS: RequiredField[] = ["time", "cycle", "current", "voltage", "charge"];

export type TimeUnit = "s" | "min" | "h";
export type CurrentUnit = "A" | "mA" | "uA";
export type VoltageUnit = "V" | "mV";
export type ChargeUnit = "Ah" | "mAh" | "uAh" | "C";

export interface FieldMapping {
  column: string | null;
  unit: string | null;
  /** True until the user edits this row -- clears the "auto-detected" marker (§5.1). */
  autoDetected: boolean;
}

export type ColumnMapping = Record<RequiredField, FieldMapping>;

export function emptyMapping(): ColumnMapping {
  return {
    time: { column: null, unit: "s", autoDetected: false },
    cycle: { column: null, unit: null, autoDetected: false },
    current: { column: null, unit: "A", autoDetected: false },
    voltage: { column: null, unit: "V", autoDetected: false },
    charge: { column: null, unit: "Ah", autoDetected: false },
  };
}

export interface ValidationIssue {
  kind: "nonFinite" | "nonMonotonicTime" | "duplicateColumn" | "nonNumericColumn";
  severity: "warning" | "block";
  message: string;
  column?: string;
  count?: number;
  firstRowIndex?: number;
  /** For "warning" issues with a user-chosen action (§5.3). */
  action?: "dropRows" | "sortByTime" | "dropDecreasingRows";
  resolved?: boolean;
}

export interface RegressionWindow {
  tMin: number;
  tMax: number;
}

/** R's own default regression window (§10.1), until milestone 9's "estimate optimal window" runs. */
export function defaultRegressionWindow(): RegressionWindow {
  return { tMin: 2, tMax: 14 };
}

// ---------------------------------------------------------------------
// Stage A / Stage B (§7, §8) -- mirrors wasm's Stage A/B DTOs.
// ---------------------------------------------------------------------

export interface StageAConfig {
  stateThreshold: number;
  voltageInterpolationWindow: number | null;
  currentAverageWindow: number | null;
  edgePoints: number;
  dropUnrestedReversals: boolean;
  legacyCompatibility: boolean;
  /** Drops rows outside "the ICI cycle" itself (a capacity-check cycle, an OCV rest, a DCIR leg, ...) during segmentation -- before Q anchoring and Stage A ever see them, not just hidden afterward (`core::segment::IciDetectionConfig`). */
  nonIciDetectionEnabled: boolean;
  nonIciMaxRestDurationS: number;
  nonIciMinRepeatCount: number;
}

/** R's own defaults (§7), plus this port's own ICI-cycle-detection defaults. */
export function defaultStageAConfig(): StageAConfig {
  return {
    stateThreshold: 0,
    voltageInterpolationWindow: 10,
    currentAverageWindow: 10,
    edgePoints: 3,
    dropUnrestedReversals: true,
    legacyCompatibility: false,
    nonIciDetectionEnabled: true,
    nonIciMaxRestDurationS: 300,
    nonIciMinRepeatCount: 20,
  };
}

export interface SmootherConfig {
  monotonic: boolean;
  direction: "automatic" | "increasing" | "decreasing";
  k: number;
  m: number;
}

export interface SmoothingKeyConfig {
  useCycN: boolean;
  useState: boolean;
  groupingColumns: string[];
}

export interface StageBConfig {
  smoothingKey: SmoothingKeyConfig;
  e0Smoothing: SmootherConfig;
  kSmoothing: SmootherConfig;
  rInheritsK: boolean;
  rSmoothing: SmootherConfig;
  derivativeWindow: number;
  derivativeDegree: number;
}

/** R's own defaults (§8.1/§8.2): smoothing key all-on, E0 monotonic, k/R not, R inherits k. */
export function defaultStageBConfig(groupingColumns: string[]): StageBConfig {
  return {
    smoothingKey: { useCycN: true, useState: true, groupingColumns: [...groupingColumns] },
    e0Smoothing: { monotonic: true, direction: "automatic", k: 50, m: 1 },
    kSmoothing: { monotonic: false, direction: "automatic", k: 50, m: 1 },
    rInheritsK: true,
    rSmoothing: { monotonic: false, direction: "automatic", k: 50, m: 1 },
    derivativeWindow: 5,
    derivativeDegree: 3,
  };
}

/** Column-oriented, mirrors wasm's `AnalysisTableDto`. One entry per successfully-regressed interruption. */
export interface AnalysisTable {
  groupId: number[];
  cycN: number[];
  state: ("charge" | "discharge")[];
  rest: number[];
  t: number[];
  stepT: number[];
  e: number[];
  i: number[];
  q: number[];
  e0: number[];
  e0Err: number[];
  s: number[];
  sErr: number[];
  i0: number[];
  nPts: number[];
  r2: number[];
  adjR2: number[];
  rmse: number[];
  edgeMaeRatio: number[];
  edgeMaxZ: number[];
  r: number[];
  rErr: number[];
  k: number[];
  kErr: number[];
}

export interface RegressionFailure {
  groupId: number;
  rest: number;
  reason: string;
}

export interface NonphysicalReport {
  rByState: Record<string, number>;
  kByState: Record<string, number>;
}

export interface StageAResult {
  segmentation: SegmentationSummary;
  analysisTable: AnalysisTable;
  regressionFailures: RegressionFailure[];
  nonphysicalReport: NonphysicalReport;
}

export interface SplineDiagnostics {
  directionUsed: "increasing" | "decreasing" | "notApplicable";
  kEffective: number;
  lambda: number;
  edf: number;
}

export interface SmoothingGroup {
  key: string;
  label: string;
  n: number;
  e0: SplineDiagnostics | null;
  k: SplineDiagnostics | null;
  r: SplineDiagnostics | null;
}

/** Parallel arrays aligned to the `StageAResult.analysisTable` rows -- `NaN`/`Infinity` arrive as `null` per §7.8. */
export interface StageBResult {
  e0Smooth: (number | null)[];
  kSmooth: (number | null)[];
  rSmooth: (number | null)[];
  dVdQ: (number | null)[];
  dQdV: (number | null)[];
  /** Aligned to `StageAResult.analysisTable` rows -- which `groups[].key` each row belongs to. */
  rowGroupKey: string[];
  groups: SmoothingGroup[];
}

// ---------------------------------------------------------------------
// §10.3 "Estimate optimal window" -- mirrors wasm's optimal-window DTOs.
// ---------------------------------------------------------------------

export interface OptimalWindowConfig {
  n: number;
  lMin: number;
  tMinLowerBound: number;
}

/** §10.3 defaults: N=20 (5-100 adjustable), L_min=5s, t_min lower bound=1s (allow 0). */
export function defaultOptimalWindowConfig(): OptimalWindowConfig {
  return { n: 20, lMin: 5, tMinLowerBound: 1 };
}

export interface SampledRest {
  groupId: number;
  restId: number;
  state: "charge" | "discharge";
  q: number;
}

export interface CandidateScore {
  tMin: number;
  tMax: number;
  meanAdjR2: number;
  medianAdjR2: number;
  nValid: number;
  nSampled: number;
  medianNPts: number;
  medianEdgeMaxZ: number;
  rejected: boolean;
}

export interface OptimalWindowResult {
  sampledRests: SampledRest[];
  tMaxObserved: number;
  heterogeneousLengths: boolean;
  scores: CandidateScore[];
}

// ---------------------------------------------------------------------
// §9 Quality control -- computed client-side (web/src/analysis/qc.ts) from
// fields already in `AnalysisTable`, not part of any wasm DTO.
// ---------------------------------------------------------------------

export interface QcFlagSetting {
  enabled: boolean;
  /** Unused by `nonphysical`, which has no threshold. */
  threshold: number;
  excludeFromSmoothing: boolean;
}

export interface QcConfig {
  poorFit: QcFlagSetting; // adjR2 < threshold
  tooFewPoints: QcFlagSetting; // nPts < threshold
  edgeCurvature: QcFlagSetting; // edgeMaxZ > threshold
  edgeImbalance: QcFlagSetting; // edgeMaeRatio > threshold
  nonphysical: QcFlagSetting; // !isFinite(r) || !isFinite(k)
  degenerateDeltaI: QcFlagSetting; // |i - i0| < threshold
}

/** §9 defaults: all flags on for highlighting; only non-physical and degenerate ΔI exclude from smoothing by default. */
export function defaultQcConfig(): QcConfig {
  return {
    poorFit: { enabled: true, threshold: 0.98, excludeFromSmoothing: false },
    tooFewPoints: { enabled: true, threshold: 5, excludeFromSmoothing: false },
    edgeCurvature: { enabled: true, threshold: 3, excludeFromSmoothing: false },
    edgeImbalance: { enabled: true, threshold: 2, excludeFromSmoothing: false },
    nonphysical: { enabled: true, threshold: 0, excludeFromSmoothing: true },
    degenerateDeltaI: { enabled: true, threshold: 1e-9, excludeFromSmoothing: true },
  };
}

/** Keyed by `` `${groupId}:${rest}` `` (§9: "keyed by group + rest id") -- survives any re-run since it's semantic, not positional. */
export type ManualExclusions = Record<string, "include" | "exclude">;

export type QcTableFilter = "all" | "flagged" | "excluded";

// ---------------------------------------------------------------------
// §11.4 Additional plots -- any numeric AnalysisTable column plus the 5
// Stage B-derived columns, usable as either axis or the error-bar column.
// ---------------------------------------------------------------------

export type PlotColumnKey =
  | "groupId"
  | "cycN"
  | "rest"
  | "t"
  | "stepT"
  | "e"
  | "i"
  | "q"
  | "e0"
  | "e0Err"
  | "s"
  | "sErr"
  | "i0"
  | "nPts"
  | "r2"
  | "adjR2"
  | "rmse"
  | "edgeMaeRatio"
  | "edgeMaxZ"
  | "r"
  | "rErr"
  | "k"
  | "kErr"
  | "e0Smooth"
  | "kSmooth"
  | "rSmooth"
  | "dVdQ"
  | "dQdV";

export interface AdditionalPlotConfig {
  id: string;
  xColumn: PlotColumnKey;
  yColumn: PlotColumnKey;
  mode: "points" | "line" | "both";
  errorColumn: PlotColumnKey | null;
}

/** Defaults to Q vs R with error bars, so a freshly-added panel isn't blank. */
export function defaultAdditionalPlot(): AdditionalPlotConfig {
  return { id: crypto.randomUUID(), xColumn: "q", yColumn: "r", mode: "points", errorColumn: "rErr" };
}

// ---------------------------------------------------------------------
// §12 Export -- config export/load, run report, and the derived run log.
// ---------------------------------------------------------------------

export const CONFIG_SCHEMA_VERSION = 1;

/** §12.2: everything needed to reproduce a run against the same input file. No data. Deliberately excludes built-in-plot view state (axis ranges/log-scale/x-column) -- established in milestone 11 as ephemeral, never persisted to `Dataset`. */
export interface ExportedConfig {
  schemaVersion: number;
  /** `hashHeaderRow(columns)` at export time (§5.1's existing mechanism) -- §12.5's "header signature" match for session restore. */
  headerSignature: string;
  mapping: ColumnMapping;
  groupingColumns: string[];
  qAnchoring: AnalysisConfig["qAnchoring"];
  stageAConfig: StageAConfig;
  stageBConfig: StageBConfig;
  qcConfig: QcConfig;
  manualExclusions: ManualExclusions;
  additionalPlots: AdditionalPlotConfig[];
  electrodeAreaCm2: number;
  normalizeToArea: boolean;
  absoluteQ: boolean;
  absoluteDVDQ: boolean;
  regressionWindow: RegressionWindow;
  optimalWindowConfig: OptimalWindowConfig;
}

/** §12.3: "timestamped, ordered list of every automatic decision and warning" -- derived from already-structured result data at stage-completion granularity (not per-individual-decision), so the same function backs both the UI panel and the export ("the same object"). */
export interface LogEntry {
  timestamp: number;
  category: "parse" | "segmentation" | "stageA" | "stageB" | "optimalWindow";
  severity: "info" | "warning";
  message: string;
}

/** §11.6's per-smoothing-group stats, machine-readable (§12.3's "key statistics"). Raw/un-normalized -- §11.5's area normalisation is presentation-only, applied by callers at render/format time, same pattern as results-table.ts. */
export interface SummaryStatsRow {
  groupLabel: string;
  n: number;
  rMedian: number;
  rQ1: number;
  rQ3: number;
  kMedian: number;
  kQ1: number;
  kQ3: number;
  medianAdjR2: number;
  medianNPts: number;
  rStart: number;
  rMid: number;
  rEnd: number;
  kStart: number;
  kMid: number;
  kEnd: number;
  qMin: number | null;
  qMax: number | null;
  e0Spline: SplineDiagnostics | null;
  kSpline: SplineDiagnostics | null;
  rSpline: SplineDiagnostics | null;
}

export interface RunReportProvenance {
  filename: string;
  sizeBytes: number;
  fileSha256: string | null;
  report: ParseReport | null;
  columns: string[];
  appVersion: string;
  gitCommit: string;
}

export interface RunReportTimings {
  parseMs: number | null;
  stageAMs: number | null;
  stageBMs: number | null;
  optimalWindowMs: number | null;
}

/** §12.3: the full config (§12.2) plus provenance, the derived log, key statistics, and per-stage timings. */
export interface RunReport {
  config: ExportedConfig;
  provenance: RunReportProvenance;
  log: LogEntry[];
  keyStatistics: SummaryStatsRow[];
  timings: RunReportTimings;
}

export interface Dataset {
  id: string;
  filename: string;
  sizeBytes: number;
  status: "parsing" | "parsed" | "error";
  report?: ParseReport;
  parseError?: ParseErrorInfo;
  overrides: ParseOverrides;
  columns?: ColumnInfo[];
  mapping: ColumnMapping;
  groupingColumns: string[];
  prenormalizedChargeNote?: boolean;
  validation: ValidationIssue[];
  /** `null` once mapping/validation makes segmentation runnable but it hasn't completed yet; `undefined` before that. */
  segmentation?: SegmentationSummary | null;
  /** Rest numbering restarts at 1 *per group* (§7.2) -- `selectedRestId` alone is ambiguous whenever more than one group exists, so it's always paired with this. */
  selectedGroupId: number | null;
  selectedRestId: number | null;
  regressionWindow: RegressionWindow;
  stageAConfig: StageAConfig;
  stageBConfig: StageBConfig;
  stageAResult: StageAResult | null;
  stageAError: string | null;
  /** Incremented on every successful Stage A run -- lets the debounced Stage B effect (stage-runner.ts) tell "Stage A finished again" apart from "nothing changed", without diffing the (possibly large) result. */
  stageARunId: number;
  stageBResult: StageBResult | null;
  optimalWindowConfig: OptimalWindowConfig;
  optimalWindowResult: OptimalWindowResult | null;
  optimalWindowError: string | null;
  qcConfig: QcConfig;
  manualExclusions: ManualExclusions;
  /** UI-only: set by clicking the QC panel's flagged/excluded counts (§9: "click-through to filter the table"). */
  qcTableFilter: QcTableFilter;
  /** §11.4 user-defined plots, in display order. */
  additionalPlots: AdditionalPlotConfig[];
  /** §11.5 electrode area, cm² -- a sample property like column mapping, not a top-level methodology choice like qAnchoring. */
  electrodeAreaCm2: number;
  normalizeToArea: boolean;
  /** Presentation-only abs() over Q -- §6 anchoring legitimately crosses zero between branches; this avoids the "doubled-back" visual confusion without touching the underlying anchored data. */
  absoluteQ: boolean;
  /** Presentation-only abs() over dV/dQ (its reciprocal, dQ/dV, keeps its natural charge/discharge sign). */
  absoluteDVDQ: boolean;
  /** §12.3 provenance: SHA-256 of the original file bytes, computed once in the worker (`parseFile`), reused (not recomputed) across `reparse` calls. */
  fileSha256: string | null;
  /** §12.3 log timestamps -- `Date.now()` snapshots, one per stage-run, not per individual decision. */
  parseCompletedAt: number | null;
  stageACompletedAt: number | null;
  stageBCompletedAt: number | null;
  optimalWindowCompletedAt: number | null;
  /** §12.3 per-stage timings, measured around each stage's actual worker call. */
  parseTimingMs: number | null;
  stageATimingMs: number | null;
  stageBTimingMs: number | null;
  optimalWindowTimingMs: number | null;
}

export interface AnalysisConfig {
  qAnchoring: {
    charge: "start" | "end";
    discharge: "start" | "end";
  };
}

export interface AppState {
  datasets: Dataset[];
  activeDatasetId: string | null;
  analysisConfig: AnalysisConfig;
  /** §12.5: a restored config with nowhere to live yet (config normally lives on `Dataset`, but restore can happen before any file is loaded). Applied to the next dropped file in `import-panel.ts`, then cleared. */
  pendingRestoredConfig: ExportedConfig | null;
}

export function defaultAnalysisConfig(): AnalysisConfig {
  return { qAnchoring: { charge: "start", discharge: "start" } };
}

export interface RestBoundary {
  index: number;
  restId: number;
  groupId: number;
  tStart: number;
  tEnd: number;
  /** Sorted ascending. Rest-only step.t values, for §10.1's live point-count feedback. */
  restStepTs: number[];
}

export interface ThresholdSuggestion {
  groupId: number;
  suggestedThreshold: number;
}

export interface SegmentationSummary {
  totalRests: number;
  restBoundaries: RestBoundary[];
  /** §12.3's run-log wants these named -- straight from core's `SegmentLog`, previously computed and discarded. */
  reversalRowsDropped: number;
  incompleteFinalRowsDropped: number;
  /** §7.2: a leading Rest run with no preceding active run, dropped and flagged rather than left to fail regression. */
  leadingRestRowsDropped: number;
  /** §7.1: non-empty when a group has no rests or no active samples at the current threshold. */
  thresholdSuggestions: ThresholdSuggestion[];
  /** Rows dropped by `StageAConfig`'s ICI-cycle detection -- outside "the ICI cycle" itself (a capacity-check cycle, an OCV rest, a DCIR leg, ...). */
  nonIciRowsDropped: number;
}

/** Mirrors wasm's `GroupKeyColumnsDto` (§12.1's TSV export needs each row's actual grouping-column string values, not just the synthetic `groupId` int). Keyed by `groupId` as a string. */
export interface GroupKeyColumns {
  groupingColumnNames: string[];
  values: Record<string, string[]>;
}

export interface RestPoints {
  activeStepT: number[];
  activeVoltage: number[];
  activeCurrent: number[];
  restStepT: number[];
  restVoltage: number[];
  restCurrent: number[];
}

export interface DecimatedSeries {
  t: number[];
  e: number[];
  i: number[];
}

/** Mirrors wasm's `RestPreviewDto` (§10.2 live single-rest fit). `null` fields mean the fit failed -- see `error`. */
export interface RestPreview {
  ok: boolean;
  nPointsInWindow: number;
  e: number | null;
  i: number | null;
  i0: number | null;
  e0: number | null;
  e0Err: number | null;
  s: number | null;
  sErr: number | null;
  nPts: number | null;
  r2: number | null;
  adjR2: number | null;
  rmse: number | null;
  edgeMaeRatio: number | null;
  edgeMaxZ: number | null;
  r: number | null;
  rErr: number | null;
  k: number | null;
  kErr: number | null;
  error: string | null;
}
