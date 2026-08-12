// §5.1 auto-mapping: header-name pattern matching (case-insensitive,
// priority order) plus a localStorage mapping-preset cache keyed by a hash
// of the header row.

import { emptyMapping, type ColumnMapping, type RequiredField } from "../types";

interface UnitPattern {
  regex: RegExp;
  unit: string;
}

interface FieldPatterns {
  field: RequiredField;
  patterns: RegExp[];
  unitPatterns?: UnitPattern[];
}

// Order matters within each field's pattern list (§5.1's own "priority
// order"), and for "cycle" the full-cycle patterns must precede the
// half-cycle one so a full cycle column is preferred when both exist.
const FIELD_PATTERNS: FieldPatterns[] = [
  {
    field: "time",
    patterns: [/^time\/s$/i, /^t$/i, /^time/i, /^elapsed/i],
    unitPatterns: [{ regex: /\/s$/i, unit: "s" }],
  },
  {
    field: "voltage",
    // Bare "E" is a weak, ambiguous signal -- it's shorthand for voltage in
    // some BioLogic-style exports, but MDF4 exports seen in practice use it
    // for *Energy* (Wh) instead, with "U" (IEC convention) as the actual
    // voltage channel. Tried last, after every less ambiguous candidate,
    // so a file that has both only picks "E" when nothing better exists.
    patterns: [/^Ewe\/V$/i, /^U$/i, /^ecell/i, /voltage/i, /^E$/i],
    unitPatterns: [
      { regex: /\/mV$/i, unit: "mV" },
      { regex: /\/V$/i, unit: "V" },
    ],
  },
  {
    field: "current",
    patterns: [/^I\/mA$/i, /^I$/i, /^current/i],
    unitPatterns: [
      { regex: /\/mA$/i, unit: "mA" },
      { regex: /\/A$/i, unit: "A" },
    ],
  },
  {
    field: "charge",
    patterns: [/^\(Q-Qo\)\/mA\.h$/i, /^Q$/i, /^capacity/i, /charge/i],
    unitPatterns: [
      { regex: /\/mA\.h$/i, unit: "mAh" },
      { regex: /\/A\.h$/i, unit: "Ah" },
    ],
  },
  {
    field: "cycle",
    patterns: [/^cycle number$/i, /^cyc\.?n$/i, /^cycle/i, /^half cycle/i],
  },
];

export function computeAutoMapping(columnNames: string[]): ColumnMapping {
  const mapping = emptyMapping();
  for (const fp of FIELD_PATTERNS) {
    let matched: string | undefined;
    for (const pattern of fp.patterns) {
      matched = columnNames.find((name) => pattern.test(name));
      if (matched) break;
    }
    if (!matched) continue;
    const inferredUnit = fp.unitPatterns?.find((u) => u.regex.test(matched as string))?.unit;
    mapping[fp.field] = {
      column: matched,
      unit: inferredUnit ?? mapping[fp.field].unit,
      autoDetected: true,
    };
  }
  return mapping;
}

/** Columns matching the "half cycle" pattern -- worth a distinguishing label in the dropdown. */
export function isHalfCycleColumn(name: string): boolean {
  return /^half cycle/i.test(name) && !/^cycle number$/i.test(name);
}

// ---------------------------------------------------------------------
// localStorage mapping presets (§5.1: "keyed by a hash of the header row")
// ---------------------------------------------------------------------

const PRESET_KEY_PREFIX = "ici-web:mapping-preset:";

/** Exported for §12.5's session restore -- the same "header signature" mechanism, reused there for a stricter match than §12.2's "Load config" per-column check. */
export function hashHeaderRow(columnNames: string[]): string {
  const joined = columnNames.join("");
  let hash = 5381;
  for (let i = 0; i < joined.length; i++) {
    hash = (hash * 33) ^ joined.charCodeAt(i);
  }
  return (hash >>> 0).toString(36);
}

export interface MappingPreset {
  mapping: ColumnMapping;
  groupingColumns: string[];
}

export function loadMappingPreset(columnNames: string[]): MappingPreset | null {
  try {
    const raw = localStorage.getItem(PRESET_KEY_PREFIX + hashHeaderRow(columnNames));
    if (!raw) return null;
    return JSON.parse(raw) as MappingPreset;
  } catch {
    return null;
  }
}

export function saveMappingPreset(columnNames: string[], preset: MappingPreset): void {
  try {
    localStorage.setItem(PRESET_KEY_PREFIX + hashHeaderRow(columnNames), JSON.stringify(preset));
  } catch {
    // localStorage unavailable/full -- not fatal, just skip persistence.
  }
}
