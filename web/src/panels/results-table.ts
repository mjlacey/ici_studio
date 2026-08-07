// §11.6: the full analysis table (base Stage A columns + Stage B smoothing
// columns once available), virtualised the same way raw-data-inspector.ts
// is -- but synchronously, since the table (hundreds-low-thousands of
// rows) is already fully in memory, not paged from the worker. Sortable by
// clicking a header; charge/discharge and §9's QC filter narrow which rows
// show. Flagged/excluded rows are highlighted, a per-row toggle button
// cycles a manual override, and clicking a row (outside that button)
// selects it for Plot 2's diagnostic view -- §9's "click-through."

import { normalizeValue, normalizedLabel } from "../analysis/area-normalization";
import { applyAbsValue, absValueLabel } from "../analysis/absolute-value-display";
import { computeRowFlags, isRowExcluded, isRowFlagged, rowExclusionKey } from "../analysis/qc";
import { activeDataset, store, updateDataset } from "../state";
import type { AnalysisTable, Dataset, ManualExclusions, StageAResult, StageBResult } from "../types";

const ROW_HEIGHT = 24;
const SCROLL_BUFFER_ROWS = 15;

interface ColumnDef {
  key: string;
  label: string;
  get: (t: AnalysisTable, b: StageBResult | null) => (number | string | null)[];
}

const BASE_COLUMNS: ColumnDef[] = [
  { key: "groupId", label: "group", get: (t) => t.groupId },
  { key: "cycN", label: "cyc.n", get: (t) => t.cycN },
  { key: "state", label: "state", get: (t) => t.state },
  { key: "rest", label: "rest", get: (t) => t.rest },
  { key: "t", label: "t", get: (t) => t.t },
  { key: "stepT", label: "step.t", get: (t) => t.stepT },
  { key: "e", label: "E", get: (t) => t.e },
  { key: "i", label: "I", get: (t) => t.i },
  { key: "q", label: "Q", get: (t) => t.q },
  { key: "e0", label: "E0", get: (t) => t.e0 },
  { key: "e0Err", label: "E0_err", get: (t) => t.e0Err },
  { key: "s", label: "s", get: (t) => t.s },
  { key: "sErr", label: "s_err", get: (t) => t.sErr },
  { key: "i0", label: "I0", get: (t) => t.i0 },
  { key: "nPts", label: "n_pts", get: (t) => t.nPts },
  { key: "r2", label: "r2", get: (t) => t.r2 },
  { key: "adjR2", label: "adj_r2", get: (t) => t.adjR2 },
  { key: "rmse", label: "rmse", get: (t) => t.rmse },
  { key: "edgeMaeRatio", label: "edge_mae_ratio", get: (t) => t.edgeMaeRatio },
  { key: "edgeMaxZ", label: "edge_max_z", get: (t) => t.edgeMaxZ },
  { key: "r", label: "R", get: (t) => t.r },
  { key: "rErr", label: "R_err", get: (t) => t.rErr },
  { key: "k", label: "k", get: (t) => t.k },
  { key: "kErr", label: "k_err", get: (t) => t.kErr },
];

const SMOOTH_COLUMNS: ColumnDef[] = [
  { key: "e0Smooth", label: "E0_smooth", get: (_t, b) => b?.e0Smooth ?? [] },
  { key: "kSmooth", label: "k_smooth", get: (_t, b) => b?.kSmooth ?? [] },
  { key: "rSmooth", label: "R_smooth", get: (_t, b) => b?.rSmooth ?? [] },
  { key: "dVdQ", label: "dV/dQ", get: (_t, b) => b?.dVdQ ?? [] },
  { key: "dQdV", label: "dQ/dV", get: (_t, b) => b?.dQdV ?? [] },
];

