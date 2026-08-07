// §5: column mapping and units panel, plus §6's Q-anchoring control (shown
// "in the import section under the mapping table") and §5.3 validation
// display.

import { activeDataset, store, updateDataset } from "../state";
import { REQUIRED_FIELDS, type Dataset, type RequiredField, type ValidationIssue } from "../types";
import { isHalfCycleColumn, saveMappingPreset } from "./auto-mapping";
import { computeValidation } from "./validation";
import type { DataWorkerClient } from "../worker/client";

const FIELD_LABELS: Record<RequiredField, string> = {
  time: "Time (t)",
  cycle: "Cycle (cyc.n)",
  current: "Current (I)",
  voltage: "Voltage (E)",
  charge: "Charge (Q)",
};

const UNIT_OPTIONS: Record<RequiredField, string[]> = {
  time: ["s", "min", "h"],
  cycle: [],
  current: ["A", "mA", "uA"],
  voltage: ["V", "mV"],
  charge: ["Ah", "mAh", "uAh", "C"],
};

const UNIT_LABELS: Record<string, string> = { uA: "µA", uAh: "µAh" };

export function mountColumnMappingPanel(container: HTMLElement, worker: DataWorkerClient): void {
  function render(): void {
    const dataset = activeDataset(store.get());
    if (!dataset || dataset.status !== "parsed" || !dataset.columns) {
      container.innerHTML = "";
      return;
    }

    const mappedNames = new Set(REQUIRED_FIELDS.map((f) => dataset.mapping[f].column).filter((c): c is string => !!c));
    const groupableColumns = dataset.columns.filter((c) => !mappedNames.has(c.name));
    const qAnchor = store.get().analysisConfig.qAnchoring;

    container.innerHTML = `
      <section class="panel">
        <h2>Column mapping</h2>
        <table class="mapping-table">
          <thead><tr><th>Field</th><th>Data column</th><th>Units</th></tr></thead>
          <tbody>
            ${REQUIRED_FIELDS.map((field) => renderFieldRow(field, dataset)).join("")}
          </tbody>
        </table>

        <h3>Optional fields</h3>
        <table class="mapping-table">
          <tbody>
            <tr>
              <th>Grouping columns</th>
              <td colspan="2">
                <div class="grouping-columns-list">
                  ${groupableColumns
                    .map(
                      (c) =>
                        `<label class="grouping-column-option"><input type="checkbox" data-grouping-option value="${escapeAttr(c.name)}" ${dataset.groupingColumns.includes(c.name) ? "checked" : ""} /> ${escapeHtml(c.name)}</label>`,
                    )
                    .join("")}
                </div>
                ${groupableColumns.length === 0 ? '<p class="hint">No unmapped columns available.</p>' : ""}
              </td>
            </tr>
          </tbody>
        </table>

        <h3>Q normalisation</h3>
        <table class="mapping-table">
          <tbody>
            <tr>
              <th>Charge: Q = 0 at</th>
              <td colspan="2">
                <select data-qanchor="charge">
                  <option value="start" ${qAnchor.charge === "start" ? "selected" : ""}>start of half-cycle</option>
                  <option value="end" ${qAnchor.charge === "end" ? "selected" : ""}>end of half-cycle</option>
                </select>
              </td>
            </tr>
            <tr>
              <th>Discharge: Q = 0 at</th>
              <td colspan="2">
                <select data-qanchor="discharge">
                  <option value="start" ${qAnchor.discharge === "start" ? "selected" : ""}>start of half-cycle</option>
                  <option value="end" ${qAnchor.discharge === "end" ? "selected" : ""}>end of half-cycle</option>
                </select>
              </td>
            </tr>
          </tbody>
        </table>
        <p class="echo">${describeAnchoring(qAnchor)}</p>

        <div id="prenormalized-note"></div>
        ${renderValidation(dataset.validation)}
      </section>
    `;

    wireInputs(container, dataset, worker);
    void refreshPrenormalizedNote(container, dataset, worker);
  }

  store.subscribe(render);
  render();
}

function renderFieldRow(field: RequiredField, dataset: Dataset): string {
  const mapping = dataset.mapping[field];
  const columns = dataset.columns ?? [];
  const units = UNIT_OPTIONS[field];

  const options = columns
    .map((c) => {
      const label = isHalfCycleColumn(c.name) ? `${c.name} (half cycle)` : c.name;
      return `<option value="${escapeAttr(c.name)}" ${mapping.column === c.name ? "selected" : ""}>${escapeHtml(label)}</option>`;
    })
    .join("");

  return `
    <tr data-field-row="${field}">
      <th>
        ${FIELD_LABELS[field]}
        <span class="auto-badge" ${mapping.autoDetected ? "" : 'style="display:none"'}>auto-detected — verify</span>
      </th>
      <td>
        <select data-map="${field}">
          <option value="">(none)</option>
          ${options}
        </select>
      </td>
      <td>
        ${
          units.length
            ? `<select data-unit="${field}">${units.map((u) => `<option value="${u}" ${mapping.unit === u ? "selected" : ""}>${UNIT_LABELS[u] ?? u}</option>`).join("")}</select>`
            : `<span class="hint">(none)</span>`
        }
      </td>
    </tr>
  `;
}

function describeAnchoring(qAnchor: { charge: "start" | "end"; discharge: "start" | "end" }): string {
  const chargeLimit = qAnchor.charge === "start" ? "the discharge limit" : "the charge limit";
  const dischargeLimit = qAnchor.discharge === "start" ? "the charge limit" : "the discharge limit";
  return `Charge: Q = 0 at ${chargeLimit}. Discharge: Q = 0 at ${dischargeLimit}.`;
}

