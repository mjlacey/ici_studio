// The second, always-resident WASM instance (§2.1): lives on the main
// thread, independent of the worker's instance (no shared memory), so
// dragging the regression window (§10.2) gets an immediate fit without a
// worker round trip.

import init, { fitRestPreview } from "../wasm/pkg/ici_wasm.js";
import type { RestPoints, RestPreview, StageAConfig } from "../types";

let wasmReady: Promise<void> | null = null;
function ensureWasm(): Promise<void> {
  wasmReady ??= init().then(() => undefined);
  return wasmReady;
}

export async function fitRestPreviewLive(
  points: RestPoints,
  tMin: number,
  tMax: number,
  stageAConfig: StageAConfig,
): Promise<RestPreview> {
  await ensureWasm();
  const json = fitRestPreview(
    new Float64Array(points.activeStepT),
    new Float64Array(points.activeVoltage),
    new Float64Array(points.activeCurrent),
    new Float64Array(points.restStepT),
    new Float64Array(points.restVoltage),
    new Float64Array(points.restCurrent),
    tMin,
    tMax,
    stageAConfig.edgePoints,
    stageAConfig.voltageInterpolationWindow ?? undefined,
    stageAConfig.currentAverageWindow ?? undefined,
  );
  return JSON.parse(json) as RestPreview;
}
