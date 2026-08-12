// §11.5: "normalise to area" -- presentation-only rescaling of R/k/Q/dV·dQ⁻¹/
// dQ·dV⁻¹ by the electrode area, never touching the underlying fit or
// smoothing data. Applied at the plotting/table-formatting layer only, per
// spec's own wording. Pure functions mirroring analysis/qc.ts's style.

import type { Dataset } from "../types";
import { scaleFactor } from "../units";

export type NormalizableColumn = "r" | "rErr" | "rSmooth" | "k" | "kErr" | "kSmooth" | "q" | "dVdQ" | "dQdV";

// Internal charge unit is Ah (units.ts), but normalized Q/dQdV/dVdQ are
// displayed in mAh -- the labels below promise mAh, so the Ah->mAh factor
// has to be applied here alongside the area scaling, not left implicit.
const AH_TO_MAH = 1 / scaleFactor("charge", "mAh");

interface NormalizationSpec {
  op: "multiply" | "divide";
  label: string;
  /** Extra unit-conversion factor applied before the area op, for columns carrying a charge unit (Ah -> mAh). */
  chargeUnitScale?: number;
}

const NORMALIZATION: Record<NormalizableColumn, NormalizationSpec> = {
  r: { op: "multiply", label: "Ω cm²" },
  rErr: { op: "multiply", label: "Ω cm²" },
  rSmooth: { op: "multiply", label: "Ω cm²" },
  k: { op: "multiply", label: "Ω cm²·s⁻¹ᐟ²" },
  kErr: { op: "multiply", label: "Ω cm²·s⁻¹ᐟ²" },
  kSmooth: { op: "multiply", label: "Ω cm²·s⁻¹ᐟ²" },
  q: { op: "divide", label: "mAh cm⁻²", chargeUnitScale: AH_TO_MAH },
  dVdQ: { op: "multiply", label: "V cm²·mAh⁻¹", chargeUnitScale: 1 / AH_TO_MAH },
  dQdV: { op: "divide", label: "mAh·V⁻¹·cm⁻²", chargeUnitScale: AH_TO_MAH },
};

type NormalizationDataset = Pick<Dataset, "normalizeToArea" | "electrodeAreaCm2">;

function isNormalizable(column: string): column is NormalizableColumn {
  return Object.prototype.hasOwnProperty.call(NORMALIZATION, column);
}

export function normalizeValue(column: string, value: number, dataset: NormalizationDataset): number {
  if (!dataset.normalizeToArea || !isNormalizable(column)) return value;
  const spec = NORMALIZATION[column];
  const scaled = value * (spec.chargeUnitScale ?? 1);
  return spec.op === "multiply" ? scaled * dataset.electrodeAreaCm2 : scaled / dataset.electrodeAreaCm2;
}

export function applyColumnNormalization(column: string, values: (number | null)[], dataset: NormalizationDataset): (number | null)[] {
  if (!dataset.normalizeToArea || !isNormalizable(column)) return values;
  return values.map((v) => (v === null ? null : normalizeValue(column, v, dataset)));
}

/** Returns `baseLabel` unchanged when normalization is off or the column isn't normalizable. */
export function normalizedLabel(column: string, baseLabel: string, dataset: NormalizationDataset): string {
  if (!dataset.normalizeToArea || !isNormalizable(column)) return baseLabel;
  return `${baseLabel} (${NORMALIZATION[column].label})`;
}
