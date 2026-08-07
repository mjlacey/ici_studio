//! Sign-convention and round-trip tests, independent of R (ICI_WEB_SPEC.md
//! §13.3). Golden-file tests can't catch a sign error the reference
//! implementation itself shares -- these two checks close that gap.
//!
//! 1. `R > 0` and `k > 0` for every rest in the real reference fixture.
//! 2. A synthetic round-trip: build E(t) from *known* positive R_true(Q) and
//!    k_true(Q), recover them through the whole pipeline, separately for
//!    charge and discharge, then again with the current column's sign
//!    convention flipped, and assert identical, still-positive recovery.

use ici_core::derive::{derive, DeriveConfig};
use ici_core::regress::{rest_regression, RegressionConfig};
use ici_core::segment::{interruption_summary, segment, SegmentConfig, SummaryConfig};
use serde_json::Value;
use std::fs;
use std::path::PathBuf;

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn field_f64(row: &Value, name: &str) -> f64 {
    match row.get(name) {
        None | Some(Value::Null) => f64::NAN,
        Some(v) => v.as_f64().unwrap_or(f64::NAN),
    }
}

fn field_str(row: &Value, name: &str) -> String {
    row.get(name)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/// §13.3: `R > 0` and `k > 0` for every rest in the reference fixture,
/// computed with `nonphysical_to_na = false` so a sign bug would show up as
/// a visible negative value rather than being silently NaN'd away first.
///
/// The committed trim of the real file happens to fall entirely within an
/// initial discharge-only characterisation sequence (verified against the
/// raw file when the fixture was built), so this only exercises the
/// discharge state; charge coverage comes from the synthetic round-trip
/// test below, which is independent of R and covers both states with known
/// ground truth.
#[test]
fn real_reference_fixture_has_only_positive_r_and_k() {
    let path = fixtures_dir().join("golden_case_1.json");
    let Ok(text) = fs::read_to_string(&path) else {
        eprintln!(
            "SKIP: {} not present. Run `Rscript tools/make_fixtures.R` first.",
            path.display()
        );
        return;
    };
    let case: Value = serde_json::from_str(&text).unwrap();

    let summary_rows = case["summary"].as_array().unwrap();
    let regression_rows = case["regression"].as_array().unwrap();
    assert!(!summary_rows.is_empty());

    let mut by_state: std::collections::HashMap<String, (usize, usize)> =
        std::collections::HashMap::new();
    let mut regression_by_rest: std::collections::HashMap<u64, &Value> =
        std::collections::HashMap::new();
    for row in regression_rows {
        regression_by_rest.insert(row["rest"].as_u64().unwrap(), row);
    }

    for row in summary_rows {
        let rest = row["rest"].as_u64().unwrap();
        let reg = regression_by_rest.get(&rest).unwrap();
        let e = field_f64(row, "E");
        let i = field_f64(row, "I");
        let e0 = field_f64(reg, "E0");
        let s = field_f64(reg, "s");
        let i0 = field_f64(reg, "I0");
        let delta_i = i - i0;
        let r = (e - e0) / delta_i;
        let k = -s / delta_i;
        let state = field_str(row, "state");
        let entry = by_state.entry(state.clone()).or_insert((0, 0));
        entry.0 += 1;
        if r > 0.0 && r.is_finite() && k > 0.0 && k.is_finite() {
            entry.1 += 1;
        } else {
            panic!(
                "rest {rest} (state={state}): R={r}, k={k} -- expected both positive and finite"
            );
        }
    }

    assert!(
        by_state.contains_key("discharge"),
        "expected at least the discharge state to be present"
    );
    for (state, (total, positive)) in &by_state {
        assert_eq!(
            total, positive,
            "state {state}: {positive}/{total} rests had positive, finite R and k"
        );
    }
}

// ---------------------------------------------------------------------
// Synthetic round-trip test
// ---------------------------------------------------------------------

fn r_true(q: f64) -> f64 {
    0.05 + 0.01 * q
}

fn k_true(q: f64) -> f64 {
    0.002 + 0.0005 * q
}

/// Tiny deterministic PRNG so the "small Gaussian noise" is reproducible
/// without adding a `rand` dependency. A sum of uniforms approximates a
/// Gaussian well enough for "small noise on a synthetic fixture".
struct Lcg(u64);
impl Lcg {
    fn next_unit(&mut self) -> f64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((self.0 >> 11) as f64) / ((1u64 << 53) as f64)
    }
    fn small_noise(&mut self, sigma: f64) -> f64 {
        let u = self.next_unit() + self.next_unit() + self.next_unit() - 1.5;
        u * sigma
    }
}

