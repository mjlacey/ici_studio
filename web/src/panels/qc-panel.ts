// §9 Quality control panel. Six per-rest flags (enabled toggle, editable
// threshold, exclude-from-smoothing toggle) and a summary line with
// click-through filtering into the results table. Mounted in the left
// column between Stage A and Stage B (spec's own placement).

import { qcSummary } from "../analysis/qc";
import { activeDataset, store, updateDataset } from "../state";
import type { Dataset, QcConfig, QcTableFilter } from "../types";

interface FlagDef {
  key: keyof QcConfig;
  label: string;
  hasThreshold: boolean;
  step: string;
}

const FLAG_DEFS: FlagDef[] = [
  { key: "poorFit", label: "Poor fit (adj R² <)", hasThreshold: true, step: "0.001" },
  { key: "tooFewPoints", label: "Too few points (n_pts <)", hasThreshold: true, step: "1" },
  { key: "edgeCurvature", label: "Edge curvature (edge_max_z >)", hasThreshold: true, step: "0.1" },
  { key: "edgeImbalance", label: "Edge/centre imbalance (edge_mae_ratio >)", hasThreshold: true, step: "0.1" },
  { key: "nonphysical", label: "Non-physical (R or k is NA)", hasThreshold: false, step: "" },
  { key: "degenerateDeltaI", label: "Degenerate ΔI (|ΔI| <)", hasThreshold: true, step: "1e-10" },
];

export function mountQcPanel(container: HTMLElement): void {
  function render(): void {
    const dataset = activeDataset(store.get());
    if (!dataset?.stageAResult) {
      container.innerHTML = "";
      return;
    }
    renderPanel(container, dataset);
  }

  store.subscribe(render);
  render();
}

function renderPanel(container: HTMLElement, dataset: Dataset): void {
  const cfg = dataset.qcConfig;
  const summary = qcSummary(dataset.stageAResult!.analysisTable, cfg, dataset.manualExclusions);

  const rows = FLAG_DEFS.map((def) => {
    const setting = cfg[def.key];
    return `
      <tr>
        <th>${def.label}</th>
        <td><input type="checkbox" data-flag="${def.key}.enabled" ${setting.enabled ? "checked" : ""} /></td>
        <td>${
          def.hasThreshold
            ? `<input type="number" step="${def.step}" data-flag="${def.key}.threshold" value="${setting.threshold}" />`
            : `<span class="hint">—</span>`
        }</td>
        <td><input type="checkbox" data-flag="${def.key}.excludeFromSmoothing" ${setting.excludeFromSmoothing ? "checked" : ""} /></td>
      </tr>`;
  }).join("");

  container.innerHTML = `
    <section class="panel">
      <h2>Quality control</h2>
      <table class="mapping-table qc-table">
        <thead><tr><th>Flag</th><th>On</th><th>Threshold</th><th>Exclude</th></tr></thead>
        <tbody>${rows}</tbody>
      </table>
      <p class="hint qc-summary">
        ${summary.total} rest(s) ·
        <button type="button" class="link-btn" data-filter="flagged">${summary.flagged} flagged</button> ·
        <button type="button" class="link-btn" data-filter="excluded">${summary.excluded} excluded</button>
        ${dataset.qcTableFilter !== "all" ? `· <button type="button" class="link-btn" data-filter="all">show all</button>` : ""}
      </p>
    </section>
  `;

  wireInputs(container, dataset);
}

function wireInputs(container: HTMLElement, dataset: Dataset): void {
  for (const el of container.querySelectorAll<HTMLInputElement>("[data-flag]")) {
    el.addEventListener("change", () => {
      const [flagKey, field] = el.dataset.flag!.split(".") as [keyof QcConfig, "enabled" | "threshold" | "excludeFromSmoothing"];
      updateDataset(dataset.id, (d) => {
        const setting = { ...d.qcConfig[flagKey] };
        if (field === "enabled" || field === "excludeFromSmoothing") setting[field] = el.checked;
        else setting.threshold = Number(el.value);
        return { ...d, qcConfig: { ...d.qcConfig, [flagKey]: setting } };
      });
    });
  }

  for (const btn of container.querySelectorAll<HTMLButtonElement>("[data-filter]")) {
    btn.addEventListener("click", () => {
      const filter = btn.dataset.filter as QcTableFilter;
      updateDataset(dataset.id, (d) => ({ ...d, qcTableFilter: filter }));
    });
  }
}
