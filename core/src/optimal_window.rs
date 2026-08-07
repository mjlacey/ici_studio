//! §10.3 "Estimate optimal window" -- a spec-only feature (§14 item 7:
//! "Not present in R"). `ici_analysis.R` has no equivalent function, so
//! this module's doc comments and the spec prose are the sole
//! specification, not a port. Reuses [`crate::regress::fit_rest_window`]
//! directly -- this is "cheap once the regression path exists" (spec's own
//! words), just that function called many times over a grid of candidate
//! windows and a sample of rests.

use crate::regress::fit_rest_window;
use crate::segment::{SegmentedData, State};
use std::collections::{HashMap, HashSet};

/// One rest, ready to be repeatedly re-fit at different candidate windows:
/// `q` is the *preceding* active segment's anchored charge (§7.2's shared
/// `rest` id makes this a direct one-pass extract), `step_t`/`voltage` are
/// that rest's own points, exactly what `fit_rest_window` needs.
#[derive(Debug, Clone)]
pub struct RestCandidate {
    pub group_id: u32,
    pub rest_id: u32,
    pub state: State,
    pub q: f64,
    pub step_t: Vec<f64>,
    pub voltage: Vec<f64>,
}

/// Extracts one [`RestCandidate`] per rest that has both a preceding
/// active segment and rest-state points (every rest should, by
/// construction of `segment()`; skipped defensively otherwise).
pub fn rest_candidates(seg: &SegmentedData) -> Vec<RestCandidate> {
    type Key = (u32, u32);
    let n = seg.state.len();

    let mut order: Vec<Key> = Vec::new();
    let mut seen: HashSet<Key> = HashSet::new();
    let mut active_last_q: HashMap<Key, f64> = HashMap::new();
    let mut active_last_state: HashMap<Key, State> = HashMap::new();
    let mut rest_step_t: HashMap<Key, Vec<f64>> = HashMap::new();
    let mut rest_voltage: HashMap<Key, Vec<f64>> = HashMap::new();

    for i in 0..n {
        let key = (seg.group_id[i], seg.rest[i]);
        if seen.insert(key) {
            order.push(key);
        }
        if seg.state[i] == State::Rest {
            rest_step_t.entry(key).or_default().push(seg.step_t[i]);
            rest_voltage.entry(key).or_default().push(seg.voltage[i]);
        } else {
            // Overwritten on each occurrence -- ends up as the *last*
            // active row sharing this rest id, matching
            // `interruption_summary`'s own "last row" convention.
            active_last_q.insert(key, seg.charge[i]);
            active_last_state.insert(key, seg.state[i]);
        }
    }

    let mut out = Vec::with_capacity(order.len());
    for key in order {
        let (Some(&q), Some(&state), Some(step_t)) = (
            active_last_q.get(&key),
            active_last_state.get(&key),
            rest_step_t.get(&key),
        ) else {
            continue;
        };
        let voltage = rest_voltage.get(&key).cloned().unwrap_or_default();
        out.push(RestCandidate {
            group_id: key.0,
            rest_id: key.1,
            state,
            q,
            step_t: step_t.clone(),
            voltage,
        });
    }
    out
}

/// §10.3 "Sampling": returns indices into `candidates`. Splits `n`
/// proportionally between `Charge`/`Discharge` (min 1 each) when both are
/// present and `n >= 2`, sampling each state independently; otherwise
/// samples directly from all candidates (no stratification needed).
pub fn sample_rests(candidates: &[RestCandidate], n: usize) -> Vec<usize> {
    if candidates.is_empty() || n == 0 {
        return Vec::new();
    }

    let charge_idx: Vec<usize> = (0..candidates.len())
        .filter(|&i| candidates[i].state == State::Charge)
        .collect();
    let discharge_idx: Vec<usize> = (0..candidates.len())
        .filter(|&i| candidates[i].state == State::Discharge)
        .collect();

    if n >= 2 && !charge_idx.is_empty() && !discharge_idx.is_empty() {
        let total = (charge_idx.len() + discharge_idx.len()) as f64;
        let n_charge = (((n as f64) * (charge_idx.len() as f64) / total).round() as usize).clamp(1, n - 1);
        let n_discharge = n - n_charge;

        let mut result = bin_sample(candidates, &charge_idx, n_charge);
        result.extend(bin_sample(candidates, &discharge_idx, n_discharge));
        result
    } else {
        let all_idx: Vec<usize> = (0..candidates.len()).collect();
        bin_sample(candidates, &all_idx, n)
    }
}

