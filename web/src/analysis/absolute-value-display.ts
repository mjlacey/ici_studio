// Presentation-only absolute-value display toggles, independent of and
// composable with area normalization (analysis/area-normalization.ts).
//
// "Absolute Q": §6's per-state Q anchoring legitimately makes Q cross zero
// between a charge branch and a discharge branch (each is independently
// zeroed at its own run start) -- correct by design, but visually
// confusing ("doubled-back" curves). This is a display-only abs() over
// already-anchored Q, watching out for the case where Q genuinely crosses
// zero *within* one branch's own raw data (abs() still does the right
// thing there -- it folds the curve at zero rather than hiding the
// crossing). The anchor start/end controls stay pure curve-direction
// preferences, untouched by this toggle.
//
// "Absolute dV/dQ": dQ/dV keeps its natural sign (positive for charge,
// negative for discharge -- ordinary physical convention); dV/dQ, its
// reciprocal, reads more naturally always positive. A separate user
// preference from "absolute Q".

import type { Dataset } from "../types";

export type AbsValueColumn = "q" | "dVdQ";

type AbsValueDataset = Pick<Dataset, "absoluteQ" | "absoluteDVDQ">;

function shouldAbs(column: string, dataset: AbsValueDataset): boolean {
  if (column === "q") return dataset.absoluteQ;
  if (column === "dVdQ") return dataset.absoluteDVDQ;
  return false;
}

export function applyAbsValue(column: string, value: number, dataset: AbsValueDataset): number {
  return shouldAbs(column, dataset) ? Math.abs(value) : value;
}

export function applyColumnAbsValue(column: string, values: (number | null)[], dataset: AbsValueDataset): (number | null)[] {
  if (!shouldAbs(column, dataset)) return values;
  return values.map((v) => (v === null ? null : Math.abs(v)));
}

/**
 * abs() of a [min, max] *range* (e.g. a group's Q span), not just a single
 * value -- naively abs()-ing each bound independently is wrong whenever the
 * raw range straddles zero (the exact "raw data crosses through Q = 0"
 * case flagged as a watch-out): [-5, 10] would naively become [5, 10],
 * silently losing that the range actually dips down to 0.
 */
export function applyAbsValueRange(column: string, min: number, max: number, dataset: AbsValueDataset): [number, number] {
  if (!shouldAbs(column, dataset)) return [min, max];
  if (min <= 0 && max >= 0) return [0, Math.max(Math.abs(min), Math.abs(max))];
  return [Math.min(Math.abs(min), Math.abs(max)), Math.max(Math.abs(min), Math.abs(max))];
}

/** Returns `baseLabel` unchanged when the toggle is off or the column doesn't support it. */
export function absValueLabel(column: string, baseLabel: string, dataset: AbsValueDataset): string {
  if (!shouldAbs(column, dataset)) return baseLabel;
  return `|${baseLabel}|`;
}
