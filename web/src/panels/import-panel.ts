// §4.1: drag-and-drop import zone with click-to-browse fallback. One file
// at a time in v1 (§4.5); dropping/choosing a new file replaces the active
// dataset.

import { MAX_FILE_BYTES } from "../constants";
import { applyRestoredConfig } from "../export/config";
import { store } from "../state";
import {
  defaultOptimalWindowConfig,
  defaultQcConfig,
  defaultRegressionWindow,
  defaultStageAConfig,
  defaultStageBConfig,
  emptyMapping,
  type AnalysisConfig,
  type Dataset,
} from "../types";
import { computeAutoMapping, loadMappingPreset } from "./auto-mapping";
import { computeValidation } from "./validation";
import type { DataWorkerClient } from "../worker/client";

export function mountImportPanel(container: HTMLElement, worker: DataWorkerClient): void {
  container.innerHTML = `
    <section class="panel">
      <h2>Import</h2>
      <div class="dropzone" id="dropzone" tabindex="0">
        <p>Drag &amp; drop a file here, or</p>
        <button type="button" id="browse-btn" class="btn">Browse files…</button>
        <input type="file" id="file-input" hidden />
      </div>
      <div id="import-status"></div>
    </section>
  `;

  const dropzone = container.querySelector<HTMLDivElement>("#dropzone")!;
  const browseBtn = container.querySelector<HTMLButtonElement>("#browse-btn")!;
  const fileInput = container.querySelector<HTMLInputElement>("#file-input")!;
  const statusEl = container.querySelector<HTMLDivElement>("#import-status")!;

  browseBtn.addEventListener("click", () => fileInput.click());
  fileInput.addEventListener("change", () => {
    const file = fileInput.files?.[0];
    fileInput.value = "";
    if (file) void handleFile(file, worker);
  });

  dropzone.addEventListener("dragover", (e) => {
    e.preventDefault();
    dropzone.classList.add("drag-over");
  });
  dropzone.addEventListener("dragleave", () => dropzone.classList.remove("drag-over"));
  dropzone.addEventListener("drop", (e) => {
    e.preventDefault();
    dropzone.classList.remove("drag-over");
    const file = e.dataTransfer?.files?.[0];
    if (file) void handleFile(file, worker);
  });

  function render(): void {
    const state = store.get();
    const dataset = state.datasets.find((d) => d.id === state.activeDatasetId);
    if (!dataset) {
      statusEl.innerHTML = "";
      return;
    }
    const sizeMb = (dataset.sizeBytes / (1024 * 1024)).toFixed(1);
    if (dataset.status === "parsing") {
      statusEl.innerHTML = `<p class="status status-parsing"><strong>${escapeHtml(dataset.filename)}</strong> (${sizeMb} MB) — parsing… large files can take a moment.</p>`;
    } else if (dataset.status === "error") {
      statusEl.innerHTML = `<p class="status status-error"><strong>${escapeHtml(dataset.filename)}</strong> (${sizeMb} MB) — parse failed: ${escapeHtml(
        dataset.parseError?.message ?? "unknown error",
      )}</p>`;
    } else {
      statusEl.innerHTML = `<p class="status status-ok"><strong>${escapeHtml(dataset.filename)}</strong> (${sizeMb} MB) — parsed OK, ${dataset.report?.nRows.toLocaleString()} rows × ${dataset.report?.nColumns} columns.</p>`;
    }
  }

  store.subscribe(render);
  render();
}

