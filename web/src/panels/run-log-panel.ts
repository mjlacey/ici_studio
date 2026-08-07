// §12.3: a collapsible left-column panel over `buildRunLog`'s derived,
// timestamped entries -- the exact same function backs the run-report
// export, satisfying spec's "the log shown in the UI and the log in this
// export are the same object." A badge shows the count of warning entries
// newer than the last time the panel was opened.

import { buildRunLog } from "../analysis/run-log";
import { activeDataset, store } from "../state";
import type { Dataset, LogEntry } from "../types";

export function mountRunLogPanel(container: HTMLElement): void {
  let lastSeenAt = 0;
  let isOpen = false;

  function render(): void {
    const dataset = activeDataset(store.get());
    if (!dataset) {
      container.innerHTML = "";
      return;
    }
    renderPanel(container, dataset, lastSeenAt, isOpen, (open) => {
      isOpen = open;
      if (open) {
        lastSeenAt = Date.now();
        render();
      }
    });
  }

  store.subscribe(render);
  render();
}

function renderPanel(container: HTMLElement, dataset: Dataset, lastSeenAt: number, isOpen: boolean, onToggle: (open: boolean) => void): void {
  const entries = buildRunLog(dataset);
  const newWarnings = entries.filter((e) => e.severity === "warning" && e.timestamp > lastSeenAt).length;

  const rows = entries
    .map((e) => `<li class="run-log-entry${e.severity === "warning" ? " issue issue-warning" : ""}">${formatTime(e.timestamp)} <span class="run-log-category">${categoryLabel(e.category)}</span> ${escapeHtml(e.message)}</li>`)
    .join("");

  container.innerHTML = `
    <section class="panel">
      <details class="run-log" ${isOpen ? "open" : ""}>
        <summary>Run log${entries.length ? ` (${entries.length})` : ""}${newWarnings > 0 ? ` <span class="badge">${newWarnings} new</span>` : ""}</summary>
        ${entries.length === 0 ? `<p class="hint">Nothing logged yet.</p>` : `<ul class="run-log-list">${rows}</ul>`}
      </details>
    </section>
  `;

  container.querySelector<HTMLDetailsElement>(".run-log")!.addEventListener("toggle", (e) => {
    onToggle((e.target as HTMLDetailsElement).open);
  });
}

function categoryLabel(category: LogEntry["category"]): string {
  switch (category) {
    case "parse":
      return "parse";
    case "segmentation":
      return "segmentation";
    case "stageA":
      return "Stage A";
    case "stageB":
      return "Stage B";
    case "optimalWindow":
      return "optimal window";
  }
}

function formatTime(ts: number): string {
  return new Date(ts).toLocaleTimeString();
}

function escapeHtml(s: string): string {
  const div = document.createElement("div");
  div.textContent = s;
  return div.innerHTML;
}
