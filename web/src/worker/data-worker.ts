// The one dedicated data worker (§2.1). Owns every dataset's raw bytes and
// its WASM-side `ParsedDataset` for the life of the session; only small
// JSON summaries ever cross back to the main thread.

import init, { parse_file, parse_mf4_file, type ParsedDataset } from "../wasm/pkg/ici_wasm.js";
import type { OptimalWindowResult, ParseErrorInfo, ParseOverrides } from "../types";
import type { WorkerRequest, WorkerResponse } from "./protocol";

let wasmReady: Promise<void> | null = null;
function ensureWasm(): Promise<void> {
  wasmReady ??= init().then(() => undefined);
  return wasmReady;
}

interface DatasetEntry {
  bytes: Uint8Array;
  parsed: ParsedDataset | null;
  /** §12.3 provenance: computed once from the original bytes at `parseFile` time, reused (not recomputed) across `reparse` calls -- reparse operates on the same bytes with different sniffer overrides. */
  sha256: string;
  /** Which wasm entry point owns these bytes -- an MDF4 file has no sniffer overrides to reparse with, so `runParse` ignores `overrides` and calls `parse_mf4_file` instead when this is `"mdf4"`. */
  format: "text" | "mdf4";
}

async function sha256Hex(bytes: Uint8Array): Promise<string> {
  // `bytes` is always backed by a plain ArrayBuffer here (never SharedArrayBuffer),
  // but TS's DOM lib types `Uint8Array`'s buffer as the broader `ArrayBufferLike`.
  const digest = await crypto.subtle.digest("SHA-256", bytes as Uint8Array<ArrayBuffer>);
  return Array.from(new Uint8Array(digest))
    .map((b) => b.toString(16).padStart(2, "0"))
    .join("");
}

const datasets = new Map<string, DatasetEntry>();

// §10.3: `estimateOptimalWindow` processes its candidate grid in chunks so
// it can be cancelled mid-flight -- a `cancelOptimalWindow` message just
// adds to this set; the chunk loop below checks it between chunks (it can
// only be checked because the loop `await`s a macrotask yield each
// iteration, letting this worker's single `onmessage` interleave the two
// message handlers).
const cancelledOptimalWindowRequests = new Set<string>();
const OPTIMAL_WINDOW_CHUNK_SIZE = 25;

function yieldToEventLoop(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, 0));
}

function post(message: WorkerResponse): void {
  (self as unknown as Worker).postMessage(message);
}

function parseErrorFromThrown(err: unknown): ParseErrorInfo {
  if (typeof err === "string") {
    try {
      return JSON.parse(err) as ParseErrorInfo;
    } catch {
      return { message: err };
    }
  }
  return { message: err instanceof Error ? err.message : String(err) };
}

function runParse(datasetId: string, bytes: Uint8Array, overrides: ParseOverrides): void {
  const entry = datasets.get(datasetId);
  if (!entry) return;

  try {
    const parsed = entry.format === "mdf4" ? parse_mf4_file(bytes) : parse_file(bytes, JSON.stringify(overrides));
    entry.parsed?.free();
    entry.parsed = parsed;
    post({
      type: "parseResult",
      datasetId,
      ok: true,
      report: JSON.parse(parsed.reportJson()),
      columns: JSON.parse(parsed.columnsJson()),
      sha256: entry.sha256,
    });
  } catch (err) {
    post({ type: "parseResult", datasetId, ok: false, error: parseErrorFromThrown(err) });
  }
}

