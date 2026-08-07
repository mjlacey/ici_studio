// §12.3: the full config (§12.2) plus provenance (including SHA-256 and
// build-time app version/git commit), the derived run log ("the same
// object" as the UI panel), key statistics, and per-stage timings.

import { buildRunLog } from "../analysis/run-log";
import { computeSummaryStats } from "../analysis/summary-stats";
import type { AnalysisConfig, Dataset, RunReport } from "../types";
import { buildConfigExport } from "./config";

export function buildRunReport(dataset: Dataset, analysisConfig: AnalysisConfig): RunReport {
  return {
    config: buildConfigExport(dataset, analysisConfig),
    provenance: {
      filename: dataset.filename,
      sizeBytes: dataset.sizeBytes,
      fileSha256: dataset.fileSha256,
      report: dataset.report ?? null,
      columns: (dataset.columns ?? []).map((c) => c.name),
      appVersion: import.meta.env.VITE_APP_VERSION,
      gitCommit: import.meta.env.VITE_GIT_COMMIT,
    },
    log: buildRunLog(dataset),
    keyStatistics: computeSummaryStats(dataset),
    timings: {
      parseMs: dataset.parseTimingMs,
      stageAMs: dataset.stageATimingMs,
      stageBMs: dataset.stageBTimingMs,
      optimalWindowMs: dataset.optimalWindowTimingMs,
    },
  };
}
