// §5.2: raw data inspector. Virtual-scrolled table paged from the worker
// (never the full dataset), jump-to-row, per-column summary stats. Mapped
// columns are highlighted and labelled with the internal unit they'll
// convert to (actual conversion happens at Stage A, milestone 8 -- this is
// a label only, the displayed values are the raw parsed ones).
//
// "Show rows around rest #N" (§5.2) is deferred: there's no rest data until
// Stage A exists (milestone 8).

import { activeDataset, store } from "../state";
import type { ColumnStats, Dataset, RequiredField } from "../types";
import { REQUIRED_FIELDS } from "../types";
import type { DataWorkerClient } from "../worker/client";

const ROW_HEIGHT = 24;
const SCROLL_BUFFER_ROWS = 15;

const INTERNAL_UNIT: Record<RequiredField, string> = {
  time: "s",
  cycle: "",
  current: "A",
  voltage: "V",
  charge: "Ah",
};

export function mountRawDataInspector(container: HTMLElement, worker: DataWorkerClient): void {
  let lastDatasetId: string | null = null;
  let lastColumnsKey = "";
  let fetchToken = 0;

  function render(): void {
    const dataset = activeDataset(store.get());
    if (!dataset || dataset.status !== "parsed" || !dataset.columns || !dataset.report) {
      container.innerHTML = "";
      lastDatasetId = null;
      return;
    }

    const columnsKey = dataset.columns.map((c) => c.name).join("|");
    if (dataset.id === lastDatasetId && columnsKey === lastColumnsKey) {
      // Same shape already mounted (e.g. only mapping/validation changed) --
      // just refresh the header highlighting, not the whole scroll state.
      updateHeaderHighlighting(container, dataset);
      return;
    }
    lastDatasetId = dataset.id;
    lastColumnsKey = columnsKey;

    const mappedByColumn = mappedFieldsByColumn(dataset);

    container.innerHTML = `
      <section class="panel raw-inspector">
        <h2>Raw data</h2>
        <div class="inspector-controls">
          <label>Jump to row <input type="number" min="0" max="${dataset.report.nRows - 1}" id="jump-input" /></label>
          <button type="button" class="btn-small" id="jump-btn">Go</button>
          <span class="hint">${dataset.report.nRows.toLocaleString()} rows</span>
        </div>
        <div class="table-scroll" id="table-scroll">
          <table class="raw-table">
            <thead>
              <tr id="raw-thead-row"></tr>
            </thead>
            <tbody id="raw-tbody"></tbody>
          </table>
        </div>
        <details class="column-stats">
          <summary>Per-column summary stats</summary>
          <div id="column-stats-body">Loading…</div>
        </details>
      </section>
    `;

    const theadRow = container.querySelector<HTMLTableRowElement>("#raw-thead-row")!;
    theadRow.innerHTML = dataset.columns
      .map((c) => {
        const field = mappedByColumn.get(c.name);
        const label = field ? `${escapeHtml(c.name)} <span class="unit-badge">→ ${INTERNAL_UNIT[field] || "—"}</span>` : escapeHtml(c.name);
        return `<th class="${field ? "mapped-col" : ""}" data-col="${escapeAttr(c.name)}">${label}</th>`;
      })
      .join("");

    const scrollEl = container.querySelector<HTMLDivElement>("#table-scroll")!;
    const tbody = container.querySelector<HTMLTableSectionElement>("#raw-tbody")!;
    const datasetId = dataset.id;
    const totalRows = dataset.report.nRows;
    const nCols = dataset.columns.length;

    let fetching = false;
    let refetchPending = false;

    async function loadVisible(): Promise<void> {
      if (fetching) {
        refetchPending = true;
        return;
      }
      fetching = true;
      const myToken = ++fetchToken;

      const firstVisible = Math.floor(scrollEl.scrollTop / ROW_HEIGHT);
      const visibleCount = Math.ceil(scrollEl.clientHeight / ROW_HEIGHT);
      const start = Math.max(0, firstVisible - SCROLL_BUFFER_ROWS);
      const end = Math.min(totalRows, firstVisible + visibleCount + SCROLL_BUFFER_ROWS);
      const limit = Math.max(0, end - start);

      const page = limit > 0 ? await worker.getPage(datasetId, start, limit) : { rows: [] as (number | string | null)[][] };
      fetching = false;
      if (myToken !== fetchToken) return; // a newer request superseded this one

      const topHeight = start * ROW_HEIGHT;
      const bottomHeight = Math.max(0, (totalRows - end) * ROW_HEIGHT);
      const rowsHtml = page.rows
        .map((row) => `<tr style="height:${ROW_HEIGHT}px">${row.map((cell) => `<td>${formatCell(cell)}</td>`).join("")}</tr>`)
        .join("");
      tbody.innerHTML =
        `<tr style="height:${topHeight}px" aria-hidden="true"><td colspan="${nCols}" style="padding:0;border:0"></td></tr>` +
        rowsHtml +
        `<tr style="height:${bottomHeight}px" aria-hidden="true"><td colspan="${nCols}" style="padding:0;border:0"></td></tr>`;

      if (refetchPending) {
        refetchPending = false;
        void loadVisible();
      }
    }

    scrollEl.addEventListener("scroll", () => void loadVisible());
    void loadVisible();

    const jumpInput = container.querySelector<HTMLInputElement>("#jump-input")!;
    const jumpBtn = container.querySelector<HTMLButtonElement>("#jump-btn")!;
    const jump = () => {
      const row = Math.max(0, Math.min(totalRows - 1, Number(jumpInput.value) || 0));
      scrollEl.scrollTop = row * ROW_HEIGHT;
    };
    jumpBtn.addEventListener("click", jump);
    jumpInput.addEventListener("keydown", (e) => {
      if (e.key === "Enter") jump();
    });

    void loadColumnStats(container, dataset, worker);
  }

  store.subscribe(render);
  render();
}

