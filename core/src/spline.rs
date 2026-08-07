//! P-spline / SCOP-spline smoother (§8.2 of ICI_WEB_SPEC.md). Highest-risk
//! module in the port -- see §16.1. `scam`/`mgcv` are not available in
//! WASM, so this is a from-scratch reimplementation: small numerical
//! differences from R are expected and bounded by the golden-file
//! tolerances (§13.1), not eliminated. Only `smooth_bspline_vec`'s
//! fit-and-predict contract is ported; the unused derivative/grid machinery
//! inside R's `smooth_bspline()` is intentionally not (§14 item 9).

use nalgebra::{DMatrix, DVector};

const DEGREE: usize = 3; // cubic B-spline (order 4).
const MIN_DISTINCT_X: usize = 8;
const REML_LOG_LAMBDA_LO: f64 = -20.0;
const REML_LOG_LAMBDA_HI: f64 = 20.0;
const REML_GOLDEN_ITERS: usize = 60;
const PIRLS_MAX_ITERS: usize = 50;
const PIRLS_TOL: f64 = 1e-8;
const PIRLS_MAX_STEP_HALVINGS: usize = 20;
/// Bound on gamma's log-scale step components (`gamma[1..]`):
/// `exp(25) ~ 7e10`, already many orders of magnitude past any real
/// coefficient step -- this only stops a diverging Gauss-Newton iterate
/// from overflowing into a numerically catastrophic exponential, it never
/// binds for a well-conditioned fit.
const PIRLS_GAMMA_CLAMP: f64 = 25.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Automatic,
    Increasing,
    Decreasing,
}

#[derive(Debug, Clone, Copy)]
pub struct SplineConfig {
    pub monotonic: bool,
    pub direction: Direction,
    pub k: usize,
    pub m: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirectionUsed {
    Increasing,
    Decreasing,
    NotApplicable,
}

/// Diagnostics worth surfacing in the run log (§8.2: "Report the effective
/// k used after the clamp, and the selected lambda and effective degrees of
/// freedom").
#[derive(Debug, Clone)]
pub struct SplineDiagnostics {
    pub direction_used: DirectionUsed,
    pub k_effective: usize,
    pub lambda: f64,
    pub edf: f64,
}

/// Port of `smooth_bspline_vec()` (ici_analysis.R lines 15-58). Returns a
/// vector the same length/order as `x`/`y`; entries where `x`/`y` aren't
/// both finite are NaN. Returns all-NaN (with `None` diagnostics) when
/// fewer than 8 distinct finite `x` values are available, matching R's
/// warn-and-return-NA path.
pub fn smooth_bspline_vec(
    x: &[f64],
    y: &[f64],
    config: &SplineConfig,
) -> (Vec<f64>, Option<SplineDiagnostics>) {
    assert_eq!(x.len(), y.len());
    let n = x.len();
    let mut fitted = vec![f64::NAN; n];

    let valid: Vec<usize> = (0..n)
        .filter(|&i| x[i].is_finite() && y[i].is_finite())
        .collect();
    if valid.len() < MIN_DISTINCT_X {
        return (fitted, None);
    }

    // Average duplicate x values, then sort by x (R: aggregate(.y~.x, mean); order(.x)).
    let mut pairs: Vec<(f64, f64)> = valid.iter().map(|&i| (x[i], y[i])).collect();
    pairs.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    let mut fit_x: Vec<f64> = Vec::new();
    let mut fit_y: Vec<f64> = Vec::new();
    {
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
            fit_x.push(pairs[i].0);
            fit_y.push(sum / count);
            i = j;
        }
    }
    let n_distinct = fit_x.len();
    if n_distinct < MIN_DISTINCT_X {
        return (fitted, None);
    }

    let k = config.k.min(n_distinct.saturating_sub(2)).max(config.m + 2);
    let m = config.m;

    let direction_used = if config.monotonic {
        match config.direction {
            Direction::Increasing => DirectionUsed::Increasing,
            Direction::Decreasing => DirectionUsed::Decreasing,
            Direction::Automatic => {
                if automatic_direction_is_increasing(&fit_x, &fit_y) {
                    DirectionUsed::Increasing
                } else {
                    DirectionUsed::Decreasing
                }
            }
        }
    } else {
        DirectionUsed::NotApplicable
    };

