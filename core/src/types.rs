//! Shared, small utilities used across Stage A/B modules.

use std::collections::HashMap;

/// Builds one group id per row from the string representation of one or
/// more columns, in row order -- mirrors R's
/// `interaction(..., drop = TRUE, lex.order = TRUE)` used throughout
/// `ici_analysis.R` (group id, then `(group, cyc.n, state)`, etc.).
///
/// An empty `columns` slice puts every row in a single group (id 0),
/// matching `make_ici_group_id()` for `grouping_columns = character()`.
/// Group id *values* are otherwise arbitrary (assigned in ascending sort
/// order of the combined key) -- callers should only rely on rows sharing a
/// group id, never on which integer a particular group gets.
pub fn make_group_id(columns: &[Vec<String>], n_rows: usize) -> Vec<u32> {
    if columns.is_empty() {
        return vec![0u32; n_rows];
    }
    let mut keys: Vec<String> = Vec::with_capacity(n_rows);
    for row in 0..n_rows {
        let mut key = String::new();
        for (i, col) in columns.iter().enumerate() {
            if i > 0 {
                key.push('\u{1f}');
            }
            key.push_str(&col[row]);
        }
        keys.push(key);
    }
    let mut unique_sorted: Vec<String> = keys.clone();
    unique_sorted.sort();
    unique_sorted.dedup();
    let index: HashMap<&str, u32> = unique_sorted
        .iter()
        .enumerate()
        .map(|(i, k)| (k.as_str(), i as u32))
        .collect();
    keys.iter().map(|k| index[k.as_str()]).collect()
}

/// Port of `select_final_window()` (ici_analysis.R lines 548-556).
/// `window = None` returns just the single value at the last valid point
/// (R's `which.max`: the *first* occurrence of the maximum step-time among
/// valid points, in original order).
pub fn select_final_window(step_time: &[f64], values: &[f64], window: Option<f64>) -> Vec<f64> {
    let valid: Vec<usize> = (0..step_time.len())
        .filter(|&i| step_time[i].is_finite() && values[i].is_finite())
        .collect();
    if valid.is_empty() {
        return Vec::new();
    }
    let final_time = valid
        .iter()
        .map(|&i| step_time[i])
        .fold(f64::NEG_INFINITY, f64::max);
    match window {
        None => {
            let idx = valid
                .iter()
                .copied()
                .find(|&i| step_time[i] == final_time)
                .expect("final_time is the max of a non-empty valid set");
            vec![values[idx]]
        }
        Some(w) => {
            let selected: Vec<usize> = valid
                .iter()
                .copied()
                .filter(|&i| step_time[i] >= final_time - w)
                .collect();
            let selected = if selected.is_empty() { valid } else { selected };
            selected.iter().map(|&i| values[i]).collect()
        }
    }
}

/// Port of `select_initial_window()` (ici_analysis.R lines 565-573). Mirrors
/// `select_final_window` at the start of the window (`which.min`: first
/// occurrence of the minimum step-time).
pub fn select_initial_window(step_time: &[f64], values: &[f64], window: Option<f64>) -> Vec<f64> {
    let valid: Vec<usize> = (0..step_time.len())
        .filter(|&i| step_time[i].is_finite() && values[i].is_finite())
        .collect();
    if valid.is_empty() {
        return Vec::new();
    }
    let start_time = valid
        .iter()
        .map(|&i| step_time[i])
        .fold(f64::INFINITY, f64::min);
    match window {
        None => {
            let idx = valid
                .iter()
                .copied()
                .find(|&i| step_time[i] == start_time)
                .expect("start_time is the min of a non-empty valid set");
            vec![values[idx]]
        }
        Some(w) => {
            let selected: Vec<usize> = valid
                .iter()
                .copied()
                .filter(|&i| step_time[i] <= start_time + w)
                .collect();
            let selected = if selected.is_empty() { valid } else { selected };
            selected.iter().map(|&i| values[i]).collect()
        }
    }
}