function mappedFieldsByColumn(dataset: Dataset): Map<string, RequiredField> {
  const map = new Map<string, RequiredField>();
  for (const field of REQUIRED_FIELDS) {
    const col = dataset.mapping[field].column;
    if (col) map.set(col, field);
  }
  return map;
}

function updateHeaderHighlighting(container: HTMLElement, dataset: Dataset): void {
  const mappedByColumn = mappedFieldsByColumn(dataset);
  for (const th of container.querySelectorAll<HTMLTableCellElement>("th[data-col]")) {
    const field = mappedByColumn.get(th.dataset.col ?? "");
    th.classList.toggle("mapped-col", !!field);
  }
}

async function loadColumnStats(container: HTMLElement, dataset: Dataset, worker: DataWorkerClient): Promise<void> {
  const body = container.querySelector<HTMLDivElement>("#column-stats-body");
  if (!body || !dataset.columns) return;

  const results = await Promise.all(dataset.columns.map((c) => worker.getColumnStats(dataset.id, c.name)));

  const rows = dataset.columns
    .map((c, i) => {
      const s: ColumnStats | null = results[i];
      if (!s) return "";
      const cells = s.isNumeric
        ? `<td>${s.nFinite.toLocaleString()}</td><td>${formatNum(s.min)}</td><td>${formatNum(s.median)}</td><td>${formatNum(s.max)}</td>`
        : `<td>${s.nFinite.toLocaleString()}</td><td colspan="3">${s.distinctCount ?? "?"} distinct value(s)</td>`;
      return `<tr><th>${escapeHtml(c.name)}</th>${cells}</tr>`;
    })
    .join("");

  body.innerHTML = `
    <table class="kv-table">
      <thead><tr><th>Column</th><th>n finite</th><th>min</th><th>median</th><th>max</th></tr></thead>
      <tbody>${rows}</tbody>
    </table>
  `;
}

function formatNum(v: number | null): string {
  if (v === null) return "—";
  return Number.isInteger(v) ? String(v) : v.toPrecision(6);
}

function formatCell(v: number | string | null): string {
  if (v === null) return '<span class="cell-null">—</span>';
  if (typeof v === "number") return Number.isInteger(v) ? String(v) : v.toPrecision(6);
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
