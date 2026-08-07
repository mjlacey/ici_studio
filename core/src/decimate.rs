//! LTTB (Largest-Triangle-Three-Buckets) decimation for Plot 1 (§11.1:
//! "Decimate with LTTB to ~4,000 points for display, re-decimating on
//! zoom"). Standard algorithm (Steinarsson, "Downsampling Time Series for
//! Visual Representation"), operating on indices so callers can reuse the
//! selection across parallel series (e.g. voltage and current sharing one
//! time axis).

/// Returns the indices of `x`/`y` selected by LTTB, always including the
/// first and last point, ascending order. `threshold < 3` degenerates to
/// just the first/last point(s) (there's no triangle to form); `threshold`
/// at or above `x.len()` returns every index unchanged.
pub fn lttb(x: &[f64], y: &[f64], threshold: usize) -> Vec<usize> {
    let n = x.len();
    debug_assert_eq!(n, y.len());

    if n == 0 {
        return Vec::new();
    }
    if threshold == 0 {
        return Vec::new();
    }
    if threshold >= n || threshold < 3 {
        // Not enough room for a middle bucket structure; degrade gracefully
        // rather than producing a misleading "decimation".
        return match threshold {
            0 => Vec::new(),
            1 => vec![0],
            2 => vec![0, n - 1],
            _ => (0..n).collect(),
        };
    }

    let mut sampled = Vec::with_capacity(threshold);
    sampled.push(0usize);

    let bucket_size = (n - 2) as f64 / (threshold - 2) as f64;
    let mut a = 0usize;

    for i in 0..(threshold - 2) {
        let avg_range_start = (((i + 1) as f64) * bucket_size).floor() as usize + 1;
        let avg_range_end = ((((i + 2) as f64) * bucket_size).floor() as usize + 1).min(n);
        let avg_range_start = avg_range_start.min(avg_range_end.saturating_sub(1));
        let avg_range_len = avg_range_end.saturating_sub(avg_range_start).max(1);

        let mut avg_x = 0.0;
        let mut avg_y = 0.0;
        for j in avg_range_start..avg_range_end {
            avg_x += x[j];
            avg_y += y[j];
        }
        avg_x /= avg_range_len as f64;
        avg_y /= avg_range_len as f64;

        let range_offs = ((i as f64) * bucket_size).floor() as usize + 1;
        let range_to = (((i + 1) as f64) * bucket_size).floor() as usize + 1;
        let range_to = range_to.min(n - 1).max(range_offs + 1);

        let point_a_x = x[a];
        let point_a_y = y[a];

        let mut max_area = -1.0f64;
        let mut max_area_index = range_offs;

        for j in range_offs..range_to {
            let area = ((point_a_x - avg_x) * (y[j] - point_a_y)
                - (point_a_x - x[j]) * (avg_y - point_a_y))
                .abs()
                * 0.5;
            if area > max_area {
                max_area = area;
                max_area_index = j;
            }
        }

        sampled.push(max_area_index);
        a = max_area_index;
    }

    sampled.push(n - 1);
    sampled
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn always_includes_first_and_last() {
        let x: Vec<f64> = (0..1000).map(|i| i as f64).collect();
        let y: Vec<f64> = x.iter().map(|v| v.sin()).collect();
        let idx = lttb(&x, &y, 100);
        assert_eq!(*idx.first().unwrap(), 0);
        assert_eq!(*idx.last().unwrap(), 999);
    }

    #[test]
    fn returns_requested_count() {
        let x: Vec<f64> = (0..1000).map(|i| i as f64).collect();
        let y: Vec<f64> = x.iter().map(|v| v.cos()).collect();
        let idx = lttb(&x, &y, 250);
        assert_eq!(idx.len(), 250);
    }

    #[test]
    fn indices_are_strictly_ascending() {
        let x: Vec<f64> = (0..500).map(|i| i as f64).collect();
        let y: Vec<f64> = x.iter().map(|v| (v * 0.1).sin()).collect();
        let idx = lttb(&x, &y, 60);
        for w in idx.windows(2) {
            assert!(w[0] < w[1], "indices should be strictly increasing: {w:?}");
        }
    }

    #[test]
    fn threshold_above_length_returns_everything() {
        let x: Vec<f64> = (0..10).map(|i| i as f64).collect();
        let y = x.clone();
        let idx = lttb(&x, &y, 1000);
        assert_eq!(idx, (0..10).collect::<Vec<_>>());
    }

    #[test]
    fn threshold_below_three_degenerates_gracefully() {
        let x: Vec<f64> = (0..10).map(|i| i as f64).collect();
        let y = x.clone();
        assert_eq!(lttb(&x, &y, 0), Vec::<usize>::new());
        assert_eq!(lttb(&x, &y, 1), vec![0]);
        assert_eq!(lttb(&x, &y, 2), vec![0, 9]);
    }

    #[test]
    fn empty_input_returns_empty() {
        assert_eq!(lttb(&[], &[], 100), Vec::<usize>::new());
    }

    #[test]
    fn preserves_a_sharp_spike_better_than_uniform_sampling() {
        // A flat line with one sharp spike in the middle: LTTB should keep
        // a point at/near the spike since it maximises triangle area,
        // unlike naive every-Nth-point sampling which could miss it.
        let n = 300;
        let mut y = vec![0.0f64; n];
        y[150] = 100.0;
        let x: Vec<f64> = (0..n).map(|i| i as f64).collect();
        let idx = lttb(&x, &y, 30);
        assert!(
            idx.iter().any(|&i| (i as isize - 150).abs() <= 2),
            "spike near index 150 should survive decimation: {idx:?}"
        );
    }
}