function renderValidation(issues: ValidationIssue[]): string {
  if (issues.length === 0) return "";
  const items = issues
    .map((issue) => {
      const actionBtn = issue.action
        ? `<button type="button" class="btn-small" data-action="${issue.action}" data-column="${escapeAttr(issue.column ?? "")}">${actionLabel(issue.action)}</button>`
        : "";
      return `<li class="issue issue-${issue.severity}">${escapeHtml(issue.message)} ${actionBtn}</li>`;
    })
    .join("");
  return `<ul class="issues">${items}</ul>`;
}

function actionLabel(action: NonNullable<ValidationIssue["action"]>): string {
  switch (action) {
    case "dropRows":
      return "Drop these rows";
    case "sortByTime":
      return "Sort by time";
    case "dropDecreasingRows":
      return "Drop the decreasing rows";
  }
}

function wireInputs(container: HTMLElement, dataset: Dataset, worker: DataWorkerClient): void {
  for (const field of REQUIRED_FIELDS) {
    container.querySelector<HTMLSelectElement>(`[data-map="${field}"]`)?.addEventListener("change", (e) => {
      const column = (e.target as HTMLSelectElement).value || null;
      updateDataset(dataset.id, (d) => ({
        ...d,
        mapping: { ...d.mapping, [field]: { ...d.mapping[field], column, autoDetected: false } },
      }));
      afterMappingChange(dataset.id, worker);
    });
    container.querySelector<HTMLSelectElement>(`[data-unit="${field}"]`)?.addEventListener("change", (e) => {
      const unit = (e.target as HTMLSelectElement).value;
      updateDataset(dataset.id, (d) => ({
        ...d,
        mapping: { ...d.mapping, [field]: { ...d.mapping[field], unit, autoDetected: false } },
      }));
      afterMappingChange(dataset.id, worker);
    });
  }

  // Checkboxes, not a native `<select multiple>`: a plain click on a
  // multi-select option *replaces* the whole selection with just that
  // option (deselecting everything else) -- to toggle a single option off
  // without touching the rest, the browser requires a ctrl/cmd+click, which
  // isn't discoverable and reads as "I can only select, never deselect."
  // Checkboxes have no such ambiguity.
  for (const checkbox of container.querySelectorAll<HTMLInputElement>("[data-grouping-option]")) {
    checkbox.addEventListener("change", () => {
      const current = new Set(dataset.groupingColumns);
      if (checkbox.checked) current.add(checkbox.value);
      else current.delete(checkbox.value);
      updateDataset(dataset.id, (d) => ({ ...d, groupingColumns: [...current] }));
      afterMappingChange(dataset.id, worker);
    });
  }

  for (const state of ["charge", "discharge"] as const) {
    container.querySelector<HTMLSelectElement>(`[data-qanchor="${state}"]`)?.addEventListener("change", (e) => {
      const value = (e.target as HTMLSelectElement).value as "start" | "end";
      store.set((s) => ({
        ...s,
        analysisConfig: { ...s.analysisConfig, qAnchoring: { ...s.analysisConfig.qAnchoring, [state]: value } },
      }));
    });
  }

  for (const btn of container.querySelectorAll<HTMLButtonElement>("[data-action]")) {
    btn.addEventListener("click", () => void handleValidationAction(dataset.id, btn.dataset.action!, btn.dataset.column!, worker));
  }
}

function afterMappingChange(datasetId: string, worker: DataWorkerClient): void {
  const dataset = activeDataset(store.get());
  if (!dataset || !dataset.columns) return;
  saveMappingPreset(dataset.columns.map((c) => c.name), { mapping: dataset.mapping, groupingColumns: dataset.groupingColumns });
  void computeValidation(dataset, worker).then((validation) => updateDataset(datasetId, (d) => ({ ...d, validation })));
}

async function refreshPrenormalizedNote(container: HTMLElement, dataset: Dataset, worker: DataWorkerClient): Promise<void> {
  const noteEl = container.querySelector<HTMLDivElement>("#prenormalized-note");
  if (!noteEl) return;
  const { charge, cycle, current } = dataset.mapping;
  if (!charge.column || !cycle.column || !current.column) {
    noteEl.innerHTML = "";
    return;
  }
  const looksPrenormalized = await worker.detectPrenormalizedCharge(dataset.id, charge.column, cycle.column, current.column, 0);
  noteEl.innerHTML = looksPrenormalized
    ? `<p class="info-note">'${escapeHtml(charge.column)}' already looks reset per half-cycle — Q anchoring will be a no-op.</p>`
    : "";
}

async function handleValidationAction(datasetId: string, action: string, column: string, worker: DataWorkerClient): Promise<void> {
  let outcome;
  if (action === "dropRows") {
    outcome = await worker.dropNonFiniteRows(datasetId, column);
  } else if (action === "sortByTime") {
    outcome = await worker.sortByColumn(datasetId, column);
  } else if (action === "dropDecreasingRows") {
    outcome = await worker.dropDecreasingRows(datasetId, column);
  } else {
    return;
  }
  if (!outcome.ok) return;
  updateDataset(datasetId, (d) => ({ ...d, report: outcome.report, columns: outcome.columns }));
  const dataset = activeDataset(store.get());
  if (!dataset) return;
  const validation = await computeValidation(dataset, worker);
  updateDataset(datasetId, (d) => ({ ...d, validation }));
}

function escapeHtml(s: string): string {
  const div = document.createElement("div");
  div.textContent = s;
  return div.innerHTML;
}

function escapeAttr(s: string): string {
  return escapeHtml(s).replace(/"/g, "&quot;");
}