    let basis = BSplineBasis::new(fit_x[0], *fit_x.last().unwrap(), k);
    let s = difference_penalty(k, m);
    let mp = m;
    let mr = k - m;

    let (coeffs, lambda, edf) = if config.monotonic {
        fit_monotonic(
            &basis,
            &fit_x,
            &fit_y,
            &s,
            mp,
            mr,
            direction_used == DirectionUsed::Increasing,
        )
    } else {
        fit_unconstrained(&basis, &fit_x, &fit_y, &s, mp, mr)
    };

    // Predict at the ORIGINAL valid x positions (with duplicates, original order) -- not the grid.
    for &i in &valid {
        let row = basis.eval_row(x[i]);
        fitted[i] = row.dot(&coeffs);
    }

    (
        fitted,
        Some(SplineDiagnostics {
            direction_used,
            k_effective: k,
            lambda,
            edf,
        }),
    )
}

/// R lines 130-140: direction from the median of all pairwise slopes
/// `(y_i - y_j)/(x_i - x_j)`. O(n^2); callers with >2000 points should
/// subsample first (not needed at ICI's per-rest scale -- see §8.2).
fn automatic_direction_is_increasing(x: &[f64], y: &[f64]) -> bool {
    let n = x.len();
    let mut slopes = Vec::with_capacity(n * (n - 1) / 2);
    for i in 0..n {
        for j in (i + 1)..n {
            let dx = x[i] - x[j];
            let dy = y[i] - y[j];
            let slope = dy / dx;
            if slope.is_finite() {
                slopes.push(slope);
            }
        }
    }
    slopes.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let median = if slopes.is_empty() {
        0.0
    } else if slopes.len() % 2 == 1 {
        slopes[slopes.len() / 2]
    } else {
        (slopes[slopes.len() / 2 - 1] + slopes[slopes.len() / 2]) / 2.0
    };
    median >= 0.0
}

// ---------------------------------------------------------------------
// B-spline basis (cubic, uniform clamped knots)
// ---------------------------------------------------------------------

struct BSplineBasis {
    knots: Vec<f64>,
    k: usize,
}

impl BSplineBasis {
    /// A genuine (Eilers-Marx-style) uniform P-spline knot sequence of
    /// dimension `k` over `[x_min, x_max]`: knots are equally spaced at
    /// `dx = (x_max-x_min)/(k-degree)` and *extend `degree` steps beyond
    /// each boundary* rather than repeating/clamping at it. This (not a
    /// clamped B-spline) is what mgcv's `bs="ps"`/`"mpi"`/`"mpd"` actually
    /// use -- a clamped basis pins the curve too tightly near the two
    /// boundary data points and was found empirically to miss the golden
    /// fixtures by several percent right where it matters most (the first
    /// and last Q values).
    fn new(x_min: f64, x_max: f64, k: usize) -> Self {
        assert!(k > DEGREE + 1, "k must exceed degree+1");
        let ndx = k - DEGREE; // equal intervals spanning [x_min, x_max]
        let dx = ((x_max - x_min) / (ndx as f64)).max(1e-12);
        let n_knots = k + DEGREE + 1;
        let knots: Vec<f64> = (0..n_knots)
            .map(|i| x_min + (i as f64 - DEGREE as f64) * dx)
            .collect();
        Self { knots, k }
    }

    fn find_span(&self, x: f64) -> usize {
        let n = self.k - 1; // last basis index
        if x >= self.knots[n + 1] {
            return n;
        }
        if x <= self.knots[DEGREE] {
            return DEGREE;
        }
        let (mut low, mut high) = (DEGREE, n + 1);
        let mut mid = (low + high) / 2;
        while x < self.knots[mid] || x >= self.knots[mid + 1] {
            if x < self.knots[mid] {
                high = mid;
            } else {
                low = mid;
            }
            mid = (low + high) / 2;
        }
        mid
    }

    /// The `degree+1` nonzero basis values at `x` (NURBS book Algorithm
    /// A2.2), for basis indices `span-degree ..= span`.
    fn basis_funs(&self, span: usize, x: f64) -> [f64; DEGREE + 1] {
        let mut n = [0.0f64; DEGREE + 1];
        let mut left = [0.0f64; DEGREE + 1];
        let mut right = [0.0f64; DEGREE + 1];
        n[0] = 1.0;
        for j in 1..=DEGREE {
            left[j] = x - self.knots[span + 1 - j];
            right[j] = self.knots[span + j] - x;
            let mut saved = 0.0;
            for r in 0..j {
                let denom = right[r + 1] + left[j - r];
                let temp = if denom.abs() < 1e-300 {
                    0.0
                } else {
                    n[r] / denom
                };
                n[r] = saved + right[r + 1] * temp;
                saved = left[j - r] * temp;
            }
            n[j] = saved;
        }
        n
    }

