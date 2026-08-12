// §12.1-12.3: TSV analysed-data export, config JSON export + "Load
// config", and the run-report JSON export. Grouped in one left-column
// panel since none of these are per-plot (unlike §12.4's PNG buttons,
// which live in each plot panel's own corner).

import { applyConfigImport, buildConfigExport } from "../export/config";
import { downloadJson, downloadText } from "../export/download";
import { buildPocvTsv, pocvGroupOptions } from "../export/pocv";
import { buildRunReport } from "../export/run-report";
import { buildAnalysisTsv, defaultTsvOptions, type TsvOptions } from "../export/tsv";
import { activeDataset, store, updateDataset } from "../state";
import type { Dataset } from "../types";
import type { DataWorkerClient } from "../worker/client";

let tsvOptions = defaultTsvOptions();
let pocvGroupKey: string | null = null;

export function mountExportPanel(container: HTMLElement, worker: DataWorkerClient): void {
  let statusMessage = "";

  function render(): void {
    const dataset = activeDataset(store.get());
    if (!dataset) {
      container.innerHTML = "";
      return;
    }
    renderPanel(container, dataset, worker, statusMessage, (msg) => {
      statusMessage = msg;
      render();
    });
  }

  store.subscribe(render);
  render();
}

function renderPanel(container: HTMLElement, dataset: Dataset, worker: DataWorkerClient, statusMessage: string, setStatus: (msg: string) => void): void {
  const hasResult = !!dataset.stageAResult;
  const pocvOptions = pocvGroupOptions(dataset);
  if (!pocvGroupKey || !pocvOptions.some((o) => o.key === pocvGroupKey)) {
    pocvGroupKey = pocvOptions[0]?.key ?? null;
  }

  container.innerHTML = `
    <section class="panel">
      <h2>Export</h2>
      <h3>Analysed data (TSV)</h3>
      <table class="mapping-table">
        <tbody>
          <tr>
            <th>Precision</th>
            <td>
              <select data-tsv="precision">
                <option value="full" ${tsvOptions.precision === "full" ? "selected" : ""}>Full (round-trip)</option>
                <option value="6sig" ${tsvOptions.precision === "6sig" ? "selected" : ""}>6 significant figures</option>
              </select>
            </td>
          </tr>
          <tr>
            <th>Missing values</th>
            <td>
              <select data-tsv="missing">
                <option value="empty" ${tsvOptions.missing === "empty" ? "selected" : ""}>Empty</option>
                <option value="NA" ${tsvOptions.missing === "NA" ? "selected" : ""}>NA</option>
                <option value="NaN" ${tsvOptions.missing === "NaN" ? "selected" : ""}>NaN</option>
              </select>
            </td>
          </tr>
          <tr>
            <th>Unit header comment</th>
            <td><input type="checkbox" data-tsv="includeUnitComment" ${tsvOptions.includeUnitComment ? "checked" : ""} /></td>
          </tr>
        </tbody>
      </table>
      <button type="button" class="btn" id="download-tsv" ${hasResult ? "" : "disabled"}>Download TSV</button>
      <button type="button" class="btn-small" id="download-regression-tsv" ${hasResult ? "" : "disabled"}>Per-rest regression table only</button>
      ${!hasResult ? `<p class="hint">Run Stage A first.</p>` : ""}

      <h3>Configuration</h3>
      <button type="button" class="btn" id="download-config">Download config (JSON)</button>
      <label class="btn-small load-config-label">Load config… <input type="file" id="load-config-input" accept=".json" hidden /></label>

      <h3>Run report</h3>
      <button type="button" class="btn" id="download-run-report" ${hasResult ? "" : "disabled"}>Download run report (JSON)</button>
      ${!hasResult ? `<p class="hint">Run Stage A first.</p>` : ""}

      <details class="overrides">
        <summary>Advanced</summary>
        <h3>Export charge/discharge pOCV</h3>
        <table class="mapping-table">
          <tbody>
            <tr>
              <th>Half-cycle</th>
              <td>
                <select data-pocv-group ${pocvOptions.length ? "" : "disabled"}>
                  ${pocvOptions
                    .map((o) => `<option value="${escapeHtml(o.key)}" ${o.key === pocvGroupKey ? "selected" : ""}>${escapeHtml(o.label)} (${o.n} pts)</option>`)
                    .join("")}
                </select>
              </td>
            </tr>
          </tbody>
        </table>
        <button type="button" class="btn-small" id="download-pocv" ${pocvOptions.length ? "" : "disabled"}>Export charge/discharge pOCV</button>
        ${
          !dataset.stageBResult
            ? `<p class="hint">Run Stage B ("Smooth &amp; derive") first.</p>`
            : pocvOptions.length === 0
              ? `<p class="hint">No smoothed points available to export.</p>`
              : `<p class="hint">Two columns only — Q and the smoothed E0 curve, renamed "E" — for the selected half-cycle only, for direct import into external pOCV tooling.</p>`
        }
      </details>

      ${statusMessage ? `<p class="info-note">${escapeHtml(statusMessage)}</p>` : ""}
    </section>
  `;

  container.querySelector<HTMLSelectElement>('[data-tsv="precision"]')!.addEventListener("change", (e) => {
    tsvOptions = { ...tsvOptions, precision: (e.target as HTMLSelectElement).value as TsvOptions["precision"] };
  });
  container.querySelector<HTMLSelectElement>('[data-tsv="missing"]')!.addEventListener("change", (e) => {
    tsvOptions = { ...tsvOptions, missing: (e.target as HTMLSelectElement).value as TsvOptions["missing"] };
  });
  container.querySelector<HTMLInputElement>('[data-tsv="includeUnitComment"]')!.addEventListener("change", (e) => {
    tsvOptions = { ...tsvOptions, includeUnitComment: (e.target as HTMLInputElement).checked };
  });

  container.querySelector<HTMLButtonElement>("#download-tsv")?.addEventListener("click", () => {
    void downloadTsv(dataset, worker, { ...tsvOptions, includeSmoothed: true }, `${dataset.filename}_analysed.txt`);
  });
  container.querySelector<HTMLButtonElement>("#download-regression-tsv")?.addEventListener("click", () => {
    void downloadTsv(dataset, worker, { ...tsvOptions, includeSmoothed: false }, `${dataset.filename}_regression.txt`);
  });

  container.querySelector<HTMLButtonElement>("#download-config")!.addEventListener("click", () => {
    const config = buildConfigExport(dataset, store.get().analysisConfig);
    downloadJson(config, `${dataset.filename}_config.json`);
  });

  container.querySelector<HTMLInputElement>("#load-config-input")!.addEventListener("change", (e) => {
    const file = (e.target as HTMLInputElement).files?.[0];
    (e.target as HTMLInputElement).value = "";
    if (file) void loadConfig(file, dataset, setStatus);
  });

  container.querySelector<HTMLButtonElement>("#download-run-report")?.addEventListener("click", () => {
    const report = buildRunReport(dataset, store.get().analysisConfig);
    downloadJson(report, `${dataset.filename}_run-report.json`);
  });

  container.querySelector<HTMLSelectElement>("[data-pocv-group]")?.addEventListener("change", (e) => {
    pocvGroupKey = (e.target as HTMLSelectElement).value;
  });
  container.querySelector<HTMLButtonElement>("#download-pocv")?.addEventListener("click", () => {
    if (!pocvGroupKey) return;
    const tsv = buildPocvTsv(dataset, pocvGroupKey);
    const label = pocvOptions.find((o) => o.key === pocvGroupKey)?.label ?? "export";
    downloadText(tsv, `${dataset.filename}_pOCV_${sanitizeFilenamePart(label)}.txt`, "text/tab-separated-values;charset=utf-8");
  });
}

