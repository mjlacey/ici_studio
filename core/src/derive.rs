//! Derived resistance/diffusion-coefficient quantities and non-physical
//! value handling. Faithful port of `ici_analysis.R` lines 1003-1032
//! (§7.8 of ICI_WEB_SPEC.md).

use crate::regress::RestRegression;
use crate::segment::{InterruptionSummary, State};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy)]
pub struct DeriveConfig {
    /// Any `R`/`k` that is non-finite or negative is set to NA (along with
    /// its `_err` partner) when true. `R` and `k` are resistances and are
    /// always physically positive (§13.3) -- this is never disabled for a
    /// "real" run, only for advanced diagnosis of a suspected sign bug.
    pub nonphysical_to_na: bool,
}

impl Default for DeriveConfig {
    fn default() -> Self {
        Self {
            nonphysical_to_na: true,
        }
    }
}

/// The merged, per-interruption analysis table (R's `result` / `analysis`).
/// Row order follows `summary`'s order; rows whose rest has no matching
/// (successful) regression are dropped, mirroring R's inner `merge()`.
#[derive(Debug, Clone, Default)]
pub struct AnalysisTable {
    pub group_id: Vec<u32>,
    pub cyc_n: Vec<f64>,
    pub state: Vec<State>,
    pub rest: Vec<u32>,
    pub t: Vec<f64>,
    pub step_t: Vec<f64>,
    pub e: Vec<f64>,
    pub i: Vec<f64>,
    pub q: Vec<f64>,
    pub e0: Vec<f64>,
    pub e0_err: Vec<f64>,
    pub s: Vec<f64>,
    pub s_err: Vec<f64>,
    pub i0: Vec<f64>,
    pub n_pts: Vec<usize>,
    pub r2: Vec<f64>,
    pub adj_r2: Vec<f64>,
    pub rmse: Vec<f64>,
    pub edge_mae_ratio: Vec<f64>,
    pub edge_max_z: Vec<f64>,
    pub r: Vec<f64>,
    pub r_err: Vec<f64>,
    pub k: Vec<f64>,
    pub k_err: Vec<f64>,
}

/// Counts of non-physical (non-finite or negative) `R`/`k` values, broken
/// down by state -- negatives clustered in one state specifically indicate
/// a sign/orientation bug rather than noise (§7.8, §13.3).
#[derive(Debug, Clone, Default)]
pub struct NonphysicalReport {
    pub r_by_state: HashMap<&'static str, usize>,
    pub k_by_state: HashMap<&'static str, usize>,
}

