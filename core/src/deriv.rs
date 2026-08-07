//! Local moving-window polynomial derivative (§8.3 of ICI_WEB_SPEC.md).
//! Faithful port of `local_poly_derivative()` (ici_analysis.R lines 219-257).

use nalgebra::{DMatrix, DVector};

/// For each point, fits a degree-`degree` raw polynomial to a `window`-sized
/// local window (odd, `>= degree + 1`) and takes the linear coefficient as
/// the derivative estimate. End points reuse the terminal window rather
/// than shrinking it (§8.3). Returns NaN wherever `x`/`y` aren't both
/// finite, or everywhere if fewer than `window` points are valid.
pub fn local_poly_derivative(x: &[f64], y: &[f64], window: usize, degree: usize) -> Vec<f64> {
    assert_eq!(x.len(), y.len());
    assert!(
        window % 2 == 1 && window > degree,
        "window must be odd and at least degree + 1"
    );
    let n = x.len();
    let mut out = vec![f64::NAN; n];

    let valid: Vec<usize> = (0..n)
        .filter(|&i| x[i].is_finite() && y[i].is_finite())
        .collect();
    let nv = valid.len();
    if nv < window {
        return out;
    }

    let mut order_index: Vec<usize> = (0..nv).collect();
    order_index.sort_by(|&a, &b| x[valid[a]].partial_cmp(&x[valid[b]]).unwrap());
    let xv: Vec<f64> = order_index.iter().map(|&k| x[valid[k]]).collect();
    let yv: Vec<f64> = order_index.iter().map(|&k| y[valid[k]]).collect();

    let half_window = window / 2;
    let mut derivative = vec![f64::NAN; nv];

    for i in 0..nv {
        let start = (i as isize - half_window as isize)
            .max(0)
            .min((nv - window) as isize) as usize;
        let z: Vec<f64> = (start..start + window).map(|j| xv[j] - xv[i]).collect();
        let yw: Vec<f64> = (start..start + window).map(|j| yv[j]).collect();

        if let Some(coeffs) = poly_fit_raw(&z, &yw, degree) {
            if coeffs.len() >= 2 && coeffs[1].is_finite() {
                derivative[i] = coeffs[1];
            }
        }
    }

    let mut derivative_unsorted = vec![f64::NAN; nv];
    for (pos, &orig_k) in order_index.iter().enumerate() {
        derivative_unsorted[orig_k] = derivative[pos];
    }
    for (k, &i) in valid.iter().enumerate() {
        out[i] = derivative_unsorted[k];
    }
    out
}

/// OLS fit of `y ~ 1 + z + z^2 + ... + z^degree` (R's `poly(z, degree, raw=TRUE)`),
/// via the normal equations. `None` on a singular design (mirrors R's
/// `tryCatch(..., error = function(e) NULL)`).
fn poly_fit_raw(z: &[f64], y: &[f64], degree: usize) -> Option<Vec<f64>> {
    let n = z.len();
    let p = degree + 1;
    let mut x = DMatrix::<f64>::zeros(n, p);
    for i in 0..n {
        let mut power = 1.0;
        for j in 0..p {
            x[(i, j)] = power;
            power *= z[i];
        }
    }
    let y_vec = DVector::from_row_slice(y);
    let xtx = x.transpose() * &x;
    let xty = x.transpose() * &y_vec;
    let chol = xtx.cholesky()?;
    let coeffs = chol.solve(&xty);
    Some(coeffs.iter().copied().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovers_analytic_derivative_of_a_cubic() {
        // y = x^3 - 2x^2 + x -> y' = 3x^2 - 4x + 1. A degree-3 raw
        // polynomial fit over any window containing >= 4 points recovers
        // this exactly (up to floating point), since the true function IS
        // a cubic.
        let x: Vec<f64> = (0..21).map(|i| (i as f64 - 10.0) * 0.5).collect();
        let y: Vec<f64> = x
            .iter()
            .map(|&xi| xi.powi(3) - 2.0 * xi.powi(2) + xi)
            .collect();
        let deriv = local_poly_derivative(&x, &y, 7, 3);
        for i in 0..x.len() {
            let expected = 3.0 * x[i].powi(2) - 4.0 * x[i] + 1.0;
            assert!(
                (deriv[i] - expected).abs() < 1e-8,
                "index {i} (x={}): got {}, expected {}",
                x[i],
                deriv[i],
                expected
            );
        }
    }

    #[test]
    fn end_points_reuse_the_terminal_window() {
        // With window=5 on 10 points, index 0 and index 1 should use the
        // SAME window [0..5) (both clamped to start=0), so their local fits
        // -- and thus the coefficient extraction point -- differ only in
        // where z=0 sits, not in which points are included.
        let x: Vec<f64> = (0..10).map(|i| i as f64).collect();
        let y: Vec<f64> = x.iter().map(|&xi| xi * xi).collect(); // y=x^2, y'=2x exactly recoverable
        let deriv = local_poly_derivative(&x, &y, 5, 3);
        for i in 0..x.len() {
            let expected = 2.0 * x[i];
            assert!(
                (deriv[i] - expected).abs() < 1e-8,
                "index {i}: got {}, expected {}",
                deriv[i],
                expected
            );
        }
    }

    #[test]
    fn fewer_than_window_valid_points_returns_all_nan() {
        let x = vec![1.0, 2.0, 3.0];
        let y = vec![1.0, 4.0, 9.0];
        let deriv = local_poly_derivative(&x, &y, 5, 3);
        assert!(deriv.iter().all(|v| v.is_nan()));
    }

    #[test]
    fn non_finite_points_are_excluded_and_left_nan() {
        let x = vec![0.0, 1.0, 2.0, 3.0, f64::NAN, 5.0, 6.0, 7.0];
        let y = vec![0.0, 1.0, 4.0, 9.0, 16.0, 25.0, 36.0, 49.0];
        let deriv = local_poly_derivative(&x, &y, 5, 3);
        assert!(deriv[4].is_nan()); // the NaN x row itself
        assert!(deriv[0].is_finite());
        assert!(deriv[7].is_finite());
    }

    #[test]
    fn unsorted_input_still_produces_correct_per_point_derivatives() {
        let x = vec![3.0, 1.0, 4.0, 0.0, 2.0, 6.0, 5.0];
        let y: Vec<f64> = x.iter().map(|&xi| xi * xi).collect();
        let deriv = local_poly_derivative(&x, &y, 5, 3);
        for i in 0..x.len() {
            assert!(
                (deriv[i] - 2.0 * x[i]).abs() < 1e-8,
                "index {i} (x={}): got {}",
                x[i],
                deriv[i]
            );
        }
    }
}
