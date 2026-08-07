// Stage A/B orchestration (§7/§8). Stage A only runs on an explicit "Fit
// resistances" click -- never reactively, matching the spec's own reason
// ("so changing a smoothing parameter doesn't re-run 502 regressions").
// Stage B auto-runs after Stage A completes *and* on any Stage-B-only
// parameter change, both funnelled through the same debounced effect below
// (§8: "with Stage B auto-running after Stage A and also on any Stage-B
// parameter change (debounced ~300ms)") -- treating "Stage A just
// finished" as one more trigger for the same debounced re-run avoids a
// double round trip from also calling it explicitly.

import { applyDqdvSignConvention } from "./dqdv-sign-convention";
import { computeExcludedIndices } from "./qc";
import { segmentationInputFor } from "../panels/segmentation";
import { activeDataset, store, updateDataset } from "../state";
import type { Dataset } from "../types";
import type { DataWorkerClient } from "../worker/client";

export async function runStageA(datasetId: string, worker: DataWorkerClient): Promise<void> {
  const state = store.get();
  const dataset = state.datasets.find((d) => d.id === datasetId);
  if (!dataset) return;

  const input = segmentationInputFor(dataset, state.analysisConfig.qAnchoring);
  const { stageAConfig, regressionWindow } = dataset;

  const startedAt = performance.now();
  const outcome = await worker.runStageA(datasetId, input, {
    tMin: regressionWindow.tMin,
    tMax: regressionWindow.tMax,
    voltageInterpWindow: stageAConfig.voltageInterpolationWindow,
    currentAvgWindow: stageAConfig.currentAverageWindow,
    edgePoints: stageAConfig.edgePoints,
    legacyCompatibility: stageAConfig.legacyCompatibility,
  });

  if (!store.get().datasets.some((d) => d.id === datasetId)) return; // dataset gone

  if (!outcome.ok) {
    updateDataset(datasetId, (d) => ({ ...d, stageAError: outcome.error, stageAResult: null, stageBResult: null }));
    return;
  }

  updateDataset(datasetId, (d) => ({
    ...d,
    stageAResult: outcome.result,
    stageAError: null,
    stageARunId: d.stageARunId + 1,
    stageACompletedAt: Date.now(),
    stageATimingMs: performance.now() - startedAt,
  }));
}

export async function runStageB(datasetId: string, worker: DataWorkerClient): Promise<void> {
  const dataset = store.get().datasets.find((d) => d.id === datasetId);
  if (!dataset?.stageAResult) return;
  const { stageBConfig } = dataset;
  const excludedRowIndices = computeExcludedIndices(dataset.stageAResult.analysisTable, dataset.qcConfig, dataset.manualExclusions);

  const startedAt = performance.now();
  const outcome = await worker.runStageB(datasetId, {
    smoothingKey: stageBConfig.smoothingKey,
    e0Smoothing: stageBConfig.e0Smoothing,
    kSmoothing: stageBConfig.kSmoothing,
    rInheritsK: stageBConfig.rInheritsK,
    rSmoothing: stageBConfig.rSmoothing,
    derivativeWindow: stageBConfig.derivativeWindow,
    derivativeDegree: stageBConfig.derivativeDegree,
    excludedRowIndices,
  });

  if (!store.get().datasets.some((d) => d.id === datasetId)) return; // dataset gone

  // A Stage B failure here means runStageA hasn't produced a result yet --
  // the `dataset?.stageAResult` guard above should already prevent that;
  // silently skip rather than surface a confusing error for a state the UI
  // can't actually reach.
  if (outcome.ok) {
    updateDataset(datasetId, (d) => ({
      ...d,
      stageBResult: d.stageAResult ? applyDqdvSignConvention(outcome.result, d.stageAResult.analysisTable) : outcome.result,
      stageBCompletedAt: Date.now(),
      stageBTimingMs: performance.now() - startedAt,
    }));
  }
}

const STAGE_B_DEBOUNCE_MS = 300;
const lastStageBKey = new Map<string, string | null>();
const pendingTimers = new Map<string, ReturnType<typeof setTimeout>>();

/** `null` until Stage A has produced a result; otherwise changes whenever Stage A completes again, `stageBConfig` changes, or QC's flags/manual overrides change what's excluded from smoothing. */
function stageBKey(dataset: Dataset): string | null {
  if (!dataset.stageAResult) return null;
  return `${dataset.stageARunId}:${JSON.stringify(dataset.stageBConfig)}:${JSON.stringify(dataset.qcConfig)}:${JSON.stringify(dataset.manualExclusions)}`;
}

export function startStageBEffect(worker: DataWorkerClient): void {
  store.subscribe(() => checkStageBConfig(worker));
  checkStageBConfig(worker);
}

function checkStageBConfig(worker: DataWorkerClient): void {
  const dataset = activeDataset(store.get());
  if (!dataset) return;

  const key = stageBKey(dataset);
  if (key === lastStageBKey.get(dataset.id)) return;
  lastStageBKey.set(dataset.id, key);
  if (key === null) return; // no Stage A result yet

  const datasetId = dataset.id;
  const existingTimer = pendingTimers.get(datasetId);
  if (existingTimer) clearTimeout(existingTimer);
  pendingTimers.set(
    datasetId,
    setTimeout(() => {
      pendingTimers.delete(datasetId);
      void runStageB(datasetId, worker);
    }, STAGE_B_DEBOUNCE_MS),
  );
}
