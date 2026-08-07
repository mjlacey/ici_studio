// §2.2: internal-unit conversion. Every unit in the milestone-6 mapping
// panel maps to its internal unit (s / A / V / Ah) by a single multiplicative
// scale factor -- nothing in this app's unit set needs an offset (e.g. no
// Celsius-style conversions), so `value * scale` is always exact.

import type { ChargeUnit, CurrentUnit, RequiredField, TimeUnit, VoltageUnit } from "./types";

const TIME_SCALE: Record<TimeUnit, number> = {
  s: 1,
  min: 60,
  h: 3600,
};

const CURRENT_SCALE: Record<CurrentUnit, number> = {
  A: 1,
  mA: 1e-3,
  uA: 1e-6,
};

const VOLTAGE_SCALE: Record<VoltageUnit, number> = {
  V: 1,
  mV: 1e-3,
};

const CHARGE_SCALE: Record<ChargeUnit, number> = {
  Ah: 1,
  mAh: 1e-3,
  uAh: 1e-6,
  C: 1 / 3600,
};

/** The multiplicative factor from `unit` to this field's internal unit. `null`/unknown unit -> 1 (no-op). */
export function scaleFactor(field: RequiredField, unit: string | null): number {
  if (!unit) return 1;
  switch (field) {
    case "time":
      return TIME_SCALE[unit as TimeUnit] ?? 1;
    case "current":
      return CURRENT_SCALE[unit as CurrentUnit] ?? 1;
    case "voltage":
      return VOLTAGE_SCALE[unit as VoltageUnit] ?? 1;
    case "charge":
      return CHARGE_SCALE[unit as ChargeUnit] ?? 1;
    case "cycle":
      return 1;
  }
}
