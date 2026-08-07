// §11.5: "normalise to area" -- presentation-only rescaling of R/k/Q/dV·dQ⁻¹/
// dQ·dV⁻¹ by the electrode area, never touching the underlying fit or
// smoothing data. Applied at the plotting/table-formatting layer only, per
// spec's own wording. Pure functions mirroring analysis/qc.ts's style.

import type { Dataset } from "../types";

export type NormalizableColumn = "r" | "rErr" | "rSmooth" | "k" | "kErr" | "kSmooth" | "q" | "dVdQ" | "dQdV";

interface NormalizationSpec {
  op: "multiply" | "divide";
  label: string;
}

const NORMALIZATION: Record<NormalizableColumn, NormalizationSpec> = {
  r: { op: "multiply", label: "Ω cm²" },
  rErr: { op: "multiply", label: "Ω cm²" },
  rSmooth: { op: "multiply", label: "Ω cm²" },
  k: { op: "multiply", label: "Ω cm²·s⁻¹ᐟ²" },
  kErr: { op: "multiply", label: "Ω cm²·s⁻¹ᐟ²" },
  kSmooth: { op: "multiply", label: "Ω cm²·s⁻¹ᐟ²" },
  q: { op: "divide", label: "mAh cm⁻²" },
  dVdQ: { op: "multiply", label: "V cm²·mAh⁻¹" },
  dQdV: { op: "divide", label: "mAh·V⁻¹·cm⁻²" },
};

type NormalizationDataset = Pick<Dataset, "normalizeToArea" | "electrodeAreaCm2">;

function isNormalizable(column: string): column is NormalizableColumn {
  return Object.prototype.hasOwnProperty.call(NORMALIZATION, column);
}

export function normalizeValue(column: string, value: number, dataset: NormalizationDataset): number {
  if (!dataset.normalizeToArea || !isNormalizable(column)) return value;
  const spec = NORMALIZATION[column];
  return spec.op === "multiply" ? value * dataset.electrodeAreaCm2 : value / dataset.electrodeAreaCm2;
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