/// Equal-`Q`-bin selection within `pool` (indices into `candidates`): bins
/// `[Q_min, Q_max]` into `n` bins, picks the unused pool member nearest
/// each bin centre, preferring a member actually inside that bin; falls
/// back to the globally-nearest unused pool member when the bin has none
/// of its own left (§10.3: "if a bin is empty, take the next-nearest
/// unused rest").
fn bin_sample(candidates: &[RestCandidate], pool: &[usize], n: usize) -> Vec<usize> {
    if pool.is_empty() || n == 0 {
        return Vec::new();
    }
    let n = n.min(pool.len());
    let q_min = pool.iter().map(|&i| candidates[i].q).fold(f64::INFINITY, f64::min);
    let q_max = pool
        .iter()
        .map(|&i| candidates[i].q)
        .fold(f64::NEG_INFINITY, f64::max);

    if n == 1 || (q_max - q_min).abs() < 1e-300 {
        // A single bin, or every candidate shares (nearly) the same Q --
        // bin centres would be degenerate; take the first n by index.
        return pool.iter().take(n).copied().collect();
    }

    let bin_width = (q_max - q_min) / (n as f64);
    let mut used: HashSet<usize> = HashSet::new();
    let mut result = Vec::with_capacity(n);

    let nearest = |centre: f64, only: &dyn Fn(usize) -> bool, used: &HashSet<usize>| -> Option<usize> {
        pool.iter()
            .copied()
            .filter(|&i| !used.contains(&i) && only(i))
            .min_by(|&a, &b| {
                (candidates[a].q - centre)
                    .abs()
                    .partial_cmp(&(candidates[b].q - centre).abs())
                    .unwrap()
            })
    };

    for b in 0..n {
        let centre = q_min + (b as f64 + 0.5) * bin_width;
        let bin_lo = q_min + (b as f64) * bin_width;
        let bin_hi = q_min + ((b + 1) as f64) * bin_width;

        let chosen = nearest(centre, &|i| candidates[i].q >= bin_lo && candidates[i].q <= bin_hi, &used)
            .or_else(|| nearest(centre, &|_| true, &used));

        if let Some(idx) = chosen {
            used.insert(idx);
            result.push(idx);
        }
    }
    result
}

/// `(T, heterogeneous)`: `T` is the plain max of per-rest max `step.t`
/// when all sampled rests are (nearly) the same length; otherwise the 5th
/// percentile, per §10.3 ("use the 5th percentile ... if rest lengths
/// vary"). `heterogeneous` is true exactly when those two values differ,
/// so the UI can show the "lengths vary" note precisely when it applies.
pub fn observed_t_max(sample: &[RestCandidate]) -> (f64, bool) {
    let mut per_rest_max: Vec<f64> = sample
        .iter()
        .map(|c| c.step_t.iter().cloned().fold(f64::NEG_INFINITY, f64::max))
        .filter(|v| v.is_finite())
        .collect();
    if per_rest_max.is_empty() {
        return (0.0, false);
    }
    per_rest_max.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let true_max = *per_rest_max.last().unwrap();
    let p5 = percentile(&per_rest_max, 0.05);
    let heterogeneous = (true_max - p5).abs() > 1e-9;
    (if heterogeneous { p5 } else { true_max }, heterogeneous)
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return f64::NAN;
    }
    let pos = (sorted.len() - 1) as f64 * p;
    let base = pos.floor() as usize;
    let frac = pos - base as f64;
    if base + 1 < sorted.len() {
        sorted[base] + frac * (sorted[base + 1] - sorted[base])
    } else {
        sorted[base]
    }
}

#[derive(Debug, Clone, Copy)]
pub struct GridConfig {
    pub t_min_lower_bound: f64,
    pub l_min: f64,
}

/// Integer-second candidate grid exactly per §10.3's
/// `t_min ∈ {1,2,…,floor(T)−L_min}`, `t_max ∈ {t_min+L_min,…,floor(T)}`
/// notation (integer loop, not float accumulation, to avoid drift).
pub fn candidate_grid(floor_t_max: i64, config: &GridConfig) -> Vec<(f64, f64)> {
    let t_min_lb = config.t_min_lower_bound.round() as i64;
    let l_min = config.l_min.round() as i64;
    let mut grid = Vec::new();
    if l_min <= 0 {
        return grid;
    }
    let max_t_min = floor_t_max - l_min;
    let mut t_min = t_min_lb;
    while t_min <= max_t_min {
        let mut t_max = t_min + l_min;
        while t_max <= floor_t_max {
            grid.push((t_min as f64, t_max as f64));
            t_max += 1;
        }
        t_min += 1;
    }
    grid
}