/// §7.8: merges `summary` and `regression` on `(group, rest)`, then derives
/// `R`, `k`, and their errors from `ΔI = I - I0`.
pub fn derive(
    summary: &InterruptionSummary,
    regression: &RestRegression,
    config: &DeriveConfig,
) -> (AnalysisTable, NonphysicalReport) {
    let mut regr_index: HashMap<(u32, u32), usize> = HashMap::new();
    for i in 0..regression.rest.len() {
        regr_index.insert((regression.group_id[i], regression.rest[i]), i);
    }

    let mut out = AnalysisTable::default();
    let mut report = NonphysicalReport::default();

    for i in 0..summary.rest.len() {
        let key = (summary.group_id[i], summary.rest[i]);
        let Some(&j) = regr_index.get(&key) else {
            continue;
        };

        let e = summary.e[i];
        let i_value = summary.i[i];
        let i0 = regression.i0[j];
        let e0 = regression.e0[j];
        let s = regression.s[j];
        let e0_err = regression.e0_err[j];
        let s_err = regression.s_err[j];

        let delta_i = i_value - i0;
        let mut r = (e - e0) / delta_i;
        let mut r_err = e0_err / delta_i.abs();
        let mut k = -s / delta_i;
        let mut k_err = s_err / delta_i.abs();

        let state_str = summary.state[i].as_str();
        let r_bad = !r.is_finite() || r < 0.0;
        let k_bad = !k.is_finite() || k < 0.0;
        if r_bad {
            *report.r_by_state.entry(state_str).or_insert(0) += 1;
        }
        if k_bad {
            *report.k_by_state.entry(state_str).or_insert(0) += 1;
        }

        if config.nonphysical_to_na {
            if r_bad {
                r = f64::NAN;
                r_err = f64::NAN;
            }
            if k_bad {
                k = f64::NAN;
                k_err = f64::NAN;
            }
        }

        out.group_id.push(summary.group_id[i]);
        out.cyc_n.push(summary.cyc_n[i]);
        out.state.push(summary.state[i]);
        out.rest.push(summary.rest[i]);
        out.t.push(summary.t[i]);
        out.step_t.push(summary.step_t[i]);
        out.e.push(e);
        out.i.push(i_value);
        out.q.push(summary.q[i]);
        out.e0.push(e0);
        out.e0_err.push(e0_err);
        out.s.push(s);
        out.s_err.push(s_err);
        out.i0.push(i0);
        out.n_pts.push(regression.n_pts[j]);
        out.r2.push(regression.r2[j]);
        out.adj_r2.push(regression.adj_r2[j]);
        out.rmse.push(regression.rmse[j]);
        out.edge_mae_ratio.push(regression.edge_mae_ratio[j]);
        out.edge_max_z.push(regression.edge_max_z[j]);
        out.r.push(r);
        out.r_err.push(r_err);
        out.k.push(k);
        out.k_err.push(k_err);
    }

    (out, report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::regress::{rest_regression, RegressionConfig};
    use crate::segment::{interruption_summary, segment, SegmentConfig, SummaryConfig};

    /// A single clean charge interruption: build it through the whole
    /// segment -> summary -> regression -> derive pipeline and check R/k by
    /// hand from the fitted line.
    #[test]
    fn derive_matches_hand_computed_r_and_k() {
        // Rest (I=0) for 3 points, then charge (I=2A) for 2 points, then
        // rest again with E = E0 + s*sqrt(step_t) exactly (noise-free).
        let e0_true = 3.0;
        let s_true = -0.01;
        let current = [0.0, 0.0, 0.0, 2.0, 2.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        let n = current.len();
        let t: Vec<f64> = (0..n).map(|i| i as f64).collect();
        let cyc_n = vec![1.0; n];
        let charge = vec![0.0; n];
        // Rest-phase voltage after the pulse follows E0 + s*sqrt(step_t);
        // step_t for the second rest run (indices 5..10) restarts at 0.
        // Note: deliberately not equal to e0_true -- an (E - E0) that's
        // mathematically exactly zero is sign-unstable under floating-point
        // rounding (the fitted E0 and the hand-picked E would each carry
        // independent last-bit noise), which would make R's sign arbitrary.
        let e_active = 3.05;
        let voltage: Vec<f64> = (0..n)
            .map(|i| {
                if i < 5 {
                    e_active
                } else {
                    let step_t = (i - 5) as f64;
                    e0_true + s_true * step_t.sqrt()
                }
            })
            .collect();

        let seg_config = SegmentConfig::default();
        let group_id = vec![0u32; n];
        let (seg, _log) = segment(
            &group_id,
            &t,
            &cyc_n,
            &current,
            &voltage,
            &charge,
            &seg_config,
        );

        let summary = interruption_summary(
            &seg,
            &SummaryConfig {
                voltage_interpolation_window: None,
                current_average_window: None,
                legacy_compatibility: false,
            },
        );
        let (regression, reg_log) = rest_regression(
            &seg,
            &RegressionConfig {
                regression_window: (0.0, 10.0),
                current_average_window: None,
                edge_points: 1,
            },
        );
        assert!(
            reg_log.failed_rests.is_empty(),
            "{:?}",
            reg_log.failed_rests
        );

        let (analysis, _report) = derive(&summary, &regression, &DeriveConfig::default());
        assert_eq!(analysis.rest.len(), 1);
        assert_eq!(analysis.state[0], State::Charge);

        // I0 (initial window of the rest, window=None) = the first rest
        // point's current = 0. I (final window of active, window=None) = 2.
        let delta_i = 2.0 - 0.0;
        let expected_r = (e_active - e0_true) / delta_i;
        let expected_k = -s_true / delta_i;

        assert!((analysis.r[0] - expected_r).abs() < 1e-9);
        assert!((analysis.k[0] - expected_k).abs() < 1e-9);
        assert!(analysis.r[0] > 0.0);
        assert!(analysis.k[0] > 0.0);
    }

    #[test]
    fn nonphysical_negative_r_is_nad_and_counted_by_state() {
        // Force a negative R by making E *rise* above E0 on a charge pulse
        // (E - E0 > 0 while delta_I > 0 gives R > 0 normally; flip E below
        // E0 here to force R < 0 and check it gets NaN'd and counted).
        let e0_true = 3.0;
        let s_true = -0.01;
        let current = [0.0, 0.0, 0.0, 2.0, 2.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        let n = current.len();
        let t: Vec<f64> = (0..n).map(|i| i as f64).collect();
        let cyc_n = vec![1.0; n];
        let charge = vec![0.0; n];
        let voltage: Vec<f64> = (0..n)
            .map(|i| {
                if i < 5 {
                    2.5 // below E0 -> forces (E - E0) < 0 -> R < 0
                } else {
                    let step_t = (i - 5) as f64;
                    e0_true + s_true * step_t.sqrt()
                }
            })
            .collect();

        let group_id = vec![0u32; n];
        let (seg, _log) = segment(
            &group_id,
            &t,
            &cyc_n,
            &current,
            &voltage,
            &charge,
            &SegmentConfig::default(),
        );
        let summary = interruption_summary(
            &seg,
            &SummaryConfig {
                voltage_interpolation_window: None,
                current_average_window: None,
                legacy_compatibility: false,
            },
        );
        let (regression, _log) = rest_regression(
            &seg,
            &RegressionConfig {
                regression_window: (0.0, 10.0),
                current_average_window: None,
                edge_points: 1,
            },
        );

        let (analysis, report) = derive(
            &summary,
            &regression,
            &DeriveConfig {
                nonphysical_to_na: true,
            },
        );
        assert!(analysis.r[0].is_nan());
        assert_eq!(report.r_by_state.get("charge"), Some(&1));

        let (analysis_raw, _) = derive(
            &summary,
            &regression,
            &DeriveConfig {
                nonphysical_to_na: false,
            },
        );
        assert!(analysis_raw.r[0] < 0.0);
    }
}