    /// Full-width (length `k`) basis row at `x` (zeros outside local support).
    fn eval_row(&self, x: f64) -> DVector<f64> {
        let x_clamped = x.clamp(self.knots[DEGREE], self.knots[self.k]);
        let span = self.find_span(x_clamped);
        let values = self.basis_funs(span, x_clamped);
        let mut row = DVector::<f64>::zeros(self.k);
        for (offset, &v) in values.iter().enumerate() {
            row[span - DEGREE + offset] = v;
        }
        row
    }

    /// The full `n x k` design matrix for a set of x values.
    fn eval_matrix(&self, xs: &[f64]) -> DMatrix<f64> {
        let mut mat = DMatrix::<f64>::zeros(xs.len(), self.k);
        for (i, &x) in xs.iter().enumerate() {
            let row = self.eval_row(x);
            mat.set_row(i, &row.transpose());
        }
        mat
    }
}

// ---------------------------------------------------------------------
// Difference penalty
// ---------------------------------------------------------------------

/// `S = D_m^T D_m`, the order-`m` difference penalty on a length-`k`
/// coefficient vector. `rank(S) = k - m`, `null(S)` = degree-`(m-1)`
/// polynomial sequences (dimension `m`).
fn difference_penalty(k: usize, m: usize) -> DMatrix<f64> {
    let d = difference_matrix(k, m);
    d.transpose() * d
}

fn difference_matrix(k: usize, m: usize) -> DMatrix<f64> {
    let mut current = DMatrix::<f64>::identity(k, k);
    let mut rows = k;
    for _ in 0..m {
        let d1 = first_difference_matrix(rows);
        current = d1 * current;
        rows -= 1;
    }
    current
}

fn first_difference_matrix(k: usize) -> DMatrix<f64> {
    let mut d = DMatrix::<f64>::zeros(k - 1, k);
    for i in 0..k - 1 {
        d[(i, i)] = -1.0;
        d[(i, i + 1)] = 1.0;
    }
    d
}

// ---------------------------------------------------------------------
// REML criterion and 1-D optimizer (§8.2: "smoothing parameter chosen by
// REML"; see spline.rs module tests for the derivation reference)
// ---------------------------------------------------------------------

/// Precomputed sufficient statistics for REML scoring at a fixed design
/// (X, y): X'X, X'y, and y'y. Re-used across every lambda trial.
struct RemlProblem {
    xtx: DMatrix<f64>,
    xty: DVector<f64>,
    yty: f64,
    n: usize,
    mp: usize,
    mr: usize,
}

impl RemlProblem {
    fn new(x: &DMatrix<f64>, y: &DVector<f64>, mp: usize, mr: usize) -> Self {
        Self {
            xtx: x.transpose() * x,
            xty: x.transpose() * y,
            yty: y.dot(y),
            n: x.nrows(),
            mp,
            mr,
        }
    }

    /// Solves the penalized normal equations at `lambda` and returns
    /// `(coefficients, log|A|)`.
    fn solve(&self, s: &DMatrix<f64>, lambda: f64) -> (DVector<f64>, f64) {
        let a = &self.xtx + lambda * s;
        let chol = a.clone().cholesky().unwrap_or_else(|| {
            // Extremely small/large lambda can make A numerically
            // ill-conditioned; nudge the diagonal and retry once.
            let bumped = &a + DMatrix::<f64>::identity(a.nrows(), a.ncols()) * 1e-10;
            bumped
                .cholesky()
                .expect("penalized normal equations should be SPD")
        });
        let beta = chol.solve(&self.xty);
        let log_det = 2.0 * chol.l().diagonal().iter().map(|v| v.ln()).sum::<f64>();
        (beta, log_det)
    }