/// One candidate window's score. `rejected` candidates are still reported
/// (for the heatmap) rather than dropped, per §10.3.
#[derive(Debug, Clone, Copy)]
pub struct CandidateScore {
    pub t_min: f64,
    pub t_max: f64,
    pub mean_adj_r2: f64,
    pub median_adj_r2: f64,
    pub n_valid: usize,
    pub n_sampled: usize,
    pub median_n_pts: f64,
    pub median_edge_max_z: f64,
    pub rejected: bool,
}

/// §10.3: "discard rests with fewer than max(4, 3) points" = 4.
/// `fit_rest_window`'s own floor is 3 -- this is a stricter post-filter on
/// top of it, not a change to that function (§10.1's live preview still
/// wants the looser 3-point floor).
const MIN_VALID_POINTS: usize = 4;
/// §10.3: "Reject a candidate outright if fewer than 80% of the sampled
/// rests fitted."
const MIN_FIT_FRACTION: f64 = 0.8;

/// Fits every sampled rest at `window`, aggregates. Mirrors `fit_rest_window`'s
/// own tolerance for a handful of individually-failing rests -- only the
/// aggregate 80% threshold gates rejection.
pub fn score_candidate(sample: &[RestCandidate], window: (f64, f64), edge_points: usize) -> CandidateScore {
    let mut adj_r2s = Vec::new();
    let mut n_pts_list = Vec::new();
    let mut edge_max_zs = Vec::new();

    for c in sample {
        if let Ok(fit) = fit_rest_window(&c.step_t, &c.voltage, window, edge_points) {
            if fit.n_pts >= MIN_VALID_POINTS {
                adj_r2s.push(fit.adj_r2);
                n_pts_list.push(fit.n_pts as f64);
                edge_max_zs.push(fit.edge_max_z);
            }
        }
    }

    let n_valid = adj_r2s.len();
    let n_sampled = sample.len();
    let rejected = n_sampled == 0 || (n_valid as f64) < MIN_FIT_FRACTION * (n_sampled as f64);

    CandidateScore {
        t_min: window.0,
        t_max: window.1,
        mean_adj_r2: mean(&adj_r2s),
        median_adj_r2: median(&adj_r2s),
        n_valid,
        n_sampled,
        median_n_pts: median(&n_pts_list),
        median_edge_max_z: median(&edge_max_zs),
        rejected,
    }
}

/// Non-finite inputs are dropped before aggregating -- `edge_max_z` in
/// particular is legitimately NA (§7.7: "NA if sigma is not finite and
/// positive") on an exact zero-residual fit, and one such rest shouldn't
/// corrupt the whole sample's median/mean.
fn mean(values: &[f64]) -> f64 {
    let finite: Vec<f64> = values.iter().copied().filter(|v| v.is_finite()).collect();
    if finite.is_empty() {
        return f64::NAN;
    }
    finite.iter().sum::<f64>() / finite.len() as f64
}