export function mountResultsTable(container: HTMLElement): void {
  let sortKey = "rest";
  let sortDir: 1 | -1 = 1;
  let showCharge = true;
  let showDischarge = true;

  let lastResult: StageAResult | null = null;
  let lastStageB: StageBResult | null = null;
  let lastFilterKey = "";

  function render(): void {
    const dataset = activeDataset(store.get());
    const result = dataset?.stageAResult ?? null;
    if (!dataset || !result) {
      container.innerHTML = "";
      lastResult = null;
      return;
    }

    const stageB = dataset.stageBResult;
    const filterKey = `${showCharge}:${showDischarge}:${sortKey}:${sortDir}:${dataset.qcTableFilter}:${JSON.stringify(dataset.qcConfig)}:${JSON.stringify(dataset.manualExclusions)}:${dataset.normalizeToArea}:${dataset.electrodeAreaCm2}:${dataset.absoluteQ}:${dataset.absoluteDVDQ}`;
    if (result === lastResult && stageB === lastStageB && filterKey === lastFilterKey) return;
    lastResult = result;
    lastStageB = stageB;
    lastFilterKey = filterKey;

    renderTable(dataset, result.analysisTable, stageB);
  }

  function renderTable(dataset: Dataset, table: AnalysisTable, stageB: StageBResult | null): void {
    const columns = stageB ? [...BASE_COLUMNS, ...SMOOTH_COLUMNS] : BASE_COLUMNS;
    const n = table.rest.length;
    const qcConfig = dataset.qcConfig;
    const manualExclusions = dataset.manualExclusions;

    let indices = Array.from({ length: n }, (_, i) => i).filter((i) => (table.state[i] === "charge" ? showCharge : showDischarge));
    if (dataset.qcTableFilter === "flagged") {
      indices = indices.filter((i) => isRowFlagged(computeRowFlags(table, i, qcConfig)));
    } else if (dataset.qcTableFilter === "excluded") {
      indices = indices.filter((i) => isRowExcluded(table, i, qcConfig, manualExclusions));
    }

    const sortCol = columns.find((c) => c.key === sortKey) ?? columns[0];
    const sortValues = sortCol.get(table, stageB);
    indices.sort((a, b) => {
      const va = sortValues[a];
      const vb = sortValues[b];
      const na = va === null || va === undefined || (typeof va === "number" && !Number.isFinite(va));
      const nb = vb === null || vb === undefined || (typeof vb === "number" && !Number.isFinite(vb));
      if (na && nb) return 0;
      if (na) return 1; // NaN/null always sorts last, regardless of direction
      if (nb) return -1;
      if (typeof va === "string" || typeof vb === "string") return sortDir * String(va).localeCompare(String(vb));
      return sortDir * ((va as number) - (vb as number));
    });

    const totalRows = indices.length;

    container.innerHTML = `
      <section class="panel results-table-panel">
        <h2>Results table</h2>
        <div class="table-controls">
          <label><input type="checkbox" id="filter-charge" ${showCharge ? "checked" : ""} /> charge</label>
          <label><input type="checkbox" id="filter-discharge" ${showDischarge ? "checked" : ""} /> discharge</label>
          ${dataset.qcTableFilter !== "all" ? `<span class="hint">QC filter: ${dataset.qcTableFilter}</span>` : ""}
          <span class="hint">${totalRows.toLocaleString()} row(s)</span>
        </div>
        <div class="table-scroll" id="results-scroll">
          <table class="raw-table">
            <thead><tr id="results-thead-row"></tr></thead>
            <tbody id="results-tbody"></tbody>
          </table>
        </div>
      </section>
    `;

    const theadRow = container.querySelector<HTMLTableRowElement>("#results-thead-row")!;
    theadRow.innerHTML =
      `<th>QC</th>` +
      columns
        .map(
          (c) =>
            `<th data-sort-col="${c.key}" class="${c.key === sortKey ? `sorted-${sortDir === 1 ? "asc" : "desc"}` : ""}">${escapeHtml(absValueLabel(c.key, normalizedLabel(c.key, c.label, dataset), dataset))}</th>`,
        )
        .join("");

    for (const th of theadRow.querySelectorAll<HTMLTableCellElement>("[data-sort-col]")) {
      th.addEventListener("click", () => {
        const key = th.dataset.sortCol!;
        if (key === sortKey) sortDir = sortDir === 1 ? -1 : 1;
        else {
          sortKey = key;
          sortDir = 1;
        }
        lastFilterKey = "";
        render();
      });
    }

    container.querySelector<HTMLInputElement>("#filter-charge")!.addEventListener("change", (e) => {
      showCharge = (e.target as HTMLInputElement).checked;
      lastFilterKey = "";
      render();
    });
    container.querySelector<HTMLInputElement>("#filter-discharge")!.addEventListener("change", (e) => {
      showDischarge = (e.target as HTMLInputElement).checked;
      lastFilterKey = "";
      render();
    });

    const scrollEl = container.querySelector<HTMLDivElement>("#results-scroll")!;
    const tbody = container.querySelector<HTMLTableSectionElement>("#results-tbody")!;
    const nCols = columns.length + 1;
    // Sorting stays on the raw (un-normalized) values above -- multiplying/dividing
    // every value by the same positive constant preserves order, so no separate
    // normalized-sort variant is needed. Only the displayed values are normalized.
    const columnValues = columns.map((c) =>
      c.get(table, stageB).map((v) => (typeof v === "number" ? applyAbsValue(c.key, normalizeValue(c.key, v, dataset), dataset) : v)),
    );

    function loadVisible(): void {
      const firstVisible = Math.floor(scrollEl.scrollTop / ROW_HEIGHT);
      const visibleCount = Math.ceil(scrollEl.clientHeight / ROW_HEIGHT);
      const start = Math.max(0, firstVisible - SCROLL_BUFFER_ROWS);
      const end = Math.min(totalRows, firstVisible + visibleCount + SCROLL_BUFFER_ROWS);
      const topHeight = start * ROW_HEIGHT;
      const bottomHeight = Math.max(0, (totalRows - end) * ROW_HEIGHT);

      let rowsHtml = "";
      for (let r = start; r < end; r++) {
        const rowIdx = indices[r];
        const flags = computeRowFlags(table, rowIdx, qcConfig);
        const excluded = isRowExcluded(table, rowIdx, qcConfig, manualExclusions);
        const flagged = isRowFlagged(flags);
        const rowClass = excluded ? "qc-row-excluded" : flagged ? "qc-row-flagged" : "";
        const key = rowExclusionKey(table, rowIdx);
        const manual = manualExclusions[key];
        const toggleLabel = manual === "exclude" ? "⊘" : manual === "include" ? "✓" : "○";
        const toggleTitle = manual === "exclude" ? "Manually excluded -- click to include" : manual === "include" ? "Manually included -- click to clear" : "Not manually overridden -- click to exclude";
        rowsHtml += `<tr style="height:${ROW_HEIGHT}px" class="${rowClass}" data-group-id="${table.groupId[rowIdx]}" data-rest-id="${table.rest[rowIdx]}">
          <td><button type="button" class="btn-small qc-toggle" data-toggle-key="${escapeAttr(key)}" title="${escapeAttr(toggleTitle)}">${toggleLabel}</button></td>
          ${columnValues.map((vals) => `<td>${formatCell(vals[rowIdx])}</td>`).join("")}
        </tr>`;
      }
      tbody.innerHTML =
        `<tr style="height:${topHeight}px" aria-hidden="true"><td colspan="${nCols}" style="padding:0;border:0"></td></tr>` +
        rowsHtml +
        `<tr style="height:${bottomHeight}px" aria-hidden="true"><td colspan="${nCols}" style="padding:0;border:0"></td></tr>`;
    }

    tbody.addEventListener("click", (e) => {
      const target = e.target as HTMLElement;

      const toggleBtn = target.closest<HTMLElement>("[data-toggle-key]");
      if (toggleBtn) {
        const key = toggleBtn.dataset.toggleKey!;
        const current = dataset.manualExclusions[key];
        updateDataset(dataset.id, (d) => {
          const next: ManualExclusions = { ...d.manualExclusions };
          if (current === undefined) next[key] = "exclude";
          else if (current === "exclude") next[key] = "include";
          else delete next[key];
          return { ...d, manualExclusions: next };
        });
        return;
      }

      const row = target.closest<HTMLElement>("tr[data-group-id]");
      if (!row) return;
      const groupId = Number(row.dataset.groupId);
      const restId = Number(row.dataset.restId);
      updateDataset(dataset.id, (d) => ({ ...d, selectedGroupId: groupId, selectedRestId: restId }));
    });

    scrollEl.addEventListener("scroll", loadVisible);
    loadVisible();
  }

  store.subscribe(render);
  render();
}

function formatCell(v: number | string | null | undefined): string {
  if (v === null || v === undefined) return '<span class="cell-null">—</span>';
  if (typeof v === "number") {
    if (!Number.isFinite(v)) return '<span class="cell-null">—</span>';
    return Number.isInteger(v) ? String(v) : v.toPrecision(6);
  }
  return escapeHtml(v);
}

function escapeHtml(s: string): string {
  const div = document.createElement("div");
  div.textContent = s;
  return div.innerHTML;
}

function escapeAttr(s: string): string {
  return escapeHtml(s).replace(/"/g, "&quot;");
}