async function handleFile(file: File, worker: DataWorkerClient): Promise<void> {
  const id = crypto.randomUUID();
  const dataset: Dataset = {
    id,
    filename: file.name,
    sizeBytes: file.size,
    status: "parsing",
    overrides: {},
    mapping: emptyMapping(),
    groupingColumns: [],
    validation: [],
    selectedGroupId: null,
    selectedRestId: null,
    regressionWindow: defaultRegressionWindow(),
    stageAConfig: defaultStageAConfig(),
    stageBConfig: defaultStageBConfig([]),
    stageAResult: null,
    stageAError: null,
    stageARunId: 0,
    stageBResult: null,
    optimalWindowConfig: defaultOptimalWindowConfig(),
    optimalWindowResult: null,
    optimalWindowError: null,
    qcConfig: defaultQcConfig(),
    manualExclusions: {},
    qcTableFilter: "all",
    additionalPlots: [],
    electrodeAreaCm2: 1.0,
    normalizeToArea: false,
    absoluteQ: false,
    absoluteDVDQ: true,
    fileSha256: null,
    parseCompletedAt: null,
    stageACompletedAt: null,
    stageBCompletedAt: null,
    optimalWindowCompletedAt: null,
    parseTimingMs: null,
    stageATimingMs: null,
    stageBTimingMs: null,
    optimalWindowTimingMs: null,
  };

  if (file.size > MAX_FILE_BYTES) {
    store.set((s) => ({
      ...s,
      datasets: [
        ...s.datasets,
        {
          ...dataset,
          status: "error",
          parseError: {
            message: `File is ${(file.size / (1024 * 1024)).toFixed(0)} MB, exceeding the ${MAX_FILE_BYTES / (1024 * 1024)} MB limit.`,
          },
        },
      ],
      activeDatasetId: id,
    }));
    return;
  }

  store.set((s) => ({ ...s, datasets: [...s.datasets, dataset], activeDatasetId: id }));

  const bytes = await file.arrayBuffer();
  const startedAt = performance.now();
  const format = file.name.toLowerCase().endsWith(".mf4") ? "mdf4" : "text";
  const outcome = await worker.parseFile(id, bytes, {}, format);

  if (!outcome.ok) {
    store.set((s) => ({
      ...s,
      datasets: s.datasets.map((d) => (d.id === id ? { ...d, status: "error", parseError: outcome.error } : d)),
    }));
    return;
  }

  const columnNames = outcome.columns.map((c) => c.name);
  const pendingRestoredConfig = store.get().pendingRestoredConfig;

  let mapping: Dataset["mapping"];
  let groupingColumns: string[];
  let restoredFields: Partial<Dataset> = {};
  let restoredAnalysisConfig: AnalysisConfig | null = null;

  if (pendingRestoredConfig) {
    // §12.5's session restore takes priority over the mapping-preset/auto-mapping
    // fallback chain -- narrowed scope: it just silently applies to the next
    // dropped file (matching the mapping-preset precedent's own behavior),
    // rather than pre-populating panels before any file exists.
    const restored = applyRestoredConfig(pendingRestoredConfig, columnNames);
    restoredFields = restored.fields;
    restoredAnalysisConfig = restored.analysisConfig;
    mapping = restored.fields.mapping ?? computeAutoMapping(columnNames);
    groupingColumns = restored.fields.groupingColumns ?? [];
  } else {
    const preset = loadMappingPreset(columnNames);
    mapping = preset?.mapping ?? computeAutoMapping(columnNames);
    // Grouping columns are deliberately *not* auto-restored from the silent
    // mapping preset (unlike the field mapping above): a stale grouping
    // selection from an earlier, unrelated file is much higher-risk than a
    // stale field mapping -- it fragments the whole dataset into many tiny
    // groups with no visible error, which is exactly what happened here
    // (a leftover "Qneg" grouping column, restored on every import with no
    // way to tell why the results looked wrong). Explicit §12.5 session
    // restore (the `pendingRestoredConfig` branch above) still restores it,
    // since that's a confirmed, user-initiated action, not a silent one.
    groupingColumns = [];
  }

  const parsedDataset: Dataset = {
    ...dataset,
    status: "parsed",
    report: outcome.report,
    columns: outcome.columns,
    mapping,
    groupingColumns,
    fileSha256: outcome.sha256,
    parseCompletedAt: Date.now(),
    parseTimingMs: performance.now() - startedAt,
    ...restoredFields,
  };
  const validation = await computeValidation(parsedDataset, worker);

  store.set((s) => ({
    ...s,
    datasets: s.datasets.map((d) => (d.id === id ? { ...parsedDataset, validation } : d)),
    ...(pendingRestoredConfig ? { pendingRestoredConfig: null } : {}),
    ...(restoredAnalysisConfig ? { analysisConfig: restoredAnalysisConfig } : {}),
  }));
}

function escapeHtml(s: string): string {
  const div = document.createElement("div");
  div.textContent = s;
  return div.innerHTML;
}