fn median(values: &[f64]) -> f64 {
    let mut sorted: Vec<f64> = values.iter().copied().filter(|v| v.is_finite()).collect();
    if sorted.is_empty() {
        return f64::NAN;
    }
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mid = sorted.len() / 2;
    if sorted.len().is_multiple_of(2) {
        (sorted[mid - 1] + sorted[mid]) / 2.0
    } else {
        sorted[mid]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::segment::{segment, SegmentConfig};

    fn linear_candidate(rest_id: u32, state: State, q: f64, step_t: Vec<f64>, e0: f64, slope: f64) -> RestCandidate {
        let voltage = step_t.iter().map(|&t| e0 + slope * t.sqrt()).collect();
        RestCandidate {
            group_id: 0,
            rest_id,
            state,
            q,
            step_t,
            voltage,
        }
    }

    #[test]
    fn rest_candidates_extracts_q_from_preceding_active_segment() {
        // R R charge charge charge R R discharge discharge discharge R R,
        // single group/cycle -- same shape as segment.rs's own Q-anchoring
        // fixture, reused here to check rest_candidates' extraction.
        let current = [0.0, 0.0, 1.0, 1.0, 1.0, 0.0, 0.0, -1.0, -1.0, -1.0, 0.0, 0.0];
        let n = current.len();
        let group_id = vec![0u32; n];
        let t: Vec<f64> = (0..n).map(|i| i as f64).collect();
        let cyc_n = vec![1.0; n];
        let voltage = vec![0.0; n];
        let charge = vec![0.0, 0.0, 1.0, 2.0, 3.0, 3.0, 3.0, 2.0, 1.0, 0.0, 0.0, 0.0];
        let config = SegmentConfig {
            state_threshold: 0.0,
            drop_unrested_reversals: false,
        };
        let (seg, _log) = segment(&group_id, &t, &cyc_n, &current, &voltage, &charge, &config);

        let candidates = rest_candidates(&seg);
        // rest 1 (leading R R) has no preceding active segment -- skipped.
        // rest 2 follows the charge run. `seg.charge` is already §6
        // start-anchored *within (group, cyc.n, state)* -- the charge-state
        // rows (raw 1,2,3) anchor to their own first value, giving 0,1,2 --
        // matching segment.rs's own `reanchor_charge` fixture.
        let rest2 = candidates.iter().find(|c| c.rest_id == 2).expect("rest 2");
        assert_eq!(rest2.state, State::Charge);
        assert!((rest2.q - 2.0).abs() < 1e-12, "q={}", rest2.q);
        assert_eq!(rest2.step_t.len(), 2); // rows 5,6

        // rest 3 follows the discharge run (raw 2,1,0 -> anchored 0,-1,-2).
        let rest3 = candidates.iter().find(|c| c.rest_id == 3).expect("rest 3");
        assert_eq!(rest3.state, State::Discharge);
        assert!((rest3.q - (-2.0)).abs() < 1e-12, "q={}", rest3.q);
        assert_eq!(rest3.step_t.len(), 2); // rows 10,11
    }

    #[test]
    fn sample_rests_single_state_skips_stratification_and_spreads_across_q() {
        let candidates: Vec<RestCandidate> = (0..10)
            .map(|i| linear_candidate(i, State::Discharge, i as f64, vec![1.0, 4.0, 9.0], 1.0, -0.1))
            .collect();
        let sample = sample_rests(&candidates, 4);
        assert_eq!(sample.len(), 4);
        let mut qs: Vec<f64> = sample.iter().map(|&i| candidates[i].q).collect();
        qs.sort_by(|a, b| a.partial_cmp(b).unwrap());
        // Spread across the range, not clustered at one end.
        assert!(qs[0] <= 3.0, "should include a low-Q rest: {qs:?}");
        assert!(qs[3] >= 6.0, "should include a high-Q rest: {qs:?}");
    }

    #[test]
    fn sample_rests_mixed_state_stratifies_proportionally() {
        // 8 charge, 2 discharge -- n=5 should give roughly a 4/1 split.
        let mut candidates: Vec<RestCandidate> = (0..8)
            .map(|i| linear_candidate(i, State::Charge, i as f64, vec![1.0, 4.0, 9.0], 1.0, -0.1))
            .collect();
        candidates.extend((8..10).map(|i| linear_candidate(i, State::Discharge, i as f64, vec![1.0, 4.0, 9.0], 1.0, -0.1)));

        let sample = sample_rests(&candidates, 5);
        assert_eq!(sample.len(), 5);
        let n_charge = sample.iter().filter(|&&i| candidates[i].state == State::Charge).count();
        let n_discharge = sample.iter().filter(|&&i| candidates[i].state == State::Discharge).count();
        assert_eq!(n_charge + n_discharge, 5);
        assert!(n_discharge >= 1, "discharge must not be starved to zero: {n_discharge}");
        assert!(n_charge >= n_discharge, "charge (8/10 of the pool) should get the larger share");
    }

    #[test]
    fn bin_sample_falls_back_to_nearest_unused_when_a_bin_is_empty() {
        // Q values clustered at 0 and 100 -- with n=3 equal bins over
        // [0,100], the middle bin [33.3,66.7] is empty; its centre (50)
        // should fall back to the nearest *unused* candidate (100, since
        // 0 gets claimed by the first bin).
        let candidates = vec![
            linear_candidate(0, State::Discharge, 0.0, vec![1.0, 4.0, 9.0], 1.0, -0.1),
            linear_candidate(1, State::Discharge, 100.0, vec![1.0, 4.0, 9.0], 1.0, -0.1),
        ];
        let sample = sample_rests(&candidates, 3);
        // Only 2 distinct candidates exist -- both get used exactly once,
        // even though 3 bins were requested.
        assert_eq!(sample.len(), 2);
        let mut used: Vec<usize> = sample.clone();
        used.sort_unstable();
        assert_eq!(used, vec![0, 1]);
    }

    #[test]
    fn candidate_grid_matches_spec_worked_example() {
        // §10.3: "T ≈ 9.95 s, so floor(T) = 9 ... a grid of ~10 candidates".
        let grid = candidate_grid(9, &GridConfig { t_min_lower_bound: 1.0, l_min: 5.0 });
        assert_eq!(grid.len(), 10);
        assert_eq!(grid[0], (1.0, 6.0));
        assert_eq!(grid[grid.len() - 1], (4.0, 9.0));
        for &(t_min, t_max) in &grid {
            assert!(t_max - t_min >= 5.0);
            assert!(t_min >= 1.0);
            assert!(t_max <= 9.0);
        }
    }

    #[test]
    fn candidate_grid_empty_when_range_too_narrow() {
        let grid = candidate_grid(3, &GridConfig { t_min_lower_bound: 1.0, l_min: 5.0 });
        assert!(grid.is_empty());
    }

    #[test]
    fn score_candidate_computes_mean_and_median_adj_r2() {
        // Three noise-free linear rests (E = e0 + slope*sqrt(t)) -> each
        // fits with adj_r2 == 1.0 exactly.
        let step_t = vec![1.0, 4.0, 9.0, 16.0, 25.0];
        let sample = vec![
            linear_candidate(1, State::Discharge, 0.0, step_t.clone(), 1.0, -0.1),
            linear_candidate(2, State::Discharge, 1.0, step_t.clone(), 2.0, -0.2),
            linear_candidate(3, State::Discharge, 2.0, step_t, 3.0, -0.3),
        ];
        let score = score_candidate(&sample, (0.0, 30.0), 1);
        assert_eq!(score.n_sampled, 3);
        assert_eq!(score.n_valid, 3);
        assert!(!score.rejected);
        assert!((score.mean_adj_r2 - 1.0).abs() < 1e-9, "{}", score.mean_adj_r2);
        assert!((score.median_adj_r2 - 1.0).abs() < 1e-9);
    }

    #[test]
    fn score_candidate_discards_fits_with_fewer_than_four_points() {
        let step_t_short = vec![1.0, 4.0, 9.0]; // exactly 3 pts -- fits, but discarded by the >=4 post-filter
        let step_t_long = vec![1.0, 4.0, 9.0, 16.0, 25.0]; // 5 pts -- kept
        let sample = vec![
            linear_candidate(1, State::Discharge, 0.0, step_t_short, 1.0, -0.1),
            linear_candidate(2, State::Discharge, 1.0, step_t_long, 2.0, -0.2),
        ];
        let score = score_candidate(&sample, (0.0, 30.0), 1);
        assert_eq!(score.n_sampled, 2);
        assert_eq!(score.n_valid, 1);
        assert!((score.mean_adj_r2 - 1.0).abs() < 1e-9);
    }

    #[test]
    fn score_candidate_rejects_below_80_percent_fit_rate() {
        let step_t_ok = vec![1.0, 4.0, 9.0, 16.0, 25.0];
        let step_t_too_few = vec![1.0]; // fails fit_rest_window outright (<3 pts)
        let sample = vec![
            linear_candidate(1, State::Discharge, 0.0, step_t_ok.clone(), 1.0, -0.1),
            linear_candidate(2, State::Discharge, 1.0, step_t_ok.clone(), 2.0, -0.2),
            linear_candidate(3, State::Discharge, 2.0, step_t_ok, 3.0, -0.3),
            linear_candidate(4, State::Discharge, 3.0, step_t_too_few.clone(), 4.0, -0.4),
            linear_candidate(5, State::Discharge, 4.0, step_t_too_few, 5.0, -0.5),
        ];
        // 3/5 = 60% < 80% -> rejected, even though the 3 that did fit are perfect.
        let score = score_candidate(&sample, (0.0, 30.0), 1);
        assert_eq!(score.n_sampled, 5);
        assert_eq!(score.n_valid, 3);
        assert!(score.rejected);
    }
}
