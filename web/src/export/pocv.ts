// "pOCV" export: just Q and the smoothed E0 curve (renamed "E") for one
// selected charge or discharge half-cycle, for direct compatibility with
// external pseudo-OCV tooling that expects exactly those two columns under
// those names -- nothing else this app exports matches that shape. A
// "half-cycle" here is a Stage B smoothing group (§8.1): for the default
// smoothing key (cyc.n + state) that's exactly one charge or discharge
// half-cycle, and reusing the group the E0 curve was actually smoothed
// within keeps this consistent with what Plots 3/4 draw for it.

import type { Dataset } from "../types";

export interface PocvGroupOption {
  key: string;
  label: string;
  n: number;
}

/** Stage B smoothing groups that have at least one finite E0_smooth point -- the only ones worth offering here. */
export function pocvGroupOptions(dataset: Dataset): PocvGroupOption[] {
  const stageB = dataset.stageBResult;
  if (!stageB) return [];

  const pointCounts = new Map<string, number>();
  for (let i = 0; i < stageB.rowGroupKey.length; i++) {
    if (stageB.e0Smooth[i] === null) continue;
    const key = stageB.rowGroupKey[i];
    pointCounts.set(key, (pointCounts.get(key) ?? 0) + 1);
  }

  return stageB.groups
    .filter((g) => (pointCounts.get(g.key) ?? 0) > 0)
    .map((g) => ({ key: g.key, label: g.label, n: pointCounts.get(g.key)! }));
}

/** `Q\tE` (`E` = that group's smoothed `E0`), sorted by ascending `Q`, one selected half-cycle only. */
export function buildPocvTsv(dataset: Dataset, groupKey: string): string {
  const table = dataset.stageAResult?.analysisTable;
  const stageB = dataset.stageBResult;
  if (!table || !stageB) return "";

  const rows: { q: number; e: number }[] = [];
  for (let i = 0; i < stageB.rowGroupKey.length; i++) {
    if (stageB.rowGroupKey[i] !== groupKey) continue;
    const e = stageB.e0Smooth[i];
    if (e === null) continue;
    rows.push({ q: table.q[i], e });
  }
  rows.sort((a, b) => a.q - b.q);

  const lines = ["Q\tE", ...rows.map((r) => `${r.q}\t${r.e}`)];
  return lines.join("\n") + "\n";
}

/**
 * `Q\tE\tR\tk` (`E`/`R`/`k` = that group's smoothed E0/R/k), sorted by
 * ascending `Q`, one selected half-cycle only. A row missing any of the
 * three smoothed values (rare -- each smoother fits independently, see
 * `runStageB` in wasm/src/lib.rs) is skipped rather than emitted with a
 * gap, same "finite or not present at all" rule as `buildPocvTsv`.
 */
export function buildPocvRkTsv(dataset: Dataset, groupKey: string): string {
  const table = dataset.stageAResult?.analysisTable;
  const stageB = dataset.stageBResult;
  if (!table || !stageB) return "";

  const rows: { q: number; e: number; r: number; k: number }[] = [];
  for (let i = 0; i < stageB.rowGroupKey.length; i++) {
    if (stageB.rowGroupKey[i] !== groupKey) continue;
    const e = stageB.e0Smooth[i];
    const r = stageB.rSmooth[i];
    const k = stageB.kSmooth[i];
    if (e === null || r === null || k === null) continue;
    rows.push({ q: table.q[i], e, r, k });
  }
  rows.sort((a, b) => a.q - b.q);

  const lines = ["Q\tE\tR\tk", ...rows.map((row) => `${row.q}\t${row.e}\t${row.r}\t${row.k}`)];
  return lines.join("\n") + "\n";
}