struct Dataset {
    t: Vec<f64>,
    cyc_n: Vec<f64>,
    current: Vec<f64>,
    voltage: Vec<f64>,
    charge: Vec<f64>,
    /// Ground truth per pulse, in pulse order: (rest_id, anchored_q, is_charge).
    truth: Vec<(u32, f64, bool)>,
}

const DQ: f64 = 0.05;
const I_PULSE: f64 = 1.0;
const E_END: f64 = 3.0;
const ACTIVE_POINTS: usize = 3;
const REST_POINTS: usize = 6;
const NOISE_SIGMA: f64 = 0.0003;

/// Builds a self-consistent synthetic dataset: `current_sign` flips the
/// polarity convention used for *both* generating E(t) from `ΔI` and (later)
/// recovering `ΔI` from the same current column, so a correct pipeline must
/// recover the same positive `R_true`/`k_true` regardless of its value.
fn build_dataset(n_pulses: usize, current_sign: f64, seed: u64, noise_sigma: f64) -> Dataset {
    let mut rng = Lcg(seed);
    let mut t = Vec::new();
    let mut cyc_n = Vec::new();
    let mut current = Vec::new();
    let mut voltage = Vec::new();
    let mut charge = Vec::new();
    let mut truth = Vec::new();

    let mut time = 0.0;
    for pulse in 0..n_pulses {
        let is_charge = pulse % 2 == 0;
        let type_index = (pulse / 2) as f64;
        let anchored_q = 2.0 * type_index * DQ; // see derivation in module docs below.
        let q_raw = pulse as f64 * DQ;

        let base_sign = if is_charge { 1.0 } else { -1.0 };
        let i_signed = base_sign * I_PULSE * current_sign;
        let delta_i = i_signed; // I0 = 0 during rest.
        let r = r_true(anchored_q);
        let k = k_true(anchored_q);
        let e0 = E_END - delta_i * r;
        let s = -delta_i * k;

        for _ in 0..ACTIVE_POINTS {
            t.push(time);
            cyc_n.push(1.0);
            current.push(i_signed);
            voltage.push(E_END);
            charge.push(q_raw);
            time += 1.0;
        }
        for j in 0..REST_POINTS {
            let step_t = j as f64;
            let noise = rng.small_noise(noise_sigma);
            t.push(time);
            cyc_n.push(1.0);
            current.push(0.0);
            voltage.push(e0 + s * step_t.sqrt() + noise);
            charge.push(q_raw);
            time += 1.0;
        }

        truth.push(((pulse + 1) as u32, anchored_q, is_charge));
    }

    Dataset {
        t,
        cyc_n,
        current,
        voltage,
        charge,
        truth,
    }
}

/// Runs the whole Stage-A pipeline and returns `(rest_id -> (R, k, state))`.
fn run_pipeline(data: &Dataset) -> std::collections::HashMap<u32, (f64, f64, &'static str)> {
    let n = data.t.len();
    let group_id = vec![0u32; n];
    let (seg, log) = segment(
        &group_id,
        &data.t,
        &data.cyc_n,
        &data.current,
        &data.voltage,
        &data.charge,
        &SegmentConfig::default(),
    );
    assert_eq!(log.reversal_rows_dropped, 0);
    assert_eq!(log.incomplete_final_rows_dropped, 0);

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

    let (analysis, _report) = derive(
        &summary,
        &regression,
        &DeriveConfig {
            nonphysical_to_na: false,
        },
    );

    let mut out = std::collections::HashMap::new();
    for i in 0..analysis.rest.len() {
        out.insert(
            analysis.rest[i],
            (analysis.r[i], analysis.k[i], analysis.state[i].as_str()),
        );
    }
    out
}

