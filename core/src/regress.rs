//! Per-rest OLS regression `E ~ sqrt(step.t)` and its diagnostics. Faithful
//! port of `ici_analysis.R` lines 910-999 (§7.7 of ICI_WEB_SPEC.md).

use crate::segment::{SegmentedData, State};
use crate::types::select_initial_window;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy)]
pub struct RegressionConfig {
    /// `(t_min, t_max)` in seconds, `0 <= t_min < t_max`.
    pub regression_window: (f64, f64),
    pub current_average_window: Option<f64>,
    pub edge_points: usize,
}

/// One row per rest that fitted successfully (fewer than 3 usable points is
/// not a hard error here -- see [`RegressionLog::failed_rests`] -- unlike R,
/// which stops the whole run; §5.3/§7.7 deviation).
#[derive(Debug, Clone, Default)]
pub struct RestRegression {
    pub group_id: Vec<u32>,
    pub rest: Vec<u32>,
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
}

#[derive(Debug, Clone, Default)]
pub struct RegressionLog {
    /// `(group_id, rest, reason)` for every rest that could not be fitted.
    pub failed_rests: Vec<(u32, u32, String)>,
}

/// §7.7: fits every rest (`state == Rest` rows), grouped by `(group, rest)`.
/// Row order follows first appearance of each `(group, rest)` key among the
/// retained rows, same caveat as [`crate::segment::interruption_summary`].
pub fn rest_regression(
    seg: &SegmentedData,
    config: &RegressionConfig,
) -> (RestRegression, RegressionLog) {
    let n = seg.state.len();
    let rest_idx: Vec<usize> = (0..n).filter(|&i| seg.state[i] == State::Rest).collect();

    type Key = (u32, u32);
    let mut order: Vec<Key> = Vec::new();
    let mut groups: HashMap<Key, Vec<usize>> = HashMap::new();
    for &i in &rest_idx {
        let key: Key = (seg.group_id[i], seg.rest[i]);
        groups
            .entry(key)
            .or_insert_with(|| {
                order.push(key);
                Vec::new()
            })
            .push(i);
    }

    let mut out = RestRegression::default();
    let mut log = RegressionLog::default();

    for key in order {
        let idx = &groups[&key];
        let step_time: Vec<f64> = idx.iter().map(|&r| seg.step_t[r]).collect();
        let current: Vec<f64> = idx.iter().map(|&r| seg.current[r]).collect();
        let voltage: Vec<f64> = idx.iter().map(|&r| seg.voltage[r]).collect();

        let i0_values = select_initial_window(&step_time, &current, config.current_average_window);
        let i0 = mean_or_nan(&i0_values);

        match fit_rest_window(
            &step_time,
            &voltage,
            config.regression_window,
            config.edge_points,
        ) {
            Ok(fit) => {
                out.group_id.push(key.0);
                out.rest.push(key.1);
                out.e0.push(fit.e0);
                out.e0_err.push(fit.e0_err);
                out.s.push(fit.s);
                out.s_err.push(fit.s_err);
                out.i0.push(i0);
                out.n_pts.push(fit.n_pts);
                out.r2.push(fit.r2);
                out.adj_r2.push(fit.adj_r2);
                out.rmse.push(fit.rmse);
                out.edge_mae_ratio.push(fit.edge_mae_ratio);
                out.edge_max_z.push(fit.edge_max_z);
            }
            Err(RestFitError::TooFewPoints { n }) => {
                log.failed_rests.push((
                    key.0,
                    key.1,
                    format!(
                        "only {n} usable regression point(s) in the [{}, {}] s window (need >= 3)",
                        config.regression_window.0, config.regression_window.1
                    ),
                ));
            }
            Err(RestFitError::Degenerate) => {
                log.failed_rests.push((
                    key.0,
                    key.1,
                    "degenerate regression (zero variance in sqrt(step.t))".to_string(),
                ));
            }
        }
    }

    (out, log)
}

