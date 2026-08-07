// §12.3: "timestamped, ordered list of every automatic decision and
// warning" -- derived from already-structured result data at
// stage-completion granularity (not per-individual-decision, which would
// need much deeper Rust instrumentation for marginal value). This single
// function backs both the run-log-panel.ts UI and the run-report export,
// satisfying spec's "the log shown in the UI... and the log in this
// export are the same object."

import { rankCandidates } from "../panels/regression-window-panel";
import type { Dataset, LogEntry, NonphysicalReport } from "../types";

function nonphysicalEntries(timestamp: number, label: string, byState: NonphysicalReport["rByState"]): LogEntry[] {
  const entries = Object.entries(byState).filter(([, n]) => n > 0);
  if (entries.length === 0) return [];
  const total = entries.reduce((sum, [, n]) => sum + n, 0);
  const clustered = entries.length === 1;
  const breakdown = entries.map(([s, n]) => `${s}: ${n}`).join(", ");
  return [
    {
      timestamp,
      category: "stageA",
      severity: "warning",
      message: `${total} non-physical ${label} value(s) set to NA (${breakdown})${clustered ? " -- clustered in one state, possibly a sign/orientation issue" : ""}`,
    },
  ];
}

export function buildRunLog(dataset: Dataset): LogEntry[] {
  const entries: LogEntry[] = [];

  if (dataset.parseCompletedAt !== null && dataset.report) {
    const t = dataset.parseCompletedAt;
    for (const w of dataset.report.warnings) {
      entries.push({ timestamp: t, category: "parse", severity: "warning", message: w });
    }
    for (const c of dataset.report.coercionFailures) {
      entries.push({ timestamp: t, category: "parse", severity: "warning", message: `${c.count} value(s) in '${c.column}' failed to parse as numeric` });
    }
    if (dataset.report.raggedRowsDropped > 0) {
      entries.push({ timestamp: t, category: "parse", severity: "warning", message: `${dataset.report.raggedRowsDropped} ragged row(s) dropped` });
    }
    if (dataset.prenormalizedChargeNote) {
      entries.push({ timestamp: t, category: "parse", severity: "info", message: "Charge column already looks reset per half-cycle -- Q anchoring is a no-op" });
    }
    for (const issue of dataset.validation) {
      entries.push({ timestamp: t, category: "parse", severity: "warning", message: issue.message });
    }
  }

  // Stage A's own segmentation carries the authoritative reversal/incomplete-step
  // counts for the run that actually produced results; the cheaper reactive
  // `dataset.segmentation` (Plot 1/2's own path) is the fallback before Stage A
  // has run at all -- both share the same `SegmentationSummary` shape.
  const segmentation = dataset.stageAResult?.segmentation ?? dataset.segmentation;
  const segmentationTimestamp = dataset.stageACompletedAt ?? dataset.parseCompletedAt;
  if (segmentation && segmentationTimestamp !== null) {
    const t = segmentationTimestamp;
    if (segmentation.reversalRowsDropped > 0) {
      entries.push({ timestamp: t, category: "segmentation", severity: "info", message: `${segmentation.reversalRowsDropped} unrested reversal row(s) dropped` });
    }
    if (segmentation.incompleteFinalRowsDropped > 0) {
      entries.push({ timestamp: t, category: "segmentation", severity: "info", message: `${segmentation.incompleteFinalRowsDropped} incomplete-final-step row(s) dropped` });
    }
    if (segmentation.leadingRestRowsDropped > 0) {
      entries.push({
        timestamp: t,
        category: "segmentation",
        severity: "warning",
        message: `${segmentation.leadingRestRowsDropped} leading rest row(s) dropped (no preceding active run)`,
      });
    }
  }

  if (dataset.stageACompletedAt !== null && dataset.stageAResult) {
    const t = dataset.stageACompletedAt;
    const result = dataset.stageAResult;
    const totalRests = result.segmentation.totalRests;
    const failCount = result.regressionFailures.length;
    entries.push({ timestamp: t, category: "stageA", severity: "info", message: `${totalRests - failCount} of ${totalRests} rests fitted successfully` });
    for (const f of result.regressionFailures) {
      entries.push({ timestamp: t, category: "stageA", severity: "warning", message: `rest ${f.rest} (group ${f.groupId}): ${f.reason}` });
    }
    entries.push(...nonphysicalEntries(t, "R", result.nonphysicalReport.rByState));
    entries.push(...nonphysicalEntries(t, "k", result.nonphysicalReport.kByState));
  }

  if (dataset.stageBCompletedAt !== null && dataset.stageBResult) {
    const t = dataset.stageBCompletedAt;
    for (const group of dataset.stageBResult.groups) {
      const parts: string[] = [];
      for (const [label, d] of [
        ["E0", group.e0],
        ["k", group.k],
        ["R", group.r],
      ] as const) {
        parts.push(d ? `${label}: k=${d.kEffective}, λ=${d.lambda.toPrecision(3)}, edf=${d.edf.toFixed(1)}, dir=${d.directionUsed}` : `${label}: NA (<8 distinct x)`);
      }
      entries.push({ timestamp: t, category: "stageB", severity: "info", message: `${group.label} (n=${group.n}) -- ${parts.join("; ")}` });
    }
  }

  if (dataset.optimalWindowCompletedAt !== null && dataset.optimalWindowResult) {
    const t = dataset.optimalWindowCompletedAt;
    const result = dataset.optimalWindowResult;
    if (result.heterogeneousLengths) {
      entries.push({ timestamp: t, category: "optimalWindow", severity: "info", message: "Rest lengths vary -- used the 5th percentile of per-rest max step.t as the candidate grid's upper bound" });
    }
    entries.push({ timestamp: t, category: "optimalWindow", severity: "info", message: `Sampled ${result.sampledRests.length} rest(s) for the candidate search` });
    const top = rankCandidates(result.scores).slice(0, 5);
    if (top.length === 0) {
      entries.push({ timestamp: t, category: "optimalWindow", severity: "warning", message: "No candidate window passed the 80% fit-rate threshold" });
    } else {
      const summary = top.map((c) => `(${c.tMin}, ${c.tMax})s meanAdjR²=${c.meanAdjR2.toPrecision(4)}`).join("; ");
      entries.push({ timestamp: t, category: "optimalWindow", severity: "info", message: `Top candidates: ${summary}` });
    }
  }

  return entries.sort((a, b) => a.timestamp - b.timestamp);
}
