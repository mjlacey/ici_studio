// §4.1/§4.2: the parse card. Reports the sniffer's structural decisions
// (encoding, delimiter, decimal separator, preamble lines, header
// found/synthesised, rows, columns) with a manual override for each,
// re-parsing on change. Reports structural facts only -- no preamble
// content is ever shown (§14 item 11).

import { activeDataset, store, updateDataset } from "../state";
import type { ParseOverrides } from "../types";
import { computeAutoMapping, loadMappingPreset } from "./auto-mapping";
import { computeValidation } from "./validation";
import type { DataWorkerClient } from "../worker/client";

const ENCODING_OPTIONS = ["Utf8", "Cp1252"] as const;
const DELIMITER_OPTIONS = ["Tab", "Comma", "Semicolon", "Pipe"] as const;
const DECIMAL_OPTIONS = ["Dot", "Comma"] as const;

export function mountParseCard(container: HTMLElement, worker: DataWorkerClient): void {
  function render(): void {
    const dataset = activeDataset(store.get());
    if (!dataset || (dataset.status !== "parsed" && dataset.status !== "error")) {
      container.innerHTML = "";
      return;
    }
    // §4.1: the sniffer overrides (encoding/delimiter/decimal separator/skip
    // lines/header) are all text-file concepts -- meaningless for a binary
    // MDF4 import, which has nothing to reparse with. Filename-based rather
    // than `report.source`-based so the check still works in the error
    // state, where there's no report yet.
    const isMdf4 = dataset.filename.toLowerCase().endsWith(".mf4");

    if (dataset.status === "error") {
      container.innerHTML = `
        <section class="panel">
          <h2>Parse card</h2>
          <p class="status-error">${escapeHtml(dataset.parseError?.message ?? "Parse failed.")}</p>
          ${
            dataset.parseError?.delimitersTried
              ? `<p class="hint">Delimiters tried: ${dataset.parseError.delimitersTried.join(", ")}</p>`
              : ""
          }
          ${isMdf4 ? "" : renderOverrides(dataset.overrides)}
        </section>`;
      if (!isMdf4) wireOverrideInputs(container, dataset.id, worker);
      return;
    }

    const report = dataset.report!;
    container.innerHTML = `
      <section class="panel">
        <h2>Parse card</h2>
        <table class="kv-table">
          ${isMdf4 ? "" : `<tr><th>Encoding</th><td>${report.encoding}</td></tr>`}
          ${isMdf4 ? "" : `<tr><th>Delimiter</th><td>${report.delimiter}</td></tr>`}
          ${isMdf4 ? "" : `<tr><th>Decimal separator</th><td>${report.decimalSeparator}</td></tr>`}
          ${isMdf4 ? "" : `<tr><th>Preamble lines skipped</th><td>${report.preambleLinesSkipped}</td></tr>`}
          ${isMdf4 ? "" : `<tr><th>Header</th><td>${report.headerSynthesized ? "synthesised (col1…colN)" : "found"}</td></tr>`}
          <tr><th>Rows × columns</th><td>${report.nRows.toLocaleString()} × ${report.nColumns}</td></tr>
          ${report.trailingColumnDropped ? `<tr><th>Trailing column</th><td>dropped (trailing delimiter)</td></tr>` : ""}
          ${report.raggedRowsDropped > 0 ? `<tr><th>Ragged rows dropped</th><td>${report.raggedRowsDropped} (first: ${report.raggedRowLineNumbers.join(", ")})</td></tr>` : ""}
        </table>
        ${
          report.warnings.length
            ? `<ul class="warnings">${report.warnings.map((w) => `<li>${escapeHtml(w)}</li>`).join("")}</ul>`
            : ""
        }
        ${isMdf4 ? "" : renderOverrides(dataset.overrides)}
      </section>
    `;
    if (!isMdf4) wireOverrideInputs(container, dataset.id, worker);
  }

  store.subscribe(render);
  render();
}