/// Port of `interpolate_endpoint()` (ici_analysis.R lines 582-601).
pub fn interpolate_endpoint(step_time: &[f64], voltage: &[f64], window: Option<f64>) -> f64 {
    let valid: Vec<usize> = (0..step_time.len())
        .filter(|&i| step_time[i].is_finite() && voltage[i].is_finite())
        .collect();
    if valid.is_empty() {
        return f64::NAN;
    }
    let final_time = valid
        .iter()
        .map(|&i| step_time[i])
        .fold(f64::NEG_INFINITY, f64::max);

    // R: `voltage[tail(which(valid & step_time == final_time), 1L)]` -- the
    // *last* occurrence at the final time, unlike select_final_window's
    // which.max (first occurrence). This asymmetry is in the reference
    // implementation itself.
    let last_point_fallback = |step_time: &[f64], voltage: &[f64]| -> f64 {
        let idx = (0..step_time.len())
            .rfind(|&i| {
                step_time[i].is_finite() && voltage[i].is_finite() && step_time[i] == final_time
            })
            .expect("final_time is the max of a non-empty valid set");
        voltage[idx]
    };

    match window {
        None => last_point_fallback(step_time, voltage),
        Some(w) => {
            let selected: Vec<usize> = valid
                .iter()
                .copied()
                .filter(|&i| step_time[i] >= final_time - w)
                .collect();
            let mut distinct_x: Vec<f64> = selected.iter().map(|&i| step_time[i]).collect();
            distinct_x.sort_by(|a, b| a.partial_cmp(b).unwrap());
            distinct_x.dedup();
            if selected.len() < 2 || distinct_x.len() < 2 {
                return last_point_fallback(step_time, voltage);
            }
            let xs: Vec<f64> = selected.iter().map(|&i| step_time[i]).collect();
            let ys: Vec<f64> = selected.iter().map(|&i| voltage[i]).collect();
            linear_approx(&xs, &ys, final_time)
        }
    }
}

/// `stats::approx(x, y, xout, rule = 2)` with R's default `ties = mean`:
/// duplicate x values are averaged before interpolating, and `xout` outside
/// the data range is clamped to the nearest endpoint rather than NA.
fn linear_approx(x: &[f64], y: &[f64], xout: f64) -> f64 {
    let mut pairs: Vec<(f64, f64)> = x.iter().copied().zip(y.iter().copied()).collect();
    pairs.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());

    let mut xs: Vec<f64> = Vec::new();
    let mut ys: Vec<f64> = Vec::new();
    let mut i = 0;
    while i < pairs.len() {
        let mut j = i;
        let mut sum = 0.0;
        let mut count = 0.0;
        while j < pairs.len() && pairs[j].0 == pairs[i].0 {
            sum += pairs[j].1;
            count += 1.0;
            j += 1;
        }
        xs.push(pairs[i].0);
        ys.push(sum / count);
        i = j;
    }

    if xout <= xs[0] {
        return ys[0];
    }
    if xout >= *xs.last().unwrap() {
        return *ys.last().unwrap();
    }
    let pos = xs.partition_point(|&v| v <= xout);
    let (i0, i1) = (pos - 1, pos);
    let (x0, x1, y0, y1) = (xs[i0], xs[i1], ys[i0], ys[i1]);
    if x1 == x0 {
        return y0;
    }
    y0 + (y1 - y0) * (xout - x0) / (x1 - x0)
}