function sanitizeFilenamePart(s: string): string {
  return s.replace(/[^a-z0-9]+/gi, "_").replace(/^_+|_+$/g, "");
}

async function downloadTsv(dataset: Dataset, worker: DataWorkerClient, options: TsvOptions, filename: string): Promise<void> {
  const groupKeyColumns = await worker.getGroupKeyColumns(dataset.id);
  const tsv = buildAnalysisTsv(dataset, groupKeyColumns, options);
  downloadText(tsv, filename, "text/tab-separated-values;charset=utf-8");
}

async function loadConfig(file: File, dataset: Dataset, setStatus: (msg: string) => void): Promise<void> {
  try {
    const text = await file.text();
    const config = JSON.parse(text);
    const availableColumns = (dataset.columns ?? []).map((c) => c.name);
    const result = applyConfigImport(config, availableColumns);

    updateDataset(dataset.id, (d) => ({ ...d, ...result.fields }));
    if (result.analysisConfig) {
      store.set((s) => ({ ...s, analysisConfig: result.analysisConfig! }));
    }

    const parts: string[] = [];
    if (result.applied.length) parts.push(`Applied: ${result.applied.join(", ")}.`);
    if (result.skipped.length) parts.push(`Skipped: ${result.skipped.join("; ")}.`);
    setStatus(parts.join(" ") || "Nothing to apply.");
  } catch (err) {
    setStatus(`Could not load config: ${err instanceof Error ? err.message : String(err)}`);
  }
}

function escapeHtml(s: string): string {
  const div = document.createElement("div");
  div.textContent = s;
  return div.innerHTML;
}