self.onmessage = async (ev: MessageEvent<WorkerRequest>) => {
  const msg = ev.data;

  switch (msg.type) {
    case "parseFile": {
      await ensureWasm();
      const bytes = new Uint8Array(msg.bytes);
      datasets.get(msg.datasetId)?.parsed?.free();
      const sha256 = await sha256Hex(bytes);
      datasets.set(msg.datasetId, { bytes, parsed: null, sha256, format: msg.format });
      runParse(msg.datasetId, bytes, msg.overrides);
      break;
    }
    case "reparse": {
      await ensureWasm();
      const entry = datasets.get(msg.datasetId);
      if (!entry) return;
      runParse(msg.datasetId, entry.bytes, msg.overrides);
      break;
    }
    case "getPage": {
      const entry = datasets.get(msg.datasetId);
      if (!entry?.parsed) return;
      const page = JSON.parse(entry.parsed.pageJson(msg.offset, msg.limit));
      post({ type: "pageResult", datasetId: msg.datasetId, requestId: msg.requestId, page });
      break;
    }
    case "getColumnStats": {
      const entry = datasets.get(msg.datasetId);
      if (!entry?.parsed) return;
      const stats = JSON.parse(entry.parsed.columnStatsJson(msg.column));
      post({ type: "columnStatsResult", datasetId: msg.datasetId, requestId: msg.requestId, stats });
      break;
    }
    case "checkTimeMonotonic": {
      const entry = datasets.get(msg.datasetId);
      if (!entry?.parsed) return;
      const result = JSON.parse(entry.parsed.checkTimeMonotonicJson(msg.column));
      post({ type: "monotonicResult", datasetId: msg.datasetId, requestId: msg.requestId, result });
      break;
    }
    case "dropNonFiniteRows": {
      const entry = datasets.get(msg.datasetId);
      if (!entry?.parsed) return;
      const indices = entry.parsed.nonFiniteRowIndices(msg.column);
      entry.parsed.dropRows(indices);
      post({
        type: "mutateResult",
        datasetId: msg.datasetId,
        requestId: msg.requestId,
        report: JSON.parse(entry.parsed.reportJson()),
        columns: JSON.parse(entry.parsed.columnsJson()),
      });
      break;
    }
    case "dropDecreasingRows": {
      const entry = datasets.get(msg.datasetId);
      if (!entry?.parsed) return;
      const indices = entry.parsed.decreasingRowIndices(msg.column);
      entry.parsed.dropRows(indices);
      post({
        type: "mutateResult",
        datasetId: msg.datasetId,
        requestId: msg.requestId,
        report: JSON.parse(entry.parsed.reportJson()),
        columns: JSON.parse(entry.parsed.columnsJson()),
      });
      break;
    }
    case "sortByColumn": {
      const entry = datasets.get(msg.datasetId);
      if (!entry?.parsed) return;
      entry.parsed.sortByColumn(msg.column);
      post({
        type: "mutateResult",
        datasetId: msg.datasetId,
        requestId: msg.requestId,
        report: JSON.parse(entry.parsed.reportJson()),
        columns: JSON.parse(entry.parsed.columnsJson()),
      });
      break;
    }
    case "runSegmentation": {
      const entry = datasets.get(msg.datasetId);
      if (!entry?.parsed) return;
      try {
        const summary = JSON.parse(
          entry.parsed.runSegmentation(
            msg.timeColumn,
            msg.timeScale,
            msg.cycleColumn,
            msg.currentColumn,
            msg.currentScale,
            msg.voltageColumn,
            msg.voltageScale,
            msg.chargeColumn,
            msg.chargeScale,
            JSON.stringify(msg.groupingColumns),
            msg.stateThreshold,
            msg.dropUnrestedReversals,
            msg.chargeAnchor,
            msg.dischargeAnchor,
            msg.nonIciDetectionEnabled,
            msg.nonIciMaxRestDurationS,
            msg.nonIciMinRepeatCount,
          ),
        );
        post({ type: "segmentationResult", datasetId: msg.datasetId, requestId: msg.requestId, ok: true, summary });
      } catch (err) {
        const error = err instanceof Error ? err.message : String(err);
        post({ type: "segmentationResult", datasetId: msg.datasetId, requestId: msg.requestId, ok: false, error });
      }
      break;
    }
    case "runStageA": {
      const entry = datasets.get(msg.datasetId);
      if (!entry?.parsed) return;
      try {
        const result = JSON.parse(
          entry.parsed.runStageA(
            msg.timeColumn,
            msg.timeScale,
            msg.cycleColumn,
            msg.currentColumn,
            msg.currentScale,
            msg.voltageColumn,
            msg.voltageScale,
            msg.chargeColumn,
            msg.chargeScale,
            JSON.stringify(msg.groupingColumns),
            msg.stateThreshold,
            msg.dropUnrestedReversals,
            msg.chargeAnchor,
            msg.dischargeAnchor,
            msg.nonIciDetectionEnabled,
            msg.nonIciMaxRestDurationS,
            msg.nonIciMinRepeatCount,
            msg.tMin,
            msg.tMax,
            msg.voltageInterpWindow ?? undefined,
            msg.currentAvgWindow ?? undefined,
            msg.edgePoints,
            msg.legacyCompatibility,
          ),
        );
        post({ type: "stageAResult", datasetId: msg.datasetId, requestId: msg.requestId, ok: true, result });
      } catch (err) {
        const error = err instanceof Error ? err.message : String(err);
        post({ type: "stageAResult", datasetId: msg.datasetId, requestId: msg.requestId, ok: false, error });
      }
      break;
    }
    case "runStageB": {
      const entry = datasets.get(msg.datasetId);
      if (!entry?.parsed) return;
      try {
        const result = JSON.parse(
          entry.parsed.runStageB(
            JSON.stringify(msg.smoothingKey),
            JSON.stringify(msg.e0Smoothing),
            JSON.stringify(msg.kSmoothing),
            msg.rInheritsK,
            JSON.stringify(msg.rSmoothing),
            msg.derivativeWindow,
            msg.derivativeDegree,
            JSON.stringify(msg.excludedRowIndices),
          ),
        );
        post({ type: "stageBResult", datasetId: msg.datasetId, requestId: msg.requestId, ok: true, result });
      } catch (err) {
        const error = err instanceof Error ? err.message : String(err);
        post({ type: "stageBResult", datasetId: msg.datasetId, requestId: msg.requestId, ok: false, error });
      }
      break;
    }
    case "getRestPoints": {
      const entry = datasets.get(msg.datasetId);
      if (!entry?.parsed) return;
      const points = JSON.parse(entry.parsed.restPointsJson(msg.groupId, msg.restId));
      post({ type: "restPointsResult", datasetId: msg.datasetId, requestId: msg.requestId, points });
      break;
    }
    case "getDecimatedSeries": {
      const entry = datasets.get(msg.datasetId);
      if (!entry?.parsed) return;
      const series = JSON.parse(entry.parsed.decimatedSeriesJson(msg.xMin, msg.xMax, msg.targetPoints));
      post({ type: "decimatedSeriesResult", datasetId: msg.datasetId, requestId: msg.requestId, series });
      break;
    }
    case "estimateOptimalWindow": {
      const entry = datasets.get(msg.datasetId);
      if (!entry?.parsed) return;
      const { datasetId, requestId } = msg;
      try {
        const setup = JSON.parse(entry.parsed.setupOptimalWindow(msg.n, msg.lMin, msg.tMinLowerBound)) as {
          sampledRests: OptimalWindowResult["sampledRests"];
          tMaxObserved: number;
          heterogeneousLengths: boolean;
          totalCandidates: number;
        };
        const total = setup.totalCandidates;
        const scores: OptimalWindowResult["scores"] = [];

        for (let start = 0; start < total; start += OPTIMAL_WINDOW_CHUNK_SIZE) {
          if (cancelledOptimalWindowRequests.has(requestId)) {
            cancelledOptimalWindowRequests.delete(requestId);
            post({ type: "optimalWindowResult", datasetId, requestId, ok: false, error: "cancelled" });
            return;
          }
          const chunk = JSON.parse(entry.parsed.scoreOptimalWindowChunk(start, OPTIMAL_WINDOW_CHUNK_SIZE, msg.edgePoints));
          scores.push(...chunk);
          const completed = Math.min(start + OPTIMAL_WINDOW_CHUNK_SIZE, total);
          post({ type: "optimalWindowProgress", datasetId, requestId, completed, total });
          await yieldToEventLoop();
        }

        cancelledOptimalWindowRequests.delete(requestId);
        post({
          type: "optimalWindowResult",
          datasetId,
          requestId,
          ok: true,
          result: {
            sampledRests: setup.sampledRests,
            tMaxObserved: setup.tMaxObserved,
            heterogeneousLengths: setup.heterogeneousLengths,
            scores,
          },
        });
      } catch (err) {
        cancelledOptimalWindowRequests.delete(requestId);
        const error = err instanceof Error ? err.message : String(err);
        post({ type: "optimalWindowResult", datasetId, requestId, ok: false, error });
      }
      break;
    }
    case "cancelOptimalWindow": {
      cancelledOptimalWindowRequests.add(msg.requestId);
      break;
    }
    case "getGroupKeyColumns": {
      const entry = datasets.get(msg.datasetId);
      if (!entry?.parsed) return;
      let result = null;
      try {
        result = JSON.parse(entry.parsed.groupKeyColumnsJson());
      } catch {
        // Segmentation hasn't run yet -- report "no data" rather than an error.
      }
      post({ type: "groupKeyColumnsResult", datasetId: msg.datasetId, requestId: msg.requestId, result });
      break;
    }
    case "detectPrenormalizedCharge": {
      const entry = datasets.get(msg.datasetId);
      if (!entry?.parsed) return;
      let looksPrenormalized = false;
      try {
        looksPrenormalized = entry.parsed.detectPrenormalizedCharge(
          msg.charge,
          msg.cycle,
          msg.current,
          msg.stateThreshold,
        );
      } catch {
        // Column not found/not numeric yet (e.g. mapping still in
        // progress) -- just report "no note", not an error.
      }
      post({ type: "prenormalizedResult", datasetId: msg.datasetId, requestId: msg.requestId, looksPrenormalized });
      break;
    }
  }
};