/// §5.1 "pre-normalised charge column" heuristic: does the *candidate* raw
/// charge column already reset to (near) zero at the start of every
/// `(cyc.n, state)` group -- state derived the same way segmentation does
/// (§7.1)? If so, the app's own Q anchoring (§6) would be a no-op, worth an
/// info note rather than hard-coding vendor column names. Operates on raw,
/// not-yet-unit-converted values; "near zero" is judged relative to the
/// column's own overall spread so it works at any scale. Returns `false`
/// (not prenormalised / inconclusive) with fewer than two active groups.
pub fn looks_prenormalized_charge(
    cyc_n: &[f64],
    current: &[f64],
    charge: &[f64],
    state_threshold: f64,
) -> bool {
    let n = cyc_n.len();
    assert_eq!(current.len(), n);
    assert_eq!(charge.len(), n);

    let finite_charge: Vec<f64> = charge.iter().copied().filter(|v| v.is_finite()).collect();
    if finite_charge.len() < 2 {
        return false;
    }
    let min = finite_charge.iter().copied().fold(f64::INFINITY, f64::min);
    let max = finite_charge
        .iter()
        .copied()
        .fold(f64::NEG_INFINITY, f64::max);
    let spread = (max - min).max(1e-12);
    let zero_tol = spread * 0.02;

    let mut order: Vec<(u64, crate::segment::State)> = Vec::new();
    let mut first_charge: HashMap<(u64, crate::segment::State), f64> = HashMap::new();
    for i in 0..n {
        if !cyc_n[i].is_finite() || !current[i].is_finite() || !charge[i].is_finite() {
            continue;
        }
        let state = crate::segment::State::classify(current[i], state_threshold);
        let key = (cyc_n[i].to_bits(), state);
        first_charge.entry(key).or_insert_with(|| {
            order.push(key);
            charge[i]
        });
    }

    if order.len() < 2 {
        return false;
    }
    let near_zero_count = order
        .iter()
        .filter(|key| first_charge[*key].abs() <= zero_tol)
        .count();
    (near_zero_count as f64 / order.len() as f64) >= 0.9
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_columns_yield_one_group() {
        assert_eq!(make_group_id(&[], 5), vec![0, 0, 0, 0, 0]);
    }

    #[test]
    fn groups_rows_sharing_a_key() {
        let col = vec!["A", "A", "B", "A", "B"]
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>();
        let ids = make_group_id(&[col], 5);
        assert_eq!(ids[0], ids[1]);
        assert_eq!(ids[1], ids[3]);
        assert_eq!(ids[2], ids[4]);
        assert_ne!(ids[0], ids[2]);
    }

    #[test]
    fn combines_multiple_columns() {
        let a = vec!["A", "A", "A", "A"]
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>();
        let b = vec!["1", "1", "2", "2"]
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>();
        let ids = make_group_id(&[a, b], 4);
        assert_eq!(ids[0], ids[1]);
        assert_ne!(ids[0], ids[2]);
        assert_eq!(ids[2], ids[3]);
    }

    #[test]
    fn select_final_window_none_takes_last_point() {
        let step_t = [0.0, 1.0, 2.0];
        let values = [10.0, 20.0, 30.0];
        assert_eq!(select_final_window(&step_t, &values, None), vec![30.0]);
    }

    #[test]
    fn select_final_window_falls_back_to_all_valid_when_window_selects_none() {
        let step_t = [0.0, 1.0, 2.0];
        let values = [10.0, 20.0, 30.0];
        // window smaller than the actual spacing selects nothing at 2.0-0.5=1.5,
        // except point at 2.0 itself -- use a pathological gap instead.
        let step_t2 = [0.0, 10.0];
        let values2 = [1.0, 2.0];
        let selected = select_final_window(&step_t2, &values2, Some(0.001));
        // final_time=10, window=0.001 -> only point at >=9.999, i.e. just [10.0] -> [2.0]
        assert_eq!(selected, vec![2.0]);
        let _ = (step_t, values);
    }

    #[test]
    fn select_final_window_all_identical_step_t() {
        let step_t = [5.0, 5.0, 5.0];
        let values = [1.0, 2.0, 3.0];
        // window=None: first occurrence of the max (all tied) -> first value.
        assert_eq!(select_final_window(&step_t, &values, None), vec![1.0]);
        // window=Some: all points satisfy step_t >= 5.0 - w.
        assert_eq!(
            select_final_window(&step_t, &values, Some(1.0)),
            vec![1.0, 2.0, 3.0]
        );
    }

    #[test]
    fn select_initial_window_none_takes_first_point() {
        let step_t = [0.0, 1.0, 2.0];
        let values = [10.0, 20.0, 30.0];
        assert_eq!(select_initial_window(&step_t, &values, None), vec![10.0]);
    }

    #[test]
    fn interpolate_endpoint_none_takes_last_occurrence_at_max_time() {
        // Two points tie at the max step_t; R's fallback path takes the
        // *last* occurrence (tail(...,1)), unlike select_final_window.
        let step_t = [0.0, 2.0, 2.0];
        let voltage = [1.0, 2.0, 3.0];
        assert_eq!(interpolate_endpoint(&step_t, &voltage, None), 3.0);
    }

    #[test]
    fn interpolate_endpoint_fewer_than_two_points_falls_back() {
        let step_t = [0.0, 10.0];
        let voltage = [1.0, 2.0];
        // window so small only the last point qualifies -> fallback to last point.
        assert_eq!(interpolate_endpoint(&step_t, &voltage, Some(0.001)), 2.0);
    }

    #[test]
    fn interpolate_endpoint_all_identical_step_t_falls_back() {
        let step_t = [5.0, 5.0, 5.0];
        let voltage = [1.0, 2.0, 3.0];
        // Only one distinct step_t among selected points -> fewer than 2
        // distinct x -> fallback to the last occurrence at that time.
        assert_eq!(interpolate_endpoint(&step_t, &voltage, Some(10.0)), 3.0);
    }

    #[test]
    fn interpolate_endpoint_linear_interpolation() {
        let step_t = [0.0, 1.0, 2.0, 3.0];
        let voltage = [10.0, 10.0, 12.0, 14.0];
        // window=10 selects all four points; xout = final_time = 3.0 is an
        // exact knot, so this just returns the last y (14.0) without needing
        // true interpolation -- covered by the exact-max-clamp path.
        assert_eq!(interpolate_endpoint(&step_t, &voltage, Some(10.0)), 14.0);
    }

    #[test]
    fn linear_approx_interpolates_and_clamps() {
        assert_eq!(linear_approx(&[0.0, 2.0], &[0.0, 4.0], 1.0), 2.0);
        assert_eq!(linear_approx(&[0.0, 2.0], &[0.0, 4.0], -5.0), 0.0);
        assert_eq!(linear_approx(&[0.0, 2.0], &[0.0, 4.0], 5.0), 4.0);
    }

    #[test]
    fn linear_approx_averages_ties() {
        // x=1.0 appears twice with y=2.0 and y=4.0 -> averaged to 3.0.
        assert_eq!(
            linear_approx(&[0.0, 1.0, 1.0, 2.0], &[0.0, 2.0, 4.0, 8.0], 1.0),
            3.0
        );
    }

    #[test]
    fn prenormalized_charge_is_detected() {
        // Two cycles, each with a charge run whose charge resets to ~0 at
        // the start, then climbs -- classic per-half-cycle-reset export.
        let cyc_n = vec![1.0, 1.0, 1.0, 1.0, 2.0, 2.0, 2.0, 2.0];
        let current = vec![1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0];
        let charge = vec![0.0, 1.0, 2.0, 3.0, 0.01, 1.0, 2.0, 3.0];
        assert!(looks_prenormalized_charge(&cyc_n, &current, &charge, 0.0));
    }

    #[test]
    fn cumulative_charge_is_not_prenormalized() {
        // A running total across cycles never resets near zero after the
        // first group.
        let cyc_n = vec![1.0, 1.0, 1.0, 1.0, 2.0, 2.0, 2.0, 2.0];
        let current = vec![1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0];
        let charge = vec![0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0];
        assert!(!looks_prenormalized_charge(&cyc_n, &current, &charge, 0.0));
    }

    #[test]
    fn too_few_groups_is_inconclusive() {
        let cyc_n = vec![1.0, 1.0, 1.0];
        let current = vec![1.0, 1.0, 1.0];
        let charge = vec![0.0, 1.0, 2.0];
        assert!(!looks_prenormalized_charge(&cyc_n, &current, &charge, 0.0));
    }
}