    /// The profiled REML criterion (derived from the exact Gaussian
    /// marginal likelihood; see module docs), to be *minimised* over
    /// `log_lambda`. Also returns the fitted coefficients at this lambda.
    fn score(&self, s: &DMatrix<f64>, log_lambda: f64) -> (f64, DVector<f64>) {
        let lambda = log_lambda.exp();
        let (beta, log_det_a) = self.solve(s, lambda);
        let rss_pen = (self.yty - beta.dot(&self.xty)).max(1e-300);
        let nf = self.n as f64;
        let mpf = self.mp as f64;
        let mrf = self.mr as f64;
        let d = (nf - mpf) * rss_pen.ln() + log_det_a - mrf * lambda.ln();
        (d, beta)
    }
}

/// Golden-section search for the `log_lambda` minimising the REML score.
fn optimize_lambda(problem: &RemlProblem, s: &DMatrix<f64>) -> (f64, DVector<f64>, f64) {
    let gr = (5.0f64.sqrt() - 1.0) / 2.0; // ~0.618
    let (mut a, mut b) = (REML_LOG_LAMBDA_LO, REML_LOG_LAMBDA_HI);
    let mut c = b - gr * (b - a);
    let mut d = a + gr * (b - a);
    let (mut fc, _) = problem.score(s, c);
    let (mut fd, _) = problem.score(s, d);

    for _ in 0..REML_GOLDEN_ITERS {
        if fc < fd {
            b = d;
            d = c;
            fd = fc;
            c = b - gr * (b - a);
            fc = problem.score(s, c).0;
        } else {
            a = c;
            c = d;
            fc = fd;
            d = a + gr * (b - a);
            fd = problem.score(s, d).0;
        }
    }

    let log_lambda = (a + b) / 2.0;
    let (_, beta) = problem.score(s, log_lambda);
    (log_lambda.exp(), beta, log_lambda)
}

fn effective_df(problem: &RemlProblem, s: &DMatrix<f64>, lambda: f64) -> f64 {
    let a = &problem.xtx + lambda * s;
    let chol = a.clone().cholesky().unwrap_or_else(|| {
        let bumped = &a + DMatrix::<f64>::identity(a.nrows(), a.ncols()) * 1e-10;
        bumped.cholesky().expect("A should be SPD")
    });
    let a_inv = chol.inverse();
    (a_inv * &problem.xtx).trace()
}

fn fit_unconstrained(
    basis: &BSplineBasis,
    x: &[f64],
    y: &[f64],
    s: &DMatrix<f64>,
    mp: usize,
    mr: usize,
) -> (DVector<f64>, f64, f64) {
    let x_mat = basis.eval_matrix(x);
    let y_vec = DVector::from_row_slice(y);
    let problem = RemlProblem::new(&x_mat, &y_vec, mp, mr);
    let (lambda, beta, _) = optimize_lambda(&problem, s);
    let edf = effective_df(&problem, s, lambda);
    (beta, lambda, edf)
}

// ---------------------------------------------------------------------
// Monotonic SCOP-spline fit (PIRLS)
// ---------------------------------------------------------------------

/// `beta_1 = gamma_1`, `beta_j = beta_1 (+/-) sum_{i=2..j} exp(gamma_i)`
/// (Pya & Wood 2015). `increasing` selects `+` (mpi) vs `-` (mpd).
fn beta_from_gamma(gamma: &DVector<f64>, increasing: bool) -> DVector<f64> {
    let k = gamma.len();
    let mut beta = DVector::<f64>::zeros(k);
    beta[0] = gamma[0];
    let mut cum = 0.0;
    let sign = if increasing { 1.0 } else { -1.0 };
    for j in 1..k {
        cum += gamma[j].exp();
        beta[j] = gamma[0] + sign * cum;
    }
    beta
}

/// Jacobian d(beta)/d(gamma), k x k: column 0 is all-ones; column i>=1 has
/// `sign * exp(gamma_i)` in rows `>= i` and zero above.
fn jacobian_beta_gamma(gamma: &DVector<f64>, increasing: bool) -> DMatrix<f64> {
    let k = gamma.len();
    let mut j = DMatrix::<f64>::zeros(k, k);
    for row in 0..k {
        j[(row, 0)] = 1.0;
    }
    let sign = if increasing { 1.0 } else { -1.0 };
    for col in 1..k {
        let v = sign * gamma[col].exp();
        for row in col..k {
            j[(row, col)] = v;
        }
    }
    j
}

