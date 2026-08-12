import "./style.css";
import { startStageBEffect } from "./analysis/stage-runner";
import { startSessionPersistEffect } from "./analysis/session-persist";
import { activeDataset, store } from "./state";
import { cycleThemePreference, getThemePreference, initTheme, type ThemePreference } from "./theme";
import { mountAdditionalPlotsPanel } from "./panels/additional-plots-panel";
import { mountColumnMappingPanel } from "./panels/column-mapping-panel";
import { mountExportPanel } from "./panels/export-panel";
import { mountImportPanel } from "./panels/import-panel";
import { mountParseCard } from "./panels/parse-card";
import { mountRawDataInspector } from "./panels/raw-data-inspector";
import { mountQcPanel } from "./panels/qc-panel";
import { mountRegressionWindowPanel } from "./panels/regression-window-panel";
import { mountResultsTable } from "./panels/results-table";
import { mountRestoreBanner } from "./panels/restore-banner";
import { mountRunLogPanel } from "./panels/run-log-panel";
import { startSegmentationEffect } from "./panels/segmentation";
import { mountStageAPanel } from "./panels/stage-a-panel";
import { mountStageBPanel } from "./panels/stage-b-panel";
import { mountSummaryStats } from "./panels/summary-stats";
import { mountThresholdSuggestionBanner } from "./panels/threshold-suggestion-banner";
import { mountVisualisationPanel } from "./panels/visualisation-panel";
import { mountPlot1 } from "./plots/plot1-raw-timeseries";
import { mountPlot2 } from "./plots/plot2-diagnostic";
import { mountResistancePlot } from "./plots/plot34-resistance";
import { DataWorkerClient } from "./worker/client";

// Desktop-only three-column shell (playbook §4 / spec §5.2 raw-data-viewer
// placement): left = import/parse-card/mapping/regression-window
// (scrollable), centre = plots, right = raw data inspector.
initTheme();

const app = document.querySelector<HTMLDivElement>("#app")!;
app.innerHTML = `
  <header class="app-header">
    <span>ICI Studio <span class="hint">— ICI analysis</span></span>
    <button type="button" class="theme-toggle" id="theme-toggle"></button>
  </header>
  <div id="restore-banner-container"></div>
  <div id="threshold-suggestion-banner-container"></div>
  <div class="app-shell">
    <aside class="left-column" id="left-column"></aside>
    <main class="center-column" id="center-column">
      <p class="placeholder" id="center-placeholder">Map the required columns to see the raw time series and regression diagnostic.</p>
    </main>
    <aside class="right-column" id="right-column"></aside>
  </div>
`;

const THEME_LABEL: Record<ThemePreference, string> = { system: "🖥 System", light: "☀ Light", dark: "🌙 Dark" };
const themeToggle = document.querySelector<HTMLButtonElement>("#theme-toggle")!;
themeToggle.textContent = THEME_LABEL[getThemePreference()];
themeToggle.title = "Cycle theme: system / light / dark";
themeToggle.addEventListener("click", () => {
  themeToggle.textContent = THEME_LABEL[cycleThemePreference()];
});

mountRestoreBanner(document.querySelector<HTMLDivElement>("#restore-banner-container")!);
mountThresholdSuggestionBanner(document.querySelector<HTMLDivElement>("#threshold-suggestion-banner-container")!);

const worker = new DataWorkerClient();

const leftColumn = document.querySelector<HTMLElement>("#left-column")!;
const centerColumn = document.querySelector<HTMLElement>("#center-column")!;
const rightColumn = document.querySelector<HTMLElement>("#right-column")!;

const importContainer = document.createElement("div");
const parseCardContainer = document.createElement("div");
const mappingContainer = document.createElement("div");
const regressionWindowContainer = document.createElement("div");
const stageAContainer = document.createElement("div");
const qcContainer = document.createElement("div");
const stageBContainer = document.createElement("div");
const visualisationContainer = document.createElement("div");
const exportContainer = document.createElement("div");
const runLogContainer = document.createElement("div");
leftColumn.append(
  importContainer,
  parseCardContainer,
  mappingContainer,
  regressionWindowContainer,
  stageAContainer,
  qcContainer,
  stageBContainer,
  visualisationContainer,
  exportContainer,
  runLogContainer,
);

mountImportPanel(importContainer, worker);
mountParseCard(parseCardContainer, worker);
mountColumnMappingPanel(mappingContainer, worker);
mountRegressionWindowPanel(regressionWindowContainer, worker);
mountStageAPanel(stageAContainer, worker);
mountQcPanel(qcContainer);
mountStageBPanel(stageBContainer, worker);
mountVisualisationPanel(visualisationContainer);
mountExportPanel(exportContainer, worker);
mountRunLogPanel(runLogContainer);
mountRawDataInspector(rightColumn, worker);
startSegmentationEffect(worker);
startStageBEffect(worker);
startSessionPersistEffect();

mountPlot1(centerColumn, worker);
mountPlot2(centerColumn, worker);

const plot3Container = document.createElement("div");
const plot4Container = document.createElement("div");
const additionalPlotsContainer = document.createElement("div");
const resultsTableContainer = document.createElement("div");
const summaryStatsContainer = document.createElement("div");
centerColumn.append(plot3Container, plot4Container, additionalPlotsContainer, resultsTableContainer, summaryStatsContainer);

mountResistancePlot(plot3Container, { valueColumn: "r", title: "Plot 3 — R", baseUnit: "Ω" });
mountResistancePlot(plot4Container, { valueColumn: "k", title: "Plot 4 — k", baseUnit: "Ω·s⁻¹ᐟ²" });
mountAdditionalPlotsPanel(additionalPlotsContainer);
mountResultsTable(resultsTableContainer);
mountSummaryStats(summaryStatsContainer);

const centerPlaceholder = document.querySelector<HTMLParagraphElement>("#center-placeholder")!;
function updateCenterPlaceholder(): void {
  const dataset = activeDataset(store.get());
  centerPlaceholder.style.display = dataset?.segmentation ? "none" : "";
}
store.subscribe(updateCenterPlaceholder);
updateCenterPlaceholder();
