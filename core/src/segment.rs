//! State classification, rest indexing, step time, Q anchoring, and the two
//! row-drop passes. Faithful port of `ici_analysis.R` lines 699-855
//! (§7.1-7.5 of ICI_WEB_SPEC.md). Regression/summary/smoothing are later
//! modules -- this one only produces what R calls `segmented`.

use std::collections::HashMap;
use std::hash::Hash;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum State {
    Rest,
    Charge,
    Discharge,
}

impl State {
    pub fn as_str(self) -> &'static str {
        match self {
            State::Rest => "R",
            State::Charge => "charge",
            State::Discharge => "discharge",
        }
    }

    /// §7.1: `state = "R" if |I| <= threshold, "charge" if I > 0, else "discharge"`.
    pub(crate) fn classify(current: f64, threshold: f64) -> State {
        if current.abs() <= threshold {
            State::Rest
        } else if current > 0.0 {
            State::Charge
        } else {
            State::Discharge
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct SegmentConfig {
    pub state_threshold: f64,
    pub drop_unrested_reversals: bool,
}

impl Default for SegmentConfig {
    fn default() -> Self {
        Self {
            state_threshold: 0.0,
            drop_unrested_reversals: true,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct SegmentLog {
    pub reversal_rows_dropped: usize,
    pub incomplete_final_rows_dropped: usize,
    /// §7.2: a Rest run at the very start of a group's data, before any
    /// active run has occurred -- not a genuine interruption (there's no
    /// active run for it to interrupt), just idle time before cycling
    /// started. Dropped and counted here rather than left to surface
    /// downstream as a mysterious "0 usable points" regression failure.
    pub leading_rest_rows_dropped: usize,
}

/// One row per *retained* sample, in original relative order. Mirrors R's
/// `segmented` data frame: the original columns are carried through
/// unchanged alongside the newly-computed ones, since later stages (the
/// interruption summary, the per-rest regression) need the raw `t`/`I`/`E`
/// values, not just the classification.
#[derive(Debug, Clone)]
pub struct SegmentedData {
    /// Index into the original input arrays this row came from.
    pub row_index: Vec<usize>,
    pub group_id: Vec<u32>,
    pub cyc_n: Vec<f64>,
    pub state: Vec<State>,
    pub rest: Vec<u32>,
    pub t: Vec<f64>,
    pub step_t: Vec<f64>,
    pub voltage: Vec<f64>,
    pub current: Vec<f64>,
    /// The mapped charge column, anchored per §6 (start of each
    /// `(group, cyc.n, state)` run -- R's only behaviour; the per-state
    /// start/end choice from §6 is a presentation-layer config applied by
    /// the caller, not this function).
    pub charge: Vec<f64>,
}

/// Runs §7.1-7.5 of ICI_WEB_SPEC.md (ici_analysis.R lines 699-855).
///
/// `group_id` should come from [`crate::types::make_group_id`] over
/// whatever grouping columns are configured (or all-zero for none).
/// All slices must have equal length; rows are assumed already sorted by
/// time within each group, matching R's `ave()`/`split()` behaviour.
#[allow(clippy::too_many_arguments)]
pub fn segment(
    group_id: &[u32],
    t: &[f64],
    cyc_n: &[f64],
    current: &[f64],
    voltage: &[f64],
    charge: &[f64],
    config: &SegmentConfig,
) -> (SegmentedData, SegmentLog) {
    let n = t.len();
    assert_eq!(group_id.len(), n);
    assert_eq!(cyc_n.len(), n);
    assert_eq!(current.len(), n);
    assert_eq!(voltage.len(), n);
    assert_eq!(charge.len(), n);

    let state: Vec<State> = current
        .iter()
        .map(|&i| State::classify(i, config.state_threshold))
        .collect();

    let groups = group_indices(group_id);

    // §7.2 rest indexing, per group.
    let mut rest = vec![0u32; n];
    for idx in &groups {
        let mut r = 1u32;
        for (k, &row) in idx.iter().enumerate() {
            if k > 0 {
                let prev_row = idx[k - 1];
                if state[prev_row] == State::Rest && state[row] != State::Rest {
                    r += 1;
                }
            }
            rest[row] = r;
        }
    }

    // §7.3 step.t = t - t[first] within (group, rest, state).
    let mut step_t = vec![0.0f64; n];
    {
        let keys: Vec<(u32, State)> = (0..n).map(|i| (rest[i], state[i])).collect();
        assign_within_group(&groups, &keys, t, &mut step_t);
    }

    // §6 Q anchoring (R's start/start default) = charge - charge[first]
    // within (group, cyc.n, state).
    let mut anchored_charge = vec![0.0f64; n];
    {
        let cyc_bits: Vec<u64> = cyc_n.iter().map(|&c| c.to_bits()).collect();
        let keys: Vec<(u64, State)> = (0..n).map(|i| (cyc_bits[i], state[i])).collect();
        assign_within_group(&groups, &keys, charge, &mut anchored_charge);
    }

    let mut keep = vec![true; n];

    // §7.2 spurious leading rest: if a group's data starts with `Rest`
    // (before any active run has occurred), that leading rest run has no
    // active companion to derive Q from -- it's idle time before cycling
    // started, not a genuine on-then-off interruption. Drop it (all rows
    // sharing its rest id, which by construction of the indexing above is
    // exactly the leading Rest-only run).
    let mut leading_rest_rows_dropped = 0usize;
    for idx in &groups {
        if let Some(&first_row) = idx.first() {
            if state[first_row] == State::Rest {
                let leading_rest_id = rest[first_row];
                for &row in idx {
                    if rest[row] == leading_rest_id {
                        keep[row] = false;
                        leading_rest_rows_dropped += 1;
                    }
                }
            }
        }
    }

    // §7.4 drop unrested reversals, per group.
    let mut reversal_rows_dropped = 0usize;
    if config.drop_unrested_reversals {
        for idx in &groups {
            let group_state: Vec<State> = idx.iter().map(|&row| state[row]).collect();
            let remove = find_unrested_reversal_rows(&group_state);
            for (k, &row) in idx.iter().enumerate() {
                if remove[k] {
                    keep[row] = false;
                    reversal_rows_dropped += 1;
                }
            }
        }
    }

    // §7.5 drop the incomplete final active step, per group, evaluated on
    // the *already reversal-filtered* rows (matching R's row order: the
    // filter is applied before this pass re-splits by group).
    let groups_after_reversal: Vec<Vec<usize>> = groups
        .iter()
        .map(|idx| idx.iter().copied().filter(|&row| keep[row]).collect())
        .collect();
    let mut incomplete_final_rows_dropped = 0usize;
    for idx in &groups_after_reversal {
        if let Some(&last_row) = idx.last() {
            if state[last_row] != State::Rest {
                let final_rest = rest[last_row];
                for &row in idx {
                    if rest[row] == final_rest && keep[row] {
                        keep[row] = false;
                        incomplete_final_rows_dropped += 1;
                    }
                }
            }
        }
    }

    let mut out = SegmentedData {
        row_index: Vec::new(),
        group_id: Vec::new(),
        cyc_n: Vec::new(),
        state: Vec::new(),
        rest: Vec::new(),
        t: Vec::new(),
        step_t: Vec::new(),
        voltage: Vec::new(),
        current: Vec::new(),
        charge: Vec::new(),
    };
    for row in 0..n {
        if keep[row] {
            out.row_index.push(row);
            out.group_id.push(group_id[row]);
            out.cyc_n.push(cyc_n[row]);
            out.state.push(state[row]);
            out.rest.push(rest[row]);
            out.t.push(t[row]);
            out.step_t.push(step_t[row]);
            out.voltage.push(voltage[row]);
            out.current.push(current[row]);
            out.charge.push(anchored_charge[row]);
        }
    }

    (
        out,
        SegmentLog {
            reversal_rows_dropped,
            incomplete_final_rows_dropped,
            leading_rest_rows_dropped,
        },
    )
}

/// §6: which end of a `(group, cyc.n, state)` run `Q` is anchored to zero
/// at. `Start` matches `segment()`'s own (R's only) behaviour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnchorPoint {
    Start,
    End,
}

#[derive(Debug, Clone, Copy)]
pub struct QAnchorConfig {
    pub charge: AnchorPoint,
    pub discharge: AnchorPoint,
}

/// Re-derives per-state Q anchoring (§6) from `seg.charge`, which
/// `segment()` always start-anchors. For a `(group, cyc.n, state)` key
/// whose state maps to [`AnchorPoint::End`], subtracts that key's *last*
/// (already start-anchored) value from every row sharing the key --
/// algebraically identical to re-anchoring from the last raw sample, since
/// it's just a second constant shift within the same start-anchored group.
/// `State::Rest` rows are always left as `segment()` produced them: R has
/// no other behaviour for them and the UI exposes no control over it.
///
/// Uses the same grouping semantics as `segment()`'s own anchoring pass
/// (`assign_within_group`, keyed on `(group_id, cyc.n, state)`) -- note
/// this is *not* "contiguous run", it's every row sharing the key
/// anywhere within the group, matching R's `ave(..., interaction(...))`.
pub fn reanchor_charge(seg: &SegmentedData, config: &QAnchorConfig) -> Vec<f64> {
    type Key = (u32, u64, State);
    let key_of = |row: usize| -> Key { (seg.group_id[row], seg.cyc_n[row].to_bits(), seg.state[row]) };

    let mut last_value: HashMap<Key, f64> = HashMap::new();
    for row in 0..seg.charge.len() {
        last_value.insert(key_of(row), seg.charge[row]);
    }

    let anchor_for = |state: State| -> AnchorPoint {
        match state {
            State::Charge => config.charge,
            State::Discharge => config.discharge,
            State::Rest => AnchorPoint::Start,
        }
    };

    (0..seg.charge.len())
        .map(|row| match anchor_for(seg.state[row]) {
            AnchorPoint::Start => seg.charge[row],
            AnchorPoint::End => seg.charge[row] - last_value[&key_of(row)],
        })
        .collect()
}

#[derive(Debug, Clone, Copy)]
pub struct SummaryConfig {
    pub voltage_interpolation_window: Option<f64>,
    pub current_average_window: Option<f64>,
    /// When true, `I` is the mean of *all* finite current values in the
    /// active segment rather than the final-window mean.
    pub legacy_compatibility: bool,
}

/// One row per interruption (active segment). Row order is simply the order
/// in which each `(group, cyc.n, state, rest)` key first appears among the
/// retained rows -- unlike R's `interaction()`-driven iteration order, this
/// carries no particular meaning and callers that care about a specific
/// order (e.g. golden-file comparison) should sort/key-match explicitly.
#[derive(Debug, Clone)]
pub struct InterruptionSummary {
    pub group_id: Vec<u32>,
    pub cyc_n: Vec<f64>,
    pub state: Vec<State>,
    pub rest: Vec<u32>,
    pub t: Vec<f64>,
    pub step_t: Vec<f64>,
    pub e: Vec<f64>,
    pub i: Vec<f64>,
    pub q: Vec<f64>,
}

/// §7.6: per-interruption summary over active segments, grouped by
/// `(group, cyc.n, state, rest)`. `Efirst` (R's exact duplicate of `E`) is
/// dropped per §14 item 3.
pub fn interruption_summary(seg: &SegmentedData, config: &SummaryConfig) -> InterruptionSummary {
    let n = seg.state.len();
    let active_idx: Vec<usize> = (0..n).filter(|&i| seg.state[i] != State::Rest).collect();

    type Key = (u32, u64, State, u32);
    let mut order: Vec<Key> = Vec::new();
    let mut groups: HashMap<Key, Vec<usize>> = HashMap::new();
    for &i in &active_idx {
        let key: Key = (
            seg.group_id[i],
            seg.cyc_n[i].to_bits(),
            seg.state[i],
            seg.rest[i],
        );
        groups
            .entry(key)
            .or_insert_with(|| {
                order.push(key);
                Vec::new()
            })
            .push(i);
    }

    let mut out = InterruptionSummary {
        group_id: Vec::new(),
        cyc_n: Vec::new(),
        state: Vec::new(),
        rest: Vec::new(),
        t: Vec::new(),
        step_t: Vec::new(),
        e: Vec::new(),
        i: Vec::new(),
        q: Vec::new(),
    };

    for key in order {
        let idx = &groups[&key];
        let step_time: Vec<f64> = idx.iter().map(|&r| seg.step_t[r]).collect();
        let voltage: Vec<f64> = idx.iter().map(|&r| seg.voltage[r]).collect();
        let current: Vec<f64> = idx.iter().map(|&r| seg.current[r]).collect();

        let e_value = crate::types::interpolate_endpoint(
            &step_time,
            &voltage,
            config.voltage_interpolation_window,
        );
        let current_window: Vec<f64> = if config.legacy_compatibility {
            current.iter().copied().filter(|v| v.is_finite()).collect()
        } else {
            crate::types::select_final_window(&step_time, &current, config.current_average_window)
        };
        let i_value = if current_window.is_empty() {
            f64::NAN
        } else {
            current_window.iter().sum::<f64>() / current_window.len() as f64
        };

        let last = *idx.last().expect("group is non-empty by construction");
        out.group_id.push(key.0);
        out.cyc_n.push(seg.cyc_n[last]);
        out.state.push(key.2);
        out.rest.push(key.3);
        out.t.push(seg.t[last]);
        out.step_t.push(seg.step_t[last]);
        out.e.push(e_value);
        out.i.push(i_value);
        out.q.push(seg.charge[last]);
    }

    out
}

fn group_indices(group_id: &[u32]) -> Vec<Vec<usize>> {
    let mut map: HashMap<u32, Vec<usize>> = HashMap::new();
    let mut order: Vec<u32> = Vec::new();
    for (i, &g) in group_id.iter().enumerate() {
        map.entry(g)
            .or_insert_with(|| {
                order.push(g);
                Vec::new()
            })
            .push(i);
    }
    order.into_iter().map(|g| map.remove(&g).unwrap()).collect()
}

/// For each group (in the row order given by `groups`), assigns
/// `source[row] - source[first row with this key in the group]` into `out`.
fn assign_within_group<K: Eq + Hash + Clone>(
    groups: &[Vec<usize>],
    keys: &[K],
    source: &[f64],
    out: &mut [f64],
) {
    for idx in groups {
        let mut first_seen: HashMap<K, f64> = HashMap::new();
        for &row in idx {
            let key = keys[row].clone();
            let first = *first_seen.entry(key).or_insert(source[row]);
            out[row] = source[row] - first;
        }
    }
}

/// Port of `find_unrested_reversal_rows()` (ici_analysis.R lines 354-376).
/// Marks every sample in an active run that reverses directly into the
/// opposite active state with no intervening rest.
pub fn find_unrested_reversal_rows(state: &[State]) -> Vec<bool> {
    let n = state.len();
    let mut remove = vec![false; n];
    if n < 2 {
        return remove;
    }
    for i in 0..n - 1 {
        let active_i = state[i] != State::Rest;
        let active_next = state[i + 1] != State::Rest;
        if active_i && active_next && state[i] != state[i + 1] {
            let end_index = i;
            let mut start_index = end_index;
            while start_index > 0
                && state[start_index - 1] != State::Rest
                && state[start_index - 1] == state[end_index]
            {
                start_index -= 1;
            }
            for row in &mut remove[start_index..=end_index] {
                *row = true;
            }
        }
    }
    remove
}

#[derive(Debug, Clone)]
pub struct CurrentLevels {
    pub low_centre: f64,
    pub high_centre: f64,
    pub low_sd: f64,
    pub high_sd: f64,
    pub gap: f64,
    pub clearly_separated: bool,
    pub suggested_threshold: f64,
}

/// Port of `split_current_levels()` (ici_analysis.R lines 264-305): an exact
/// 1-D two-cluster split of `|I|` minimising total within-cluster SSE.
pub fn split_current_levels(current: &[f64]) -> Option<CurrentLevels> {
    let mut all_values: Vec<f64> = current
        .iter()
        .copied()
        .filter(|v| v.is_finite())
        .map(f64::abs)
        .collect();
    all_values.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = all_values.len();
    if n < 4 {
        return None;
    }
    let mut distinct = all_values.clone();
    distinct.dedup();
    if distinct.len() < 2 {
        return None;
    }

    let mut cumulative = vec![0.0f64; n];
    let mut cumulative_sq = vec![0.0f64; n];
    let mut running = 0.0;
    let mut running_sq = 0.0;
    for i in 0..n {
        running += all_values[i];
        running_sq += all_values[i] * all_values[i];
        cumulative[i] = running;
        cumulative_sq[i] = running_sq;
    }
    let total_sum = cumulative[n - 1];
    let total_sq = cumulative_sq[n - 1];

    let mut best_left_count = 1usize;
    let mut best_sse = f64::INFINITY;
    for left_count in 1..n {
        let left_n = left_count as f64;
        let right_n = (n - left_count) as f64;
        let left_sum = cumulative[left_count - 1];
        let left_sq = cumulative_sq[left_count - 1];
        let left_sse = left_sq - left_sum * left_sum / left_n;
        let right_sum = total_sum - left_sum;
        let right_sq = total_sq - left_sq;
        let right_sse = right_sq - right_sum * right_sum / right_n;
        let total = left_sse + right_sse;
        if total < best_sse {
            best_sse = total;
            best_left_count = left_count;
        }
    }

    let low = &all_values[..best_left_count];
    let high = &all_values[best_left_count..];
    let low_centre = mean(low);
    let high_centre = mean(high);
    let low_sd = if low.len() > 1 {
        sd(low, low_centre)
    } else {
        0.0
    };
    let high_sd = if high.len() > 1 {
        sd(high, high_centre)
    } else {
        0.0
    };
    let gap = high_centre - low_centre;
    let clearly_separated = gap.is_finite()
        && gap > 3.0 * low_sd.max(high_sd).max(1e-12)
        && high_centre > (low_centre * 1.5f64).max(1e-12);

    Some(CurrentLevels {
        low_centre,
        high_centre,
        low_sd,
        high_sd,
        gap,
        clearly_separated,
        suggested_threshold: (low_centre + high_centre) / 2.0,
    })
}

fn mean(values: &[f64]) -> f64 {
    values.iter().sum::<f64>() / values.len() as f64
}

fn sd(values: &[f64], mean_val: f64) -> f64 {
    let variance =
        values.iter().map(|v| (v - mean_val).powi(2)).sum::<f64>() / (values.len() as f64 - 1.0);
    variance.sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn states(labels: &[&str]) -> Vec<State> {
        labels
            .iter()
            .map(|s| match *s {
                "R" => State::Rest,
                "charge" | "A" => State::Charge,
                "discharge" | "B" => State::Discharge,
                other => panic!("unknown state label {other}"),
            })
            .collect()
    }

    #[test]
    fn rest_indexing_matches_spec_example() {
        // R R A A R R A A -> 1 1 2 2 2 2 3 3 is §7.2's worked indexing
        // example, single group. A trailing R R is appended so the group
        // doesn't *end* active -- §7.5 unconditionally drops a final active
        // step, which would otherwise remove the trailing "A A" this
        // example is about and defeat the point of the check. The leading
        // "R R" (rest id 1) has no preceding active run, though, so it's
        // dropped by the leading-rest exclusion -- only rest ids 2 and 3
        // survive into `seg`.
        let current = [0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0];
        let group_id = vec![0u32; current.len()];
        let t: Vec<f64> = (0..current.len()).map(|i| i as f64).collect();
        let cyc_n = vec![1.0; current.len()];
        let voltage = vec![0.0; current.len()];
        let charge = vec![0.0; current.len()];
        let config = SegmentConfig {
            state_threshold: 0.0,
            drop_unrested_reversals: false,
        };
        let (seg, log) = segment(&group_id, &t, &cyc_n, &current, &voltage, &charge, &config);
        assert_eq!(log.leading_rest_rows_dropped, 2);
        assert_eq!(seg.row_index, vec![2, 3, 4, 5, 6, 7, 8, 9]);
        assert_eq!(seg.rest, vec![2, 2, 2, 2, 3, 3, 3, 3]);
    }

    #[test]
    fn rest_indexing_group_starting_active() {
        // Trailing R avoids the unconditional §7.5 incomplete-final-step
        // drop, isolating the "no leading R -> no transition yet" behaviour
        // this test is actually about.
        let current = [1.0, 1.0, 0.0, 0.0, -1.0, -1.0, 0.0];
        let group_id = vec![0u32; current.len()];
        let t: Vec<f64> = (0..current.len()).map(|i| i as f64).collect();
        let cyc_n = vec![1.0; current.len()];
        let voltage = vec![0.0; current.len()];
        let charge = vec![0.0; current.len()];
        let config = SegmentConfig {
            state_threshold: 0.0,
            drop_unrested_reversals: false,
        };
        let (seg, log) = segment(&group_id, &t, &cyc_n, &current, &voltage, &charge, &config);
        // No leading R -> no transition yet -> rest stays 1 through the R
        // run, then increments at the next R->active transition.
        assert_eq!(seg.rest, vec![1, 1, 1, 1, 2, 2, 2]);
        // A group starting active has no leading rest to exclude.
        assert_eq!(log.leading_rest_rows_dropped, 0);
    }

    #[test]
    fn leading_rest_with_no_preceding_active_run_is_excluded_and_flagged() {
        // A single spurious rest sample at the very start of a group's data
        // (e.g. a large-format cell cycler's first logged row before
        // cycling begins) has no active run to pair with under §7.2's
        // on-then-off convention -- it should be dropped, not surfaced
        // downstream as a "0 usable points" regression failure.
        let current = [0.0, 1.0, 1.0, 0.0, 0.0];
        let group_id = vec![0u32; current.len()];
        let t: Vec<f64> = (0..current.len()).map(|i| i as f64).collect();
        let cyc_n = vec![1.0; current.len()];
        let voltage = vec![0.0; current.len()];
        let charge = vec![0.0; current.len()];
        let config = SegmentConfig::default();
        let (seg, log) = segment(&group_id, &t, &cyc_n, &current, &voltage, &charge, &config);
        assert_eq!(log.leading_rest_rows_dropped, 1);
        assert_eq!(seg.row_index, vec![1, 2, 3, 4]);
        assert_eq!(seg.rest, vec![2, 2, 2, 2]);
    }

    #[test]
    fn group_that_is_entirely_leading_rest_is_dropped_cleanly() {
        // No active run ever occurs -- the whole group is the "leading
        // rest" and should be dropped without panicking.
        let current = [0.0, 0.0, 0.0];
        let group_id = vec![0u32; current.len()];
        let t: Vec<f64> = (0..current.len()).map(|i| i as f64).collect();
        let cyc_n = vec![1.0; current.len()];
        let voltage = vec![0.0; current.len()];
        let charge = vec![0.0; current.len()];
        let config = SegmentConfig::default();
        let (seg, log) = segment(&group_id, &t, &cyc_n, &current, &voltage, &charge, &config);
        assert_eq!(log.leading_rest_rows_dropped, 3);
        assert!(seg.row_index.is_empty());
    }

    #[test]
    fn rest_indexing_group_ending_active_is_dropped_by_incomplete_step() {
        // A A R R A A: starts active (so the leading-rest exclusion doesn't
        // interfere) and ends active (so §7.5's incomplete-final-step drop
        // fires), isolating the behaviour this test is actually about.
        let current = [1.0, 1.0, 0.0, 0.0, 1.0, 1.0];
        let group_id = vec![0u32; current.len()];
        let t: Vec<f64> = (0..current.len()).map(|i| i as f64).collect();
        let cyc_n = vec![1.0; current.len()];
        let voltage = vec![0.0; current.len()];
        let charge = vec![0.0; current.len()];
        let config = SegmentConfig::default();
        let (seg, log) = segment(&group_id, &t, &cyc_n, &current, &voltage, &charge, &config);
        assert_eq!(log.incomplete_final_rows_dropped, 2);
        assert_eq!(seg.row_index, vec![0, 1, 2, 3]);
    }

    #[test]
    fn interruption_summary_basic() {
        // R(0) R(1) charge(2..4) R(5..7): one interruption.
        let current = [0.0, 0.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0];
        let voltage = [1.0, 1.0, 1.1, 1.2, 1.3, 1.35, 1.34, 1.33];
        let n = current.len();
        let group_id = vec![0u32; n];
        let t: Vec<f64> = (0..n).map(|i| i as f64).collect();
        let cyc_n = vec![1.0; n];
        let charge = vec![0.0, 0.0, 0.1, 0.2, 0.3, 0.4, 0.4, 0.4];
        let config = SegmentConfig::default();
        let (seg, _log) = segment(&group_id, &t, &cyc_n, &current, &voltage, &charge, &config);

        let summary_config = SummaryConfig {
            voltage_interpolation_window: None,
            current_average_window: None,
            legacy_compatibility: false,
        };
        let summary = interruption_summary(&seg, &summary_config);
        assert_eq!(summary.rest.len(), 1);
        assert_eq!(summary.state[0], State::Charge);
        assert_eq!(summary.t[0], 4.0);
        assert_eq!(summary.step_t[0], 2.0); // step.t within (group,rest,state): starts at row2 -> 0,1,2
        assert_eq!(summary.e[0], 1.3); // window=None -> last point
        assert_eq!(summary.i[0], 1.0); // window=None -> last point
                                       // Q is anchored within (group, cyc.n, state) before this reads it:
                                       // charge state starts at row 2 (raw Q=0.1), so the last active row's
                                       // (row 4, raw Q=0.3) anchored value is 0.3 - 0.1 = 0.2.
        assert!((summary.q[0] - 0.2).abs() < 1e-12);
    }

    #[test]
    fn interruption_summary_legacy_compatibility_averages_all_active_current() {
        let current = [0.0, 1.0, 1.0, 2.0, 0.0];
        let voltage = [1.0, 1.1, 1.2, 1.3, 1.29];
        let n = current.len();
        let group_id = vec![0u32; n];
        let t: Vec<f64> = (0..n).map(|i| i as f64).collect();
        let cyc_n = vec![1.0; n];
        let charge = vec![0.0; n];
        let config = SegmentConfig::default();
        let (seg, _log) = segment(&group_id, &t, &cyc_n, &current, &voltage, &charge, &config);

        let summary_config = SummaryConfig {
            voltage_interpolation_window: None,
            current_average_window: None,
            legacy_compatibility: true,
        };
        let summary = interruption_summary(&seg, &summary_config);
        // legacy: mean of ALL active current values (1.0, 1.0, 2.0), not just the final window.
        assert!((summary.i[0] - 4.0 / 3.0).abs() < 1e-12);
    }

    #[test]
    fn unrested_reversal_at_very_first_sample() {
        let state = states(&["charge", "discharge", "R", "R"]);
        let remove = find_unrested_reversal_rows(&state);
        assert_eq!(remove, vec![true, false, false, false]);
    }

    #[test]
    fn unrested_reversal_at_very_last_sample() {
        let state = states(&["R", "R", "charge", "discharge"]);
        let remove = find_unrested_reversal_rows(&state);
        assert_eq!(remove, vec![false, false, true, false]);
    }

    #[test]
    fn unrested_reversal_no_reversal_present() {
        let state = states(&["R", "charge", "charge", "R", "discharge"]);
        let remove = find_unrested_reversal_rows(&state);
        assert_eq!(remove, vec![false, false, false, false, false]);
    }

    #[test]
    fn unrested_reversal_multi_sample_run_before_reversal() {
        let state = states(&["R", "charge", "charge", "charge", "discharge", "R"]);
        let remove = find_unrested_reversal_rows(&state);
        assert_eq!(remove, vec![false, true, true, true, false, false]);
    }

    #[test]
    fn split_current_levels_bimodal() {
        let current = [0.0, 0.0, 0.01, -0.01, 1.0, 1.02, -0.98, 1.01];
        let levels = split_current_levels(&current).unwrap();
        assert!(levels.clearly_separated);
        assert!(levels.low_centre < 0.1);
        assert!(levels.high_centre > 0.9);
        assert!(levels.suggested_threshold > levels.low_centre);
        assert!(levels.suggested_threshold < levels.high_centre);
    }

    #[test]
    fn split_current_levels_unimodal_not_separated() {
        let current = [0.98, 1.0, 1.01, 0.99, 1.02, 0.97, 1.0, 1.01];
        let levels = split_current_levels(&current).unwrap();
        assert!(!levels.clearly_separated);
    }

    #[test]
    fn split_current_levels_too_few_values_returns_none() {
        assert!(split_current_levels(&[1.0, 2.0]).is_none());
    }

    /// R R charge charge charge R R discharge discharge discharge R R,
    /// single group/cycle, with a raw `Q` trajectory chosen so start- and
    /// end-anchored values are easy to hand-check (§6's `reanchor_charge`).
    fn q_anchoring_fixture() -> SegmentedData {
        // A single dummy leading Charge sample (its own, distinct cyc.n so
        // it doesn't share the real charge run's anchoring key) keeps the
        // group from *starting* with Rest -- this fixture is about Q
        // anchoring, not the leading-rest exclusion, so it shouldn't be
        // affected by that unrelated feature dropping rows 0-1.
        let current = [1.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.0, 0.0, -1.0, -1.0, -1.0, 0.0, 0.0];
        let n = current.len();
        let group_id = vec![0u32; n];
        let t: Vec<f64> = (0..n).map(|i| i as f64).collect();
        let cyc_n = vec![0.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0];
        let voltage = vec![0.0; n];
        let charge = vec![0.0, 0.0, 0.0, 1.0, 2.0, 3.0, 3.0, 3.0, 2.0, 1.0, 0.0, 0.0, 0.0];
        let config = SegmentConfig {
            state_threshold: 0.0,
            drop_unrested_reversals: false,
        };
        let (seg, _log) = segment(&group_id, &t, &cyc_n, &current, &voltage, &charge, &config);
        seg
    }

    #[test]
    fn reanchor_charge_start_start_is_a_no_op() {
        let seg = q_anchoring_fixture();
        let reanchored = reanchor_charge(
            &seg,
            &QAnchorConfig {
                charge: AnchorPoint::Start,
                discharge: AnchorPoint::Start,
            },
        );
        assert_eq!(reanchored, seg.charge);
    }

    #[test]
    fn reanchor_charge_end_end_zeroes_last_value_of_every_run() {
        let seg = q_anchoring_fixture();
        // Start-anchored charge state rows (3,4,5) are [0,1,2]; discharge
        // rows (8,9,10) are [0,-1,-2] (first occurrence of each key anchors
        // to 0, matching §6's "start of half-cycle" default).
        assert_eq!(&seg.charge[3..6], &[0.0, 1.0, 2.0]);
        assert_eq!(&seg.charge[8..11], &[0.0, -1.0, -2.0]);

        let reanchored = reanchor_charge(
            &seg,
            &QAnchorConfig {
                charge: AnchorPoint::End,
                discharge: AnchorPoint::End,
            },
        );
        // End-anchored: subtract each key's *last* start-anchored value.
        assert_eq!(&reanchored[3..6], &[-2.0, -1.0, 0.0]);
        assert_eq!(&reanchored[8..11], &[2.0, 1.0, 0.0]);
        // Rest rows (1,2,6,7,11,12) are always start-anchored, unchanged.
        for &row in &[1usize, 2, 6, 7, 11, 12] {
            assert_eq!(reanchored[row], seg.charge[row], "rest row {row}");
        }
    }

    #[test]
    fn reanchor_charge_mixed_config_only_changes_the_configured_state() {
        let seg = q_anchoring_fixture();
        let reanchored = reanchor_charge(
            &seg,
            &QAnchorConfig {
                charge: AnchorPoint::End,
                discharge: AnchorPoint::Start,
            },
        );
        assert_eq!(&reanchored[3..6], &[-2.0, -1.0, 0.0]); // charge: end-anchored
        assert_eq!(&reanchored[8..11], &seg.charge[8..11]); // discharge: unchanged (start)
    }
}
