// §11.4: purely structural diffing of dataset.additionalPlots -- mounts
// new ids, destroys removed ids, reorders DOM nodes for id-set/order
// changes; never touches per-instance config/range state (that all lives
// inside each mountAdditionalPlot closure). "+ Add plot" button; plain
// HTML5 drag-and-drop reordering (a function-scoped draggedId, not
// module-top-level, since this panel could in principle be mounted more
// than once for a future multi-file UI -- types.ts's own stated intent).

import { mountAdditionalPlot } from "../plots/additional-plot";
import { activeDataset, store, updateDataset } from "../state";
import { defaultAdditionalPlot } from "../types";

export function mountAdditionalPlotsPanel(container: HTMLElement): void {
  const instances = new Map<string, { host: HTMLElement; handle: { destroy: () => void } }>();
  let draggedId: string | null = null;

  const list = document.createElement("div");
  list.className = "additional-plots-list";
  container.appendChild(list);

  const addBtn = document.createElement("button");
  addBtn.type = "button";
  addBtn.className = "btn";
  addBtn.textContent = "+ Add plot";
  addBtn.addEventListener("click", () => {
    const dataset = activeDataset(store.get());
    if (!dataset) return;
    updateDataset(dataset.id, (d) => ({ ...d, additionalPlots: [...d.additionalPlots, defaultAdditionalPlot()] }));
  });
  container.appendChild(addBtn);

  function removePlot(id: string): void {
    const dataset = activeDataset(store.get());
    if (!dataset) return;
    updateDataset(dataset.id, (d) => ({ ...d, additionalPlots: d.additionalPlots.filter((p) => p.id !== id) }));
  }

  function reorder(sourceId: string, targetId: string): void {
    const dataset = activeDataset(store.get());
    if (!dataset) return;
    updateDataset(dataset.id, (d) => {
      const next = [...d.additionalPlots];
      const sourceIdx = next.findIndex((p) => p.id === sourceId);
      const targetIdx = next.findIndex((p) => p.id === targetId);
      if (sourceIdx === -1 || targetIdx === -1) return d;
      const [moved] = next.splice(sourceIdx, 1);
      next.splice(targetIdx, 0, moved);
      return { ...d, additionalPlots: next };
    });
  }

  function mountOne(id: string): { host: HTMLElement; handle: { destroy: () => void } } {
    const host = document.createElement("div");
    host.addEventListener("dragover", (e) => e.preventDefault());
    host.addEventListener("drop", (e) => {
      e.preventDefault();
      if (draggedId && draggedId !== id) reorder(draggedId, id);
      draggedId = null;
    });
    const handle = mountAdditionalPlot(host, id, {
      onRemove: () => removePlot(id),
      onDragStart: () => {
        draggedId = id;
      },
    });
    return { host, handle };
  }

  function render(): void {
    const dataset = activeDataset(store.get());
    if (!dataset?.stageAResult) {
      for (const inst of instances.values()) inst.handle.destroy();
      instances.clear();
      list.innerHTML = "";
      addBtn.style.display = "none";
      return;
    }
    addBtn.style.display = "";

    const ids = dataset.additionalPlots.map((p) => p.id);
    const idSet = new Set(ids);

    for (const [id, inst] of instances) {
      if (!idSet.has(id)) {
        inst.handle.destroy();
        inst.host.remove();
        instances.delete(id);
      }
    }

    for (const id of ids) {
      if (!instances.has(id)) instances.set(id, mountOne(id));
    }

    // Reorder DOM nodes to match array order -- appendChild on a node
    // already in the document moves it, it doesn't clone it.
    for (const id of ids) list.appendChild(instances.get(id)!.host);
  }

  store.subscribe(render);
  render();
}
