// §12.2: config-only JSON export ("everything needed to reproduce a run
// against the same input file. No data.") and the "Load config" import
// path. Deliberately excludes built-in-plot view state (axis ranges/
// log-scale/x-column selection) -- milestone 11 established that as
// ephemeral local-closure UI state, never persisted to `Dataset`; §12.2's
// "the plot panel definitions" reads as `additionalPlots`, the genuinely
// reproducibility-relevant user-defined plots.

import { hashHeaderRow } from "../panels/auto-mapping";
import { CONFIG_SCHEMA_VERSION } from "../types";
import type { AnalysisConfig, Dataset, ExportedConfig } from "../types";

export function buildConfigExport(dataset: Dataset, analysisConfig: AnalysisConfig): ExportedConfig {
  return {
    schemaVersion: CONFIG_SCHEMA_VERSION,
    headerSignature: hashHeaderRow((dataset.columns ?? []).map((c) => c.name)),
    mapping: dataset.mapping,
    groupingColumns: dataset.groupingColumns,
    qAnchoring: analysisConfig.qAnchoring,
    stageAConfig: dataset.stageAConfig,
    stageBConfig: dataset.stageBConfig,
    qcConfig: dataset.qcConfig,
    manualExclusions: dataset.manualExclusions,
    additionalPlots: dataset.additionalPlots,
    electrodeAreaCm2: dataset.electrodeAreaCm2,
    normalizeToArea: dataset.normalizeToArea,
    absoluteQ: dataset.absoluteQ,
    absoluteDVDQ: dataset.absoluteDVDQ,
    regressionWindow: dataset.regressionWindow,
    optimalWindowConfig: dataset.optimalWindowConfig,
  };
}

export interface ConfigImportResult {
  fields: Partial<Dataset>;
  analysisConfig: AnalysisConfig | null;
  applied: string[];
  skipped: string[];
}

/** The column-independent settings -- Stage A/B, QC, additional plots, electrode area, regression window, optimal window config, Q anchoring -- always apply regardless of mapping, for both import paths below. */
function columnIndependentFields(config: ExportedConfig): { fields: Partial<Dataset>; applied: string[] } {
  return {
    fields: {
      stageAConfig: config.stageAConfig,
      stageBConfig: config.stageBConfig,
      qcConfig: config.qcConfig,
      manualExclusions: config.manualExclusions,
      additionalPlots: config.additionalPlots,
      electrodeAreaCm2: config.electrodeAreaCm2,
      normalizeToArea: config.normalizeToArea,
      absoluteQ: config.absoluteQ,
      absoluteDVDQ: config.absoluteDVDQ,
      regressionWindow: config.regressionWindow,
      optimalWindowConfig: config.optimalWindowConfig,
    },
    applied: ["Stage A parameters (including ICI cycle detection)", "Stage B parameters", "QC thresholds and manual exclusions", "additional plots", "electrode area", "absolute-value display toggles", "regression window", "optimal window config", "Q anchoring"],
  };
}

/**
 * §12.2's "Load config" path: a looser, more granular per-column-existence
 * check ("reports any settings that couldn't be applied, e.g. a mapped
 * column absent from the current file") -- distinct from §12.5's session
 * restore (`applyRestoredConfig`, below), which uses a stricter full-
 * header-signature match instead.
 */
export function applyConfigImport(config: ExportedConfig, availableColumns: string[]): ConfigImportResult {
  const applied: string[] = [];
  const skipped: string[] = [];

  if (config.schemaVersion !== CONFIG_SCHEMA_VERSION) {
    skipped.push(`schema version ${config.schemaVersion} is not supported (expected ${CONFIG_SCHEMA_VERSION})`);
    return { fields: {}, analysisConfig: null, applied, skipped };
  }

  const { fields, applied: base } = columnIndependentFields(config);
  applied.push(...base);

  const available = new Set(availableColumns);
  const mappedColumns = Object.values(config.mapping)
    .map((m) => m.column)
    .filter((c): c is string => c !== null);
  const missingMapped = mappedColumns.filter((c) => !available.has(c));

  if (missingMapped.length === 0) {
    fields.mapping = config.mapping;
    applied.push("column mapping");
  } else {
    skipped.push(`column mapping (missing column(s): ${missingMapped.join(", ")})`);
  }

  const missingGrouping = config.groupingColumns.filter((c) => !available.has(c));
  fields.groupingColumns = config.groupingColumns.filter((c) => available.has(c));
  if (missingGrouping.length > 0) {
    skipped.push(`grouping column(s) not found: ${missingGrouping.join(", ")}`);
  } else if (config.groupingColumns.length > 0) {
    applied.push("grouping columns");
  }

  return { fields, analysisConfig: { qAnchoring: config.qAnchoring }, applied, skipped };
}

/**
 * §12.5's session-restore path: "the stored mapping is applied if the
 * header signature matches (§5.1) and reported as unapplied if it does
 * not" -- an all-or-nothing check on the *whole* header (not per-column),
 * stricter than `applyConfigImport`'s per-column existence check. A
 * functionally-compatible header that differs in an unrelated column's
 * name or order still counts as "does not match" here, by design.
 */
export function applyRestoredConfig(config: ExportedConfig, availableColumns: string[]): ConfigImportResult {
  const applied: string[] = [];
  const skipped: string[] = [];

  if (config.schemaVersion !== CONFIG_SCHEMA_VERSION) {
    skipped.push(`schema version ${config.schemaVersion} is not supported (expected ${CONFIG_SCHEMA_VERSION})`);
    return { fields: {}, analysisConfig: null, applied, skipped };
  }

  const { fields, applied: base } = columnIndependentFields(config);
  applied.push(...base);

  if (hashHeaderRow(availableColumns) === config.headerSignature) {
    fields.mapping = config.mapping;
    fields.groupingColumns = config.groupingColumns;
    applied.push("column mapping", "grouping columns");
  } else {
    skipped.push("column mapping (header signature does not match the new file)");
  }

  return { fields, analysisConfig: { qAnchoring: config.qAnchoring }, applied, skipped };
}
