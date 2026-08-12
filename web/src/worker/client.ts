// Main-thread wrapper around the data worker: turns its message protocol
// into a small promise-based API so panels don't touch postMessage/
// onmessage directly.

import type {
  ColumnStats,
  Page,
  ParseErrorInfo,
  ParseOverrides,
  ParseReport,
  ColumnInfo,
  GroupKeyColumns,
  OptimalWindowResult,
  SmootherConfig,
  SmoothingKeyConfig,
  StageAResult,
  StageBResult,
} from "../types";
import type {
  DecimatedSeries,
  Monotonicity,
  RestPoints,
  SegmentationInput,
  SegmentationSummary,
  WorkerRequest,
  WorkerResponse,
} from "./protocol";

export type SegmentationOutcome = { ok: true; summary: SegmentationSummary } | { ok: false; error: string };
export type StageAOutcome = { ok: true; result: StageAResult } | { ok: false; error: string };
export type StageBOutcome = { ok: true; result: StageBResult } | { ok: false; error: string };
export type OptimalWindowOutcome = { ok: true; result: OptimalWindowResult } | { ok: false; error: string };

export type ParseOutcome = { ok: true; report: ParseReport; columns: ColumnInfo[]; sha256: string | null } | { ok: false; error: ParseErrorInfo };

let requestCounter = 0;
function nextRequestId(): string {
  requestCounter += 1;
  return `r${requestCounter}`;
}

export class DataWorkerClient {
  private worker: Worker;
  private pending = new Map<string, (response: WorkerResponse) => void>();
  private parseWaiters = new Map<string, (outcome: ParseOutcome) => void>();

  constructor() {
    this.worker = new Worker(new URL("./data-worker.ts", import.meta.url), { type: "module" });
    this.worker.onmessage = (ev: MessageEvent<WorkerResponse>) => this.handleMessage(ev.data);
  }

  private handleMessage(msg: WorkerResponse): void {
    if (msg.type === "parseResult") {
      const waiter = this.parseWaiters.get(msg.datasetId);
      this.parseWaiters.delete(msg.datasetId);
      if (waiter) waiter(msg.ok ? { ok: true, report: msg.report, columns: msg.columns, sha256: msg.sha256 } : { ok: false, error: msg.error });
      return;
    }
    if ("requestId" in msg) {
      const resolve = this.pending.get(msg.requestId);
      if (!resolve) return;
      // Progress messages don't terminate the request -- the final
      // `optimalWindowResult` (or a cancellation) does.
      if (msg.type !== "optimalWindowProgress") {
        this.pending.delete(msg.requestId);
      }
      resolve(msg);
    }
  }

  private send(msg: WorkerRequest, transfer: Transferable[] = []): void {
    this.worker.postMessage(msg, transfer);
  }

  parseFile(datasetId: string, bytes: ArrayBuffer, overrides: ParseOverrides, format: "text" | "mdf4" = "text"): Promise<ParseOutcome> {
    return new Promise((resolve) => {
      this.parseWaiters.set(datasetId, resolve);
      this.send({ type: "parseFile", datasetId, bytes, overrides, format }, [bytes]);
    });
  }

  reparse(datasetId: string, overrides: ParseOverrides): Promise<ParseOutcome> {
    return new Promise((resolve) => {
      this.parseWaiters.set(datasetId, resolve);
      this.send({ type: "reparse", datasetId, overrides });
    });
  }

  getPage(datasetId: string, offset: number, limit: number): Promise<Page> {
    const requestId = nextRequestId();
    return new Promise((resolve) => {
      this.pending.set(requestId, (msg) => {
        if (msg.type === "pageResult") resolve(msg.page);
      });
      this.send({ type: "getPage", datasetId, requestId, offset, limit });
    });
  }

  getColumnStats(datasetId: string, column: string): Promise<ColumnStats | null> {
    const requestId = nextRequestId();
    return new Promise((resolve) => {
      this.pending.set(requestId, (msg) => {
        if (msg.type === "columnStatsResult") resolve(msg.stats);
      });
      this.send({ type: "getColumnStats", datasetId, requestId, column });
    });
  }

  private mutate(type: "dropNonFiniteRows" | "dropDecreasingRows" | "sortByColumn", datasetId: string, column: string): Promise<ParseOutcome> {
    const requestId = nextRequestId();
    return new Promise((resolve) => {
      this.pending.set(requestId, (msg) => {
        // `sha256` isn't part of `mutateResult` -- row-drop/sort mutations don't change the
        // original file's hash, and callers never read this field from a mutate outcome
        // (Dataset.fileSha256 is set once from the initial parseResult and left untouched).
        if (msg.type === "mutateResult") resolve({ ok: true, report: msg.report, columns: msg.columns, sha256: null });
      });
      this.send({ type, datasetId, requestId, column });
    });
  }

  dropNonFiniteRows(datasetId: string, column: string): Promise<ParseOutcome> {
    return this.mutate("dropNonFiniteRows", datasetId, column);
  }

  dropDecreasingRows(datasetId: string, column: string): Promise<ParseOutcome> {
    return this.mutate("dropDecreasingRows", datasetId, column);
  }

  sortByColumn(datasetId: string, column: string): Promise<ParseOutcome> {
    return this.mutate("sortByColumn", datasetId, column);
  }