/// One rest's fit + diagnostics, independent of any particular rest's
/// group/id bookkeeping -- reused by [`rest_regression`] (bulk, Stage A) and
/// by the main-thread live single-rest preview (§10.2/§2.1, wired up in the
/// `wasm` crate) so the two never drift apart.
#[derive(Debug, Clone)]
pub struct RestFit {
    pub e0: f64,
    pub e0_err: f64,
    pub s: f64,
    pub s_err: f64,
    pub n_pts: usize,
    pub r2: f64,
    pub adj_r2: f64,
    pub rmse: f64,
    pub edge_mae_ratio: f64,
    pub edge_max_z: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RestFitError {
    TooFewPoints { n: usize },
    Degenerate,
}

/// §7.7: filters `(step_t, voltage)` to the `[window.0, window.1]` s range,
/// sorts by `sqrt(step_t)`, fits, and computes edge diagnostics. Fewer than
/// 3 usable points or a degenerate (zero-variance) window are reported as
/// distinct errors rather than merged into one "failed" case, since callers
/// (§10.1's live point-count feedback in particular) want to tell them apart.
pub fn fit_rest_window(
    step_t: &[f64],
    voltage: &[f64],
    window: (f64, f64),
    edge_points: usize,
) -> Result<RestFit, RestFitError> {
    let fit_idx: Vec<usize> = (0..step_t.len())
        .filter(|&k| {
            step_t[k].is_finite()
                && step_t[k] >= window.0
                && step_t[k] <= window.1
                && voltage[k].is_finite()
        })
        .collect();

    if fit_idx.len() < 3 {
        return Err(RestFitError::TooFewPoints { n: fit_idx.len() });
    }

    let xs_raw: Vec<f64> = fit_idx.iter().map(|&k| step_t[k].sqrt()).collect();
    let ys_raw: Vec<f64> = fit_idx.iter().map(|&k| voltage[k]).collect();
    let mut order_idx: Vec<usize> = (0..xs_raw.len()).collect();
    order_idx.sort_by(|&a, &b| xs_raw[a].partial_cmp(&xs_raw[b]).unwrap());
    let xs: Vec<f64> = order_idx.iter().map(|&i| xs_raw[i]).collect();
    let ys: Vec<f64> = order_idx.iter().map(|&i| ys_raw[i]).collect();

    let fit = ols_fit(&xs, &ys).ok_or(RestFitError::Degenerate)?;
    let edge_count = edge_points.min(fit.n / 2);
    let (edge_mae_ratio, edge_max_z) = edge_diagnostics(&fit.residuals, fit.rmse, edge_count);

    Ok(RestFit {
        e0: fit.intercept,
        e0_err: fit.intercept_err,
        s: fit.slope,
        s_err: fit.slope_err,
        n_pts: fit.n,
        r2: fit.r2,
        adj_r2: fit.adj_r2,
        rmse: fit.rmse,
        edge_mae_ratio,
        edge_max_z,
    })
}

fn mean_or_nan(values: &[f64]) -> f64 {
    if values.is_empty() {
        f64::NAN
    } else {
        values.iter().sum::<f64>() / values.len() as f64
    }
}

#[derive(Debug, Clone)]
struct OlsFit {
    intercept: f64,
    intercept_err: f64,
    slope: f64,
    slope_err: f64,
    n: usize,
    r2: f64,
    adj_r2: f64,
    /// Residual standard error (`sigma`, R's `rmse`), `n - 2` degrees of freedom.
    rmse: f64,
    /// Same order as the input `x`/`y` (x-sorted, per the caller here).
    residuals: Vec<f64>,
}

/// Simple OLS `y = intercept + slope * x`, via centred sums for numerical
/// stability (§7.7: "rather than the naive `Sum(x^2) - (Sum x)^2/n` form").
/// Requires `n >= 3` and non-degenerate `x` (not all identical).
fn ols_fit(x: &[f64], y: &[f64]) -> Option<OlsFit> {
    let n = x.len();
    debug_assert_eq!(n, y.len());
    if n < 3 {
        return None;
    }
    let nf = n as f64;
    let mean_x = x.iter().sum::<f64>() / nf;
    let mean_y = y.iter().sum::<f64>() / nf;
    let sxx: f64 = x.iter().map(|&xi| (xi - mean_x).powi(2)).sum();
    if sxx <= 0.0 {
        return None;
    }
    let sxy: f64 = x
        .iter()
        .zip(y.iter())
        .map(|(&xi, &yi)| (xi - mean_x) * (yi - mean_y))
        .sum();

    let slope = sxy / sxx;
    let intercept = mean_y - slope * mean_x;
    let residuals: Vec<f64> = x
        .iter()
        .zip(y.iter())
        .map(|(&xi, &yi)| yi - (intercept + slope * xi))
        .collect();
    let sse: f64 = residuals.iter().map(|r| r * r).sum();
    let df = nf - 2.0;
    let sigma = (sse / df).sqrt();
    let slope_err = sigma / sxx.sqrt();
    let intercept_err = sigma * (1.0 / nf + mean_x * mean_x / sxx).sqrt();

    let sst: f64 = y.iter().map(|&yi| (yi - mean_y).powi(2)).sum();
    let r2 = 1.0 - sse / sst;
    let adj_r2 = 1.0 - (1.0 - r2) * (nf - 1.0) / df;

    Some(OlsFit {
        intercept,
        intercept_err,
        slope,
        slope_err,
        n,
        r2,
        adj_r2,
        rmse: sigma,
        residuals,
    })
}

/// §7.7 edge diagnostics (R lines 960-976). `residuals` must already be in
/// x-sorted order; `edge_count` must already be `min(edge_points, n / 2)`.
fn edge_diagnostics(residuals: &[f64], sigma: f64, edge_count: usize) -> (f64, f64) {
    if edge_count == 0 {
        return (f64::NAN, f64::NAN);
    }
    let n = residuals.len();
    let edge_residuals: Vec<f64> = residuals[..edge_count]
        .iter()
        .chain(residuals[n - edge_count..].iter())
        .copied()
        .collect();
    let centre_residuals: &[f64] = &residuals[edge_count..n - edge_count];

    let edge_mae =
        edge_residuals.iter().map(|r| r.abs()).sum::<f64>() / edge_residuals.len() as f64;
    let centre_mae = if centre_residuals.is_empty() {
        f64::NAN
    } else {
        centre_residuals.iter().map(|r| r.abs()).sum::<f64>() / centre_residuals.len() as f64
    };
    let edge_mae_ratio = edge_mae / centre_mae;
    let edge_max_z = if sigma.is_finite() && sigma > 0.0 {
        edge_residuals
            .iter()
            .map(|r| (r / sigma).abs())
            .fold(f64::NEG_INFINITY, f64::max)
    } else {
        f64::NAN
    };
    (edge_mae_ratio, edge_max_z)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ols_fit_recovers_exact_line() {
        // E = 1.0 - 0.5*x exactly -- zero residuals, r2 = 1.
        let x = [0.0, 1.0, 2.0, 3.0];
        let y: Vec<f64> = x.iter().map(|&xi| 1.0 - 0.5 * xi).collect();
        let fit = ols_fit(&x, &y).unwrap();
        assert!((fit.intercept - 1.0).abs() < 1e-12);
        assert!((fit.slope - (-0.5)).abs() < 1e-12);
        assert!((fit.r2 - 1.0).abs() < 1e-9);
        assert!(fit.rmse < 1e-12);
        assert!(fit.intercept_err < 1e-12);
        assert!(fit.slope_err < 1e-12);
    }

    #[test]
    fn ols_fit_matches_known_noisy_case() {
        // Hand-computable: x=[0,1,2], y=[1,2,2] -> slope=0.5, intercept=1.1667.
        let x = [0.0, 1.0, 2.0];
        let y = [1.0, 2.0, 2.0];
        let fit = ols_fit(&x, &y).unwrap();
        // mean_x=1, mean_y=5/3, Sxx=2, Sxy=(−1)(1−5/3)+0+(1)(2−5/3)=(−1)(−2/3)+(1)(1/3)=2/3+1/3=1
        // slope = 1/2 = 0.5; intercept = 5/3 - 0.5*1 = 7/6
        assert!((fit.slope - 0.5).abs() < 1e-12);
        assert!((fit.intercept - 7.0 / 6.0).abs() < 1e-12);
    }

    #[test]
    fn ols_fit_degenerate_x_returns_none() {
        let x = [1.0, 1.0, 1.0];
        let y = [1.0, 2.0, 3.0];
        assert!(ols_fit(&x, &y).is_none());
    }

    #[test]
    fn ols_fit_fewer_than_three_points_returns_none() {
        let x = [0.0, 1.0];
        let y = [1.0, 2.0];
        assert!(ols_fit(&x, &y).is_none());
    }

    #[test]
    fn edge_diagnostics_zero_edge_points_yields_nan() {
        let residuals = [0.1, -0.1, 0.05, -0.05, 0.2];
        let (ratio, z) = edge_diagnostics(&residuals, 0.1, 0);
        assert!(ratio.is_nan());
        assert!(z.is_nan());
    }

    #[test]
    fn edge_diagnostics_basic() {
        // n=5, edge_count=1: edge = [resid[0], resid[4]], centre = resid[1..4]
        let residuals = [1.0, 0.1, 0.1, 0.1, 2.0];
        let sigma = 0.5;
        let (ratio, z) = edge_diagnostics(&residuals, sigma, 1);
        // edge_mae = (1.0+2.0)/2 = 1.5; centre_mae = 0.1
        assert!((ratio - 15.0).abs() < 1e-9);
        // edge_max_z = max(|1.0/0.5|, |2.0/0.5|) = 4.0
        assert!((z - 4.0).abs() < 1e-9);
    }

    #[test]
    fn fit_rest_window_matches_known_line_within_window() {
        // E0=3.0, s=-0.01 exactly, 10 points at step_t=0..9, window covers all.
        let step_t: Vec<f64> = (0..10).map(|i| i as f64).collect();
        let voltage: Vec<f64> = step_t.iter().map(|&t| 3.0 - 0.01 * t.sqrt()).collect();
        let fit = fit_rest_window(&step_t, &voltage, (0.0, 20.0), 1).unwrap();
        assert!((fit.e0 - 3.0).abs() < 1e-9);
        assert!((fit.s - (-0.01)).abs() < 1e-9);
        assert_eq!(fit.n_pts, 10);
    }

    #[test]
    fn fit_rest_window_excludes_points_outside_window() {
        let step_t: Vec<f64> = (0..10).map(|i| i as f64).collect();
        let voltage: Vec<f64> = step_t.iter().map(|&t| 3.0 - 0.01 * t.sqrt()).collect();
        // Window [2,5] should keep step_t in {2,3,4,5} -> 4 points.
        let fit = fit_rest_window(&step_t, &voltage, (2.0, 5.0), 1).unwrap();
        assert_eq!(fit.n_pts, 4);
    }

    #[test]
    fn fit_rest_window_too_few_points_reports_count() {
        let step_t = [0.0, 1.0, 20.0, 21.0];
        let voltage = [3.0, 2.9, 2.5, 2.4];
        // Window [0,1] only keeps step_t 0 and 1 -> 2 points, below the minimum of 3.
        let err = fit_rest_window(&step_t, &voltage, (0.0, 1.0), 1).unwrap_err();
        assert_eq!(err, RestFitError::TooFewPoints { n: 2 });
    }

    #[test]
    fn fit_rest_window_degenerate_reports_distinctly() {
        // All step_t identical -> zero variance in sqrt(step_t).
        let step_t = [2.0, 2.0, 2.0, 2.0];
        let voltage = [3.0, 3.01, 2.99, 3.0];
        let err = fit_rest_window(&step_t, &voltage, (0.0, 10.0), 1).unwrap_err();
        assert_eq!(err, RestFitError::Degenerate);
    }
}
