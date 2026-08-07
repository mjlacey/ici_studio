// §11.5: electrode area + "normalise to area" toggle. A sample property,
// visible as soon as a dataset exists -- independent of pipeline stage,
// unlike the additional-plots panel (which needs an AnalysisTable to mean
// anything).

import { activeDataset, store, updateDataset } from "../state";
import type { Dataset } from "../types";

export function mountVisualisationPanel(container: HTMLElement): void {
  function render(): void {
    const dataset = activeDataset(store.get());
    if (!dataset) {
      container.innerHTML = "";
      return;
    }
    renderPanel(container, dataset);
  }

  store.subscribe(render);
  render();
}

function renderPanel(container: HTMLElement, dataset: Dataset): void {
  const showWarning = dataset.normalizeToArea && dataset.electrodeAreaCm2 === 1.0;

  container.innerHTML = `
    <section class="panel">
      <h2>Visualisation</h2>
      <table class="mapping-table">
        <tbody>
          <tr>
            <th>Electrode area (cm²)</th>
            <td><input type="number" min="0.0001" step="any" data-field="electrodeAreaCm2" value="${dataset.electrodeAreaCm2}" /></td>
          </tr>
          <tr>
            <th>Normalise to area</th>
            <td>
              <label class="toggle">
                <input type="checkbox" data-field="normalizeToArea" ${dataset.normalizeToArea ? "checked" : ""} /> on
              </label>
              ${showWarning ? `<span class="warning-marker" title="Area is still the default 1.0 -- normalisation is a no-op until the real electrode area is entered.">⚠ area is still 1.0</span>` : ""}
            </td>
          </tr>
          <tr>
            <th>Absolute Q</th>
            <td>
              <label class="toggle">
                <input type="checkbox" data-field="absoluteQ" ${dataset.absoluteQ ? "checked" : ""} /> on
              </label>
              <span class="hint">Q anchoring legitimately crosses zero between charge/discharge branches -- display-only, doesn't affect fitting.</span>
            </td>
          </tr>
          <tr>
            <th>Absolute dV/dQ</th>
            <td>
              <label class="toggle">
                <input type="checkbox" data-field="absoluteDVDQ" ${dataset.absoluteDVDQ ? "checked" : ""} /> on
              </label>
              <span class="hint">dQ/dV keeps its natural charge/discharge sign; dV/dQ (its reciprocal) is often read as always positive.</span>
            </td>
          </tr>
        </tbody>
      </table>
    </section>
  `;

  container.querySelector<HTMLInputElement>('[data-field="electrodeAreaCm2"]')!.addEventListener("change", (e) => {
    const v = Math.max(0.0001, Number((e.target as HTMLInputElement).value) || 1);
    updateDataset(dataset.id, (d) => ({ ...d, electrodeAreaCm2: v }));
  });
  container.querySelector<HTMLInputElement>('[data-field="normalizeToArea"]')!.addEventListener("change", (e) => {
    const checked = (e.target as HTMLInputElement).checked;
    updateDataset(dataset.id, (d) => ({ ...d, normalizeToArea: checked }));
  });
  container.querySelector<HTMLInputElement>('[data-field="absoluteQ"]')!.addEventListener("change", (e) => {
    const checked = (e.target as HTMLInputElement).checked;
    updateDataset(dataset.id, (d) => ({ ...d, absoluteQ: checked }));
  });
  container.querySelector<HTMLInputElement>('[data-field="absoluteDVDQ"]')!.addEventListener("change", (e) => {
    const checked = (e.target as HTMLInputElement).checked;
    updateDataset(dataset.id, (d) => ({ ...d, absoluteDVDQ: checked }));
  });
}