  checkTimeMonotonic(datasetId: string, column: string): Promise<Monotonicity> {
    const requestId = nextRequestId();
    return new Promise((resolve) => {
      this.pending.set(requestId, (msg) => {
        if (msg.type === "monotonicResult") resolve(msg.result);
      });
      this.send({ type: "checkTimeMonotonic", datasetId, requestId, column });
    });
  }

  detectPrenormalizedCharge(
    datasetId: string,
    charge: string,
    cycle: string,
    current: string,
    stateThreshold: number,
  ): Promise<boolean> {
    const requestId = nextRequestId();
    return new Promise((resolve) => {
      this.pending.set(requestId, (msg) => {
        if (msg.type === "prenormalizedResult") resolve(msg.looksPrenormalized);
      });
      this.send({ type: "detectPrenormalizedCharge", datasetId, requestId, charge, cycle, current, stateThreshold });
    });
  }

  runSegmentation(datasetId: string, input: SegmentationInput): Promise<SegmentationOutcome> {
    const requestId = nextRequestId();
    return new Promise((resolve) => {
      this.pending.set(requestId, (msg) => {
        if (msg.type === "segmentationResult") {
          resolve(msg.ok ? { ok: true, summary: msg.summary } : { ok: false, error: msg.error });
        }
      });
      this.send({ type: "runSegmentation", datasetId, requestId, ...input });
    });
  }

  runStageA(
    datasetId: string,
    input: SegmentationInput,
    stageA: {
      tMin: number;
      tMax: number;
      voltageInterpWindow: number | null;
      currentAvgWindow: number | null;
      edgePoints: number;
      legacyCompatibility: boolean;
    },
  ): Promise<StageAOutcome> {
    const requestId = nextRequestId();
    return new Promise((resolve) => {
      this.pending.set(requestId, (msg) => {
        if (msg.type === "stageAResult") {
          resolve(msg.ok ? { ok: true, result: msg.result } : { ok: false, error: msg.error });
        }
      });
      this.send({ type: "runStageA", datasetId, requestId, ...input, ...stageA });
    });
  }

  runStageB(
    datasetId: string,
    config: {
      smoothingKey: SmoothingKeyConfig;
      e0Smoothing: SmootherConfig;
      kSmoothing: SmootherConfig;
      rInheritsK: boolean;
      rSmoothing: SmootherConfig;
      derivativeWindow: number;
      derivativeDegree: number;
      excludedRowIndices: number[];
    },
  ): Promise<StageBOutcome> {
    const requestId = nextRequestId();
    return new Promise((resolve) => {
      this.pending.set(requestId, (msg) => {
        if (msg.type === "stageBResult") {
          resolve(msg.ok ? { ok: true, result: msg.result } : { ok: false, error: msg.error });
        }
      });
      this.send({ type: "runStageB", datasetId, requestId, ...config });
    });
  }

  /**
   * §10.3. Unlike every other method here, this isn't a single
   * request/response pair: `onProgress` fires for each chunk the worker
   * completes (without resolving the promise), and the returned `cancel()`
   * sends a `cancelOptimalWindow` the worker's chunk loop checks between
   * chunks -- the promise then resolves `{ok: false, error: "cancelled"}`.
   */
  estimateOptimalWindow(
    datasetId: string,
    params: { n: number; lMin: number; tMinLowerBound: number; edgePoints: number },
    onProgress: (completed: number, total: number) => void,
  ): { promise: Promise<OptimalWindowOutcome>; cancel: () => void } {
    const requestId = nextRequestId();
    const promise = new Promise<OptimalWindowOutcome>((resolve) => {
      this.pending.set(requestId, (msg) => {
        if (msg.type === "optimalWindowProgress") {
          onProgress(msg.completed, msg.total);
          return;
        }
        if (msg.type === "optimalWindowResult") {
          resolve(msg.ok ? { ok: true, result: msg.result } : { ok: false, error: msg.error });
        }
      });
      this.send({ type: "estimateOptimalWindow", datasetId, requestId, ...params });
    });
    const cancel = (): void => this.send({ type: "cancelOptimalWindow", datasetId, requestId });
    return { promise, cancel };
  }

  getRestPoints(datasetId: string, groupId: number, restId: number): Promise<RestPoints | null> {
    const requestId = nextRequestId();
    return new Promise((resolve) => {
      this.pending.set(requestId, (msg) => {
        if (msg.type === "restPointsResult") resolve(msg.points);
      });
      this.send({ type: "getRestPoints", datasetId, requestId, groupId, restId });
    });
  }

  getDecimatedSeries(datasetId: string, xMin: number, xMax: number, targetPoints: number): Promise<DecimatedSeries | null> {
    const requestId = nextRequestId();
    return new Promise((resolve) => {
      this.pending.set(requestId, (msg) => {
        if (msg.type === "decimatedSeriesResult") resolve(msg.series);
      });
      this.send({ type: "getDecimatedSeries", datasetId, requestId, xMin, xMax, targetPoints });
    });
  }

  /** §12.1's TSV export -- each row's actual grouping-column string values, keyed by `groupId`. `null` if segmentation hasn't run yet. */
  getGroupKeyColumns(datasetId: string): Promise<GroupKeyColumns | null> {
    const requestId = nextRequestId();
    return new Promise((resolve) => {
      this.pending.set(requestId, (msg) => {
        if (msg.type === "groupKeyColumnsResult") resolve(msg.result);
      });
      this.send({ type: "getGroupKeyColumns", datasetId, requestId });
    });
  }
}
