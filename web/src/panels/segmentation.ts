// Reactive segmentation (§7.1-7.5 state/rest/step.t) -- Plot 1 and Plot 2
// (milestone 7) only need segmentation, not the full Stage A regression, so
// this runs automatically whenever the mapping is complete and unblocked,
// rather than behind a "run" button (that's milestone 8's Stage A panel).

import { activeDataset, store, updateDataset } from "../state";
import { REQUIRED_FIELDS, type AnalysisConfig, type Dataset } from "../types";
import { scaleFactor } from "../units";
import { hasBlockingIssues } from "./validation";
import type { DataWorkerClient } from "../worker/client";
import type { SegmentationInput } from "../worker/protocol";

/** Null once mapping is incomplete or blocked -- segmentation can't run. */
function segmentationKey(dataset: Dataset, qAnchoring: AnalysisConfig["qAnchoring"]): string | null {
  if (dataset.status !== "parsed" || !dataset.columns) return null;
  if (!REQUIRED_FIELDS.every((f) => dataset.mapping[f].column)) return null;
  if (hasBlockingIssues(dataset.validation)) return null;
  return JSON.stringify({
    report: dataset.report,
    mapping: dataset.mapping,
    groupingColumns: dataset.groupingColumns,
    stateThreshold: dataset.stageAConfig.stateThreshold,
    dropUnrestedReversals: dataset.stageAConfig.dropUnrestedReversals,
    nonIciDetectionEnabled: dataset.stageAConfig.nonIciDetectionEnabled,
    nonIciMaxRestDurationS: dataset.stageAConfig.nonIciMaxRestDurationS,
    nonIciMinRepeatCount: dataset.stageAConfig.nonIciMinRepeatCount,
    qAnchoring,
  });
}

export function segmentationInputFor(dataset: Dataset, qAnchoring: AnalysisConfig["qAnchoring"]): SegmentationInput {
  const { mapping, groupingColumns, stageAConfig } = dataset;
  return {
    timeColumn: mapping.time.column!,
    timeScale: scaleFactor("time", mapping.time.unit),
    cycleColumn: mapping.cycle.column!,
    currentColumn: mapping.current.column!,
    currentScale: scaleFactor("current", mapping.current.unit),
    voltageColumn: mapping.voltage.column!,
    voltageScale: scaleFactor("voltage", mapping.voltage.unit),
    chargeColumn: mapping.charge.column!,
    chargeScale: scaleFactor("charge", mapping.charge.unit),
    groupingColumns,
    stateThreshold: stageAConfig.stateThreshold,
    dropUnrestedReversals: stageAConfig.dropUnrestedReversals,
    nonIciDetectionEnabled: stageAConfig.nonIciDetectionEnabled,
    nonIciMaxRestDurationS: stageAConfig.nonIciMaxRestDurationS,
    nonIciMinRepeatCount: stageAConfig.nonIciMinRepeatCount,
    chargeAnchor: qAnchoring.charge,
    dischargeAnchor: qAnchoring.discharge,
  };
}

const lastKeyByDataset = new Map<string, string | null>();

export function startSegmentationEffect(worker: DataWorkerClient): void {
  store.subscribe(() => void checkAndRun(worker));
  void checkAndRun(worker);
}

async function checkAndRun(worker: DataWorkerClient): Promise<void> {
  const state = store.get();
  const dataset = activeDataset(state);
  if (!dataset) return;
  const qAnchoring = state.analysisConfig.qAnchoring;

  const key = segmentationKey(dataset, qAnchoring);
  if (key === lastKeyByDataset.get(dataset.id)) return;
  lastKeyByDataset.set(dataset.id, key);

  if (!key) {
    if (dataset.segmentation !== undefined) {
      updateDataset(dataset.id, (d) => ({ ...d, segmentation: undefined, selectedGroupId: null, selectedRestId: null }));
    }
    return;
  }

  const outcome = await worker.runSegmentation(dataset.id, segmentationInputFor(dataset, qAnchoring));

  // Discard a stale response: the mapping/config moved on again while this
  // request was in flight.
  const current = activeDataset(store.get());
  if (!current || current.id !== dataset.id || segmentationKey(current, store.get().analysisConfig.qAnchoring) !== key) return;

  if (!outcome.ok) {
    updateDataset(dataset.id, (d) => ({ ...d, segmentation: null, selectedGroupId: null, selectedRestId: null }));
    return;
  }

  updateDataset(dataset.id, (d) => {
    const stillValid =
      d.selectedGroupId !== null &&
      d.selectedRestId !== null &&
      outcome.summary.restBoundaries.some((b) => b.groupId === d.selectedGroupId && b.restId === d.selectedRestId);
    const fallback = outcome.summary.restBoundaries[0] ?? null;
    return {
      ...d,
      segmentation: outcome.summary,
      selectedGroupId: stillValid ? d.selectedGroupId : (fallback?.groupId ?? null),
      selectedRestId: stillValid ? d.selectedRestId : (fallback?.restId ?? null),
    };
  });
}
