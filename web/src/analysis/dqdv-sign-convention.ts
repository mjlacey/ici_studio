// Battery-science sign convention for dV/dQ and dQ/dV: positive during
// charge, negative during discharge. R's own script (ici_analysis.R lines
// 1074-1077) computes `local_poly_derivative(q, E0_smooth, ...)` from the
// same signed, per-state-anchored `Q` for both states with no state-aware
// sign handling -- so both states come out the same sign there (confirmed:
// with this app's Q convention, where discharge Q grows more negative over
// time while E also falls, dE/dQ is genuinely positive for both states).
// That's mathematically consistent, but not the convention users expect
// from a dQ/dV plot, where the two branches should mirror around zero.
//
// The fix is exactly what falls out of computing the derivative against
// |Q| (cumulative capacity magnitude, always increasing with time in both
// states) instead of signed Q: d|Q|/dt is the *negative* of dQ/dt when
// Q<0, which flips the sign of both dV/d|Q| and dQ/d|V| for discharge
// only. Applied once here, right when a Stage B result is received,
// rather than re-derived at every display site -- the underlying Stage B
// computation itself stays untouched (matches R, keeps golden-fixture
// fidelity in core/), this is a documented presentation-layer correction.
import type { AnalysisTable, StageBResult } from "../types";

export function applyDqdvSignConvention(result: StageBResult, table: AnalysisTable): StageBResult {
  const flip = (arr: (number | null)[]): (number | null)[] =>
    arr.map((v, i) => (v !== null && table.state[i] === "discharge" ? -v : v));
  return { ...result, dVdQ: flip(result.dVdQ), dQdV: flip(result.dQdV) };
}