fn fit_monotonic(
    basis: &BSplineBasis,
    x: &[f64],
    y: &[f64],
    s: &DMatrix<f64>,
    mp: usize,
    mr: usize,
    increasing: bool,
) -> (DVector<f64>, f64, f64) {
    let k = basis.k;
    let x_mat = basis.eval_matrix(x);
    let y_vec = DVector::from_row_slice(y);

    // Initialise from a plausible monotone straight-line-ish curve.
    let mean_y = y_vec.mean();
    let range = (y_vec.max() - y_vec.min()).max(1e-6);
    let per_step_log = (range / (k as f64 - 1.0)).max(1e-6).ln();
    let mut gamma = DVector::from_element(k, per_step_log);
    gamma[0] = mean_y;

    let mut lambda = 1.0;
    let mut last_rss = f64::INFINITY;

    for _ in 0..PIRLS_MAX_ITERS {
        let beta = beta_from_gamma(&gamma, increasing);
        let jac = jacobian_beta_gamma(&gamma, increasing);
        let x_eff = &x_mat * &jac;
        let fitted = &x_mat * &beta;
        let z = (&y_vec - &fitted) + &x_eff * &gamma;

        let problem = RemlProblem::new(&x_eff, &z, mp, mr);
        let (lambda_opt, mut gamma_candidate, _) = optimize_lambda(&problem, s);
        lambda = lambda_opt;
        for i in 1..k {
            gamma_candidate[i] = gamma_candidate[i].clamp(-PIRLS_GAMMA_CLAMP, PIRLS_GAMMA_CLAMP);
        }

        // Step-halving safeguard: `z` is only a local linearization around
        // the current `gamma`, so the Gauss-Newton step can overshoot --
        // and because `beta_from_gamma` exponentiates `gamma`, an overshoot
        // makes `exp(gamma)` blow up, which then makes the *next*
        // iteration's normal equations catastrophically ill-conditioned
        // (observed: near-singular/SPD-failing fits on real, low-noise
        // rest data at high k). Halve the step toward `gamma_candidate`
        // until RSS actually improves, standard PIRLS practice (mgcv/scam)
        // for exactly this failure mode.
        let mut gamma_new = gamma_candidate;
        let mut rss: f64 = {
            let beta_new = beta_from_gamma(&gamma_new, increasing);
            let resid = &y_vec - &x_mat * &beta_new;
            resid.dot(&resid)
        };
        let mut halvings = 0;
        while rss > last_rss && halvings < PIRLS_MAX_STEP_HALVINGS {
            gamma_new = &gamma + 0.5 * (&gamma_new - &gamma);
            let beta_new = beta_from_gamma(&gamma_new, increasing);
            let resid = &y_vec - &x_mat * &beta_new;
            rss = resid.dot(&resid);
            halvings += 1;
        }

        let change = (&gamma_new - &gamma).norm() / gamma_new.norm().max(1e-12);
        gamma = gamma_new;

        if change < PIRLS_TOL || (last_rss - rss).abs() / last_rss.max(1e-300) < PIRLS_TOL {
            last_rss = rss;
            break;
        }
        last_rss = rss;
    }
    let _ = last_rss;

    let beta_final = beta_from_gamma(&gamma, increasing);
    let jac = jacobian_beta_gamma(&gamma, increasing);
    let x_eff = &x_mat * &jac;
    let problem = RemlProblem::new(&x_eff, &y_vec, mp, mr);
    let edf = effective_df(&problem, s, lambda);

    (beta_final, lambda, edf)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn linspace(a: f64, b: f64, n: usize) -> Vec<f64> {
        (0..n)
            .map(|i| a + (b - a) * (i as f64) / ((n - 1) as f64))
            .collect()
    }

    #[test]
    fn monotonic_fit_does_not_diverge_on_real_low_noise_rest_data() {
        // Real (Q, E0) pairs from a discharge branch of a user-supplied
        // fixture (large-format cell cycler export) that, pre-fix, made
        // fit_monotonic's PIRLS loop diverge: an unconstrained
        // Gauss-Newton step overshot, `beta_from_gamma`'s exp(gamma)
        // blew up, and the next iteration's normal equations either
        // failed the SPD Cholesky check outright or "succeeded" with a
        // wildly wrong fit (observed E0_smooth up to 74V for a cell
        // whose real voltage never exceeds ~4.3V). This data is
        // deterministic (near-noiseless OCV-vs-Q), so REML selects a
        // near-zero lambda (edf ~= k), which is exactly the regime that
        // triggers the divergence -- regular noisier data doesn't.
        let q: Vec<f64> = vec![-138.775921803201,-137.84234331199,-136.739575367654,-135.636807423318,-134.53403947898198,-133.431286793435,-132.328518849099,-131.225750904763,-130.122998219216,-129.02023027488,-127.917462330544,-126.814709644997,-125.711941700662,-124.60918138571999,-123.506421070779,-122.403660755837,-121.30089281150099,-120.19813249655999,-119.095364552224,-117.99260423728299,-116.88984392234099,-115.78707597800499,-114.684315663064,-113.58155534812299,-112.478787403787,-111.37602708884499,-110.273266773904,-109.170506458962,-108.06773851462599,-106.96497057028999,-105.862210255349,-104.75944231101299,-103.656681996072,-102.55392168112999,-101.451153736794,-100.34839342185299,-99.2456331069116,-98.14287279197019,-97.0401124770288,-95.93734453269279,-94.8345765883569,-93.73180864402089,-92.62904069968499,-91.52628038474359,-90.4235200698022,-89.32075975486079,-88.2179918105248,-87.11523149558339,-86.012471180642,-84.90971086570059,-83.8069581801538,-82.70419023581779,-81.6014299208764,-80.49866960593499,-79.39590929099359,-78.2931489760522,-77.19038103171619,-76.0876207167748,-74.98486040183339,-73.882100086892,-72.77933214255609,-71.6765718276147,-70.5738038832788,-69.4710435683373,-68.3682756240014,-67.26551530905999,-66.1627473647241,-65.05998704978269,-63.95722292014399,-62.85446260520259,-61.75169847556389,-60.64893434592519,-59.54617403098379,-58.44340990134519,-57.34064577170649,-56.23788164206779,-55.13511751242909,-54.03235719748769,-52.92959306784909,-51.82682893821039,-50.72406480857169,-49.62130449363029,-48.51854036399159,-47.415772419655696,-46.31301210471429,-45.21024797507559,-44.107483845436995,-43.004719715798295,-41.90195177146229,-40.799191456520894,-39.696427326882294,-38.593667011940894,-37.490902882302194,-36.388138752663494,-35.28537080832759,-34.18260667868889,-33.07984636374749,-31.977084141457492,-30.874320011818792,-29.771550160134193,-28.668787937844193,-27.566023808205493,-26.463259678566793,-25.360495548928192,-24.257735233986793,-23.154971104348093,-22.05220316001209,-20.949435215676193,-19.846672993386193,-18.743908863747492,-17.641148548806093,-16.538386326516093,-15.435625057900292,-14.332860928261692,-13.230096798622993,-12.127325039589792,-11.024563770974092,-9.921795826638112,-8.819034558022393,-7.716270428383722,-6.613506775582213,-5.510743122780692,-4.407981377327812,-3.3052162940148224,-2.2024538333062122,-1.0996938759926724];
        let e0: Vec<f64> = vec![2.8077007692393536,3.043157930782651,3.1776633167352957,3.26148828785909,3.3221488715746177,3.3686419910985648,3.4008856093619686,3.4202182149318383,3.4333173340122793,3.4434138783379904,3.451807096700927,3.4590636688250465,3.4654865896040583,3.4713939962985814,3.4775641101835957,3.485082439944576,3.4930588576545816,3.5005007235228316,3.507446265330591,3.5142336671688224,3.5211426581782597,3.528105724246283,3.5347218218597485,3.5407999525085807,3.5463845356989743,3.5515903303533327,3.5565454690432143,3.5613503373134434,3.5661139525253973,3.5708703309549983,3.5755647989910986,3.5800945466042418,3.5844146490756517,3.5885295284147705,3.592462196134964,3.5962276811480196,3.5998446652461404,3.6033262966324706,3.6066841543593426,3.609942843451864,3.6131236301570087,3.6162329897707917,3.619292268765095,3.622316152767812,3.6253144996477524,3.628301440162809,3.631292750328671,3.6342986560839425,3.6373363455816183,3.6404126321618935,3.6435486203924317,3.646759319798967,3.650058290602145,3.6534625501328066,3.6569983598376927,3.660672044919404,3.664518734814305,3.66854305811343,3.6727709730486904,3.6772270689853386,3.6819280895031903,3.6868843284629067,3.692112741876132,3.6976271215822494,3.703437572636249,3.709546419185967,3.715975836304549,3.722728701622921,3.729842638076709,3.737333804523164,3.745245352289079,3.7536174575042156,3.7624762731510897,3.771842239755897,3.7816750240382713,3.7918981743730207,3.802345572326778,3.8128685745301527,3.8233602222793706,3.833782172717068,3.8441102409698344,3.8543465136825046,3.864505578193989,3.8745963548930447,3.8846203303130418,3.894587626347318,3.9045014788323633,3.9143735507470065,3.924228036117715,3.934063781283436,3.943894958253189,3.9537485817438625,3.9636251388585366,3.9735453499123783,3.9835111020672778,3.9935242581903205,4.003587465470289,4.013705466904753,4.023858347030836,4.034044808192868,4.044240303655731,4.054434118995441,4.06459808844983,4.07472227590131,4.08478057508424,4.094767086086506,4.104671137933939,4.1144889852625495,4.124211383026701,4.133838102037999,4.143370529197626,4.152799209309394,4.162141317204721,4.171386181704147,4.1805390473994395,4.189638582514091,4.198707645796723,4.207817642302412,4.21704778630115,4.226495676120162,4.236304727703969,4.246611518368878,4.2575912214281635,4.269450046793577,4.2824863131813045,4.297214229511438];

        let config = SplineConfig {
            monotonic: true,
            direction: Direction::Increasing,
            k: 50,
            m: 1,
        };
        let (fitted, diag) = smooth_bspline_vec(&q, &e0, &config);
        let diag = diag.expect("should fit");
        assert_eq!(diag.direction_used, DirectionUsed::Increasing);

        let y_min = e0.iter().cloned().fold(f64::INFINITY, f64::min);
        let y_max = e0.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        // A generous slack band around the true data range -- catches the
        // observed order-of-magnitude divergence (fitted values reaching
        // 74V) without being brittle to normal spline overshoot at the
        // domain edges.
        let slack = (y_max - y_min) * 0.5;
        for (i, &f) in fitted.iter().enumerate() {
            assert!(
                f.is_finite() && f > y_min - slack && f < y_max + slack,
                "index {i}: fitted={f} outside plausible range [{}, {}]",
                y_min - slack,
                y_max + slack
            );
        }
    }

    #[test]
    fn unconstrained_fits_a_smooth_quadratic() {
        // y = x^2 over a moderate range: unconstrained ("ps") smoothing
        // should recover it closely with plenty of points and a generous k.
        let x = linspace(0.0, 10.0, 40);
        let y: Vec<f64> = x.iter().map(|&xi| xi * xi).collect();
        let config = SplineConfig {
            monotonic: false,
            direction: Direction::Automatic,
            k: 15,
            m: 2,
        };
        let (fitted, diag) = smooth_bspline_vec(&x, &y, &config);
        let diag = diag.expect("should fit");
        assert_eq!(diag.direction_used, DirectionUsed::NotApplicable);
        for i in 0..x.len() {
            let expected = x[i] * x[i];
            let scale = expected.abs().max(1.0);
            assert!(
                (fitted[i] - expected).abs() / scale < 0.01,
                "index {i}: fitted={}, expected={}",
                fitted[i],
                expected
            );
        }
    }

    #[test]
    fn monotonic_increasing_fits_a_known_monotone_function() {
        // A smooth, strictly increasing function: logistic-shaped curve.
        let x = linspace(-5.0, 5.0, 40);
        let y: Vec<f64> = x.iter().map(|&xi| 1.0 / (1.0 + (-xi).exp())).collect();
        let config = SplineConfig {
            monotonic: true,
            direction: Direction::Increasing,
            k: 12,
            m: 1,
        };
        let (fitted, diag) = smooth_bspline_vec(&x, &y, &config);
        let diag = diag.expect("should fit");
        assert_eq!(diag.direction_used, DirectionUsed::Increasing);
        // Recovery close to ground truth.
        for i in 0..x.len() {
            assert!(
                (fitted[i] - y[i]).abs() < 0.05,
                "index {i}: fitted={}, expected={}",
                fitted[i],
                y[i]
            );
        }
        // And the fitted curve is itself monotone non-decreasing (the whole
        // point of the shape constraint): x is already sorted ascending.
        for w in fitted.windows(2) {
            assert!(
                w[1] >= w[0] - 1e-9,
                "fitted values should be non-decreasing: {w:?}"
            );
        }
    }

    #[test]
    fn monotonic_decreasing_direction_forced() {
        let x = linspace(0.0, 10.0, 30);
        let y: Vec<f64> = x.iter().map(|&xi| 10.0 - xi + 0.01 * xi * xi).collect();
        let config = SplineConfig {
            monotonic: true,
            direction: Direction::Decreasing,
            k: 10,
            m: 1,
        };
        let (fitted, diag) = smooth_bspline_vec(&x, &y, &config);
        let diag = diag.expect("should fit");
        assert_eq!(diag.direction_used, DirectionUsed::Decreasing);
        for w in fitted.windows(2) {
            assert!(
                w[1] <= w[0] + 1e-9,
                "fitted values should be non-increasing: {w:?}"
            );
        }
    }

    #[test]
    fn automatic_direction_matches_median_pairwise_slope_sign() {
        // Mostly increasing trend with one outlier that would fool an
        // endpoint-slope heuristic but not the median-of-pairwise-slopes rule.
        let x = vec![0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0];
        let y = vec![0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, -100.0]; // last point crashes down
                                                                      // Endpoint slope (first->last) is strongly negative, but the median
                                                                      // of ALL pairwise slopes among the first 8 points (all +1) plus the
                                                                      // few negative ones from the outlier should still be positive.
        assert!(automatic_direction_is_increasing(&x, &y));
    }

    #[test]
    fn fewer_than_eight_distinct_x_returns_all_nan() {
        let x = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        let y = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        let config = SplineConfig {
            monotonic: false,
            direction: Direction::Automatic,
            k: 10,
            m: 1,
        };
        let (fitted, diag) = smooth_bspline_vec(&x, &y, &config);
        assert!(diag.is_none());
        assert!(fitted.iter().all(|v| v.is_nan()));
    }

    #[test]
    fn duplicate_x_values_are_averaged_before_fitting() {
        // x=5.0 appears twice (indices 5 and 10, y=5.0 and y=15.0 -> the
        // fit sees a single averaged point (5.0, 10.0) for that x).
        let mut x = vec![0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0];
        let mut y: Vec<f64> = x.clone();
        x.push(5.0);
        y.push(15.0);
        let config = SplineConfig {
            monotonic: false,
            direction: Direction::Automatic,
            k: 8,
            m: 1,
        };
        let (fitted, diag) = smooth_bspline_vec(&x, &y, &config);
        assert!(diag.is_some());
        // Both entries at x=5.0 (index 5 and the appended index 10) should
        // get the SAME predicted value, since prediction re-evaluates the
        // fitted curve at x=5.0 regardless of which row it came from.
        assert!((fitted[5] - fitted[10]).abs() < 1e-9);
    }

    #[test]
    fn k_is_clamped_to_n_distinct_minus_two() {
        // Exactly 8 distinct x values, k requested far larger than n-2=6.
        let x = linspace(0.0, 7.0, 8);
        let y: Vec<f64> = x.iter().map(|&xi| xi * 2.0).collect();
        let config = SplineConfig {
            monotonic: false,
            direction: Direction::Automatic,
            k: 50,
            m: 1,
        };
        let (fitted, diag) = smooth_bspline_vec(&x, &y, &config);
        let diag = diag.expect("should fit with the clamped k");
        assert_eq!(diag.k_effective, 6); // min(50, 8-2)
        assert!(fitted.iter().all(|v| v.is_finite()));
    }

    #[test]
    fn difference_penalty_has_expected_rank_and_nullspace() {
        // Order-1 penalty on k=6: rank 5, and a constant vector is in the null space.
        let s = difference_penalty(6, 1);
        let ones = DVector::from_element(6, 1.0);
        let quad_form = (ones.transpose() * &s * &ones)[(0, 0)];
        assert!(
            quad_form.abs() < 1e-9,
            "constant vector should be in the null space of D1'D1"
        );

        // Order-2 penalty: a linear ramp should also be in the null space.
        let s2 = difference_penalty(6, 2);
        let ramp = DVector::from_vec(vec![0.0, 1.0, 2.0, 3.0, 4.0, 5.0]);
        let quad_form2 = (ramp.transpose() * &s2 * &ramp)[(0, 0)];
        assert!(
            quad_form2.abs() < 1e-9,
            "linear ramp should be in the null space of D2'D2"
        );
    }
}