#[test]
fn synthetic_round_trip_recovers_known_positive_r_and_k() {
    let n_pulses = 20; // 10 charge + 10 discharge, Q spanning [0, 0.9).
    let dataset = build_dataset(n_pulses, 1.0, 42, NOISE_SIGMA);
    let results = run_pipeline(&dataset);

    let r_tol = 0.01; // Ohm; noise sigma is 0.3 mV against ~50-60 mOhm true R.
    let k_tol = 0.001; // Ohm s^-1/2

    let mut n_charge = 0;
    let mut n_discharge = 0;
    for &(rest_id, q, is_charge) in &dataset.truth {
        let &(r, k, state) = results
            .get(&rest_id)
            .unwrap_or_else(|| panic!("no analysis row for rest {rest_id}"));
        assert_eq!(
            state == "charge",
            is_charge,
            "rest {rest_id}: state label mismatch"
        );
        assert!(
            r > 0.0,
            "rest {rest_id} ({state}): R={r} should be positive"
        );
        assert!(
            k > 0.0,
            "rest {rest_id} ({state}): k={k} should be positive"
        );
        assert!(
            (r - r_true(q)).abs() < r_tol,
            "rest {rest_id} ({state}): R={r}, expected ~{} (Q={q})",
            r_true(q)
        );
        assert!(
            (k - k_true(q)).abs() < k_tol,
            "rest {rest_id} ({state}): k={k}, expected ~{} (Q={q})",
            k_true(q)
        );
        if is_charge {
            n_charge += 1;
        } else {
            n_discharge += 1;
        }
    }
    assert_eq!(n_charge, 10);
    assert_eq!(n_discharge, 10);
}

#[test]
fn synthetic_round_trip_is_invariant_to_current_sign_flip() {
    // Noise-free here deliberately: this test isolates the *algebraic*
    // sign-convention property (ΔI flips, (E - E0) is generated
    // self-consistently from the same ΔI, so R/k must come out identical).
    // With noise, the two runs become two *independent* noisy estimates of
    // the same R_true/k_true -- adding noise scaled by 1/ΔI on one side and
    // 1/(-ΔI) on the other, which legitimately differ by ~noise/ΔI. That
    // recovery-under-noise property is already covered by the test above.
    let n_pulses = 20;
    let normal = build_dataset(n_pulses, 1.0, 7, 0.0);
    let flipped = build_dataset(n_pulses, -1.0, 7, 0.0);

    let normal_results = run_pipeline(&normal);
    let flipped_results = run_pipeline(&flipped);

    for &(rest_id, q, is_charge_normal) in &normal.truth {
        let &(r_n, k_n, state_n) = normal_results.get(&rest_id).unwrap();
        let &(r_f, k_f, state_f) = flipped_results.get(&rest_id).unwrap();

        // The current-sign flip swaps which physical pulse type is *labelled*
        // charge vs discharge, but every individual rest's recovered R/k
        // must be unchanged and still positive (§13.3) -- that's the point
        // of the test.
        assert_eq!(state_n == "charge", is_charge_normal);
        assert_ne!(
            state_f, state_n,
            "rest {rest_id}: flipping current sign should swap the state label"
        );

        assert!(
            r_n > 0.0 && r_f > 0.0,
            "rest {rest_id}: R should stay positive under a sign flip"
        );
        assert!(
            k_n > 0.0 && k_f > 0.0,
            "rest {rest_id}: k should stay positive under a sign flip"
        );
        assert!(
            (r_n - r_f).abs() < 1e-9,
            "rest {rest_id}: R changed under sign flip ({r_n} vs {r_f})"
        );
        assert!(
            (k_n - k_f).abs() < 1e-9,
            "rest {rest_id}: k changed under sign flip ({k_n} vs {k_f})"
        );
        assert!((r_n - r_true(q)).abs() < 0.01);
        assert!((k_n - k_true(q)).abs() < 0.001);
    }
}