function renderOverrides(overrides: ParseOverrides): string {
  return `
    <details class="overrides">
      <summary>Manual override</summary>
      <div class="override-grid">
        <label>Encoding
          <select data-override="encoding">
            <option value="">auto</option>
            ${ENCODING_OPTIONS.map((o) => `<option value="${o}" ${overrides.encoding === o ? "selected" : ""}>${o}</option>`).join("")}
          </select>
        </label>
        <label>Delimiter
          <select data-override="delimiter">
            <option value="">auto</option>
            ${DELIMITER_OPTIONS.map((o) => `<option value="${o}" ${overrides.delimiter === o ? "selected" : ""}>${o}</option>`).join("")}
          </select>
        </label>
        <label>Decimal separator
          <select data-override="decimalSeparator">
            <option value="">auto</option>
            ${DECIMAL_OPTIONS.map((o) => `<option value="${o}" ${overrides.decimalSeparator === o ? "selected" : ""}>${o}</option>`).join("")}
          </select>
        </label>
        <label>Skip N lines
          <input type="number" min="0" data-override="skipLines" value="${overrides.skipLines ?? ""}" placeholder="auto" />
        </label>
        <label>First row is a header
          <select data-override="headerPresent">
            <option value="">auto</option>
            <option value="true" ${overrides.headerPresent === true ? "selected" : ""}>yes</option>
            <option value="false" ${overrides.headerPresent === false ? "selected" : ""}>no</option>
          </select>
        </label>
      </div>
    </details>
  `;
}

function wireOverrideInputs(container: HTMLElement, datasetId: string, worker: DataWorkerClient): void {
  const inputs = container.querySelectorAll<HTMLSelectElement | HTMLInputElement>("[data-override]");
  for (const input of inputs) {
    input.addEventListener("change", () => void handleOverrideChange(container, datasetId, worker));
  }
}

async function handleOverrideChange(container: HTMLElement, datasetId: string, worker: DataWorkerClient): Promise<void> {
  const overrides: ParseOverrides = {};
  const encoding = container.querySelector<HTMLSelectElement>('[data-override="encoding"]')?.value;
  const delimiter = container.querySelector<HTMLSelectElement>('[data-override="delimiter"]')?.value;
  const decimalSeparator = container.querySelector<HTMLSelectElement>('[data-override="decimalSeparator"]')?.value;
  const skipLines = container.querySelector<HTMLInputElement>('[data-override="skipLines"]')?.value;
  const headerPresent = container.querySelector<HTMLSelectElement>('[data-override="headerPresent"]')?.value;

  if (encoding) overrides.encoding = encoding as ParseOverrides["encoding"];
  if (delimiter) overrides.delimiter = delimiter as ParseOverrides["delimiter"];
  if (decimalSeparator) overrides.decimalSeparator = decimalSeparator as ParseOverrides["decimalSeparator"];
  if (skipLines) overrides.skipLines = Number(skipLines);
  if (headerPresent) overrides.headerPresent = headerPresent === "true";

  updateDataset(datasetId, (d) => ({ ...d, status: "parsing", overrides }));
  const outcome = await worker.reparse(datasetId, overrides);

  if (!outcome.ok) {
    updateDataset(datasetId, (d) => ({ ...d, status: "error", parseError: outcome.error, overrides }));
    return;
  }

  const preset = loadMappingPreset(outcome.columns.map((c) => c.name));
  const mapping = preset?.mapping ?? computeAutoMapping(outcome.columns.map((c) => c.name));
  const groupingColumns = preset?.groupingColumns ?? [];

  updateDataset(datasetId, (d) => ({
    ...d,
    status: "parsed",
    report: outcome.report,
    columns: outcome.columns,
    mapping,
    groupingColumns,
    overrides,
  }));
  const refreshed = activeDataset(store.get());
  if (refreshed) {
    const validation = await computeValidation(refreshed, worker);
    updateDataset(datasetId, (d) => ({ ...d, validation }));
  }
}

function escapeHtml(s: string): string {
  const div = document.createElement("div");
  div.textContent = s;
  return div.innerHTML;
}
