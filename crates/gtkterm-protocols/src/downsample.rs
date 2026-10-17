//! Largest-Triangle-Three-Buckets downsampling for plotting and for MCP series excerpts.

/// Reduce `points` (sorted by x) to at most `threshold` points, keeping the visual shape.
/// Returns indices into `points`, always including the first and last point.
pub fn lttb_indices(points: &[(f64, f64)], threshold: usize) -> Vec<usize> {
    let n = points.len();
    if threshold >= n || threshold == 0 {
        return (0..n).collect();
    }
    if threshold < 3 {
        return if threshold == 1 {
            vec![n - 1]
        } else {
            vec![0, n - 1]
        };
    }
    let mut out = Vec::with_capacity(threshold);
    out.push(0);
    let bucket = (n - 2) as f64 / (threshold - 2) as f64;
    let mut a = 0usize;
    for i in 0..threshold - 2 {
        let start = (i as f64 * bucket) as usize + 1;
        let end = (((i + 1) as f64 * bucket) as usize + 1).min(n - 1);
        // Average of the next bucket.
        let next_start = end;
        let next_end = (((i + 2) as f64 * bucket) as usize + 1).min(n);
        let next = &points[next_start..next_end.max(next_start + 1).min(n)];
        let (avg_x, avg_y) = {
            let len = next.len() as f64;
            let (sx, sy) = next.iter().fold((0.0, 0.0), |(sx, sy), &(x, y)| (sx + x, sy + y));
            (sx / len, sy / len)
        };
        let (ax, ay) = points[a];
        let mut best = start;
        let mut best_area = -1.0;
        for (j, &(x, y)) in points.iter().enumerate().take(end).skip(start) {
            let area = ((ax - avg_x) * (y - ay) - (ax - x) * (avg_y - ay)).abs();
            if area > best_area {
                best_area = area;
                best = j;
            }
        }
        out.push(best);
        a = best;
    }
    out.push(n - 1);
    out
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    #[test]
    fn small_inputs_unchanged() {
        let pts: Vec<_> = (0..5).map(|i| (f64::from(i), 0.0)).collect();
        assert_eq!(lttb_indices(&pts, 10), vec![0, 1, 2, 3, 4]);
        assert_eq!(lttb_indices(&pts, 2), vec![0, 4]);
        assert_eq!(lttb_indices(&[], 3), Vec::<usize>::new());
    }

    #[test]
    fn keeps_spike() {
        let mut pts: Vec<_> = (0..1000).map(|i| (f64::from(i), 0.0)).collect();
        pts[500].1 = 100.0;
        let idx = lttb_indices(&pts, 50);
        assert_eq!(idx.len(), 50);
        assert!(idx.contains(&500), "the spike must survive downsampling");
    }

    proptest! {
        #[test]
        fn indices_sorted_bounded_and_endpoints_kept(ys in proptest::collection::vec(-1e3f64..1e3, 3..500), t in 3usize..100) {
            let pts: Vec<_> = ys.iter().enumerate().map(|(i, &y)| (i as f64, y)).collect();
            let idx = lttb_indices(&pts, t);
            prop_assert_eq!(idx.len(), t.min(pts.len()));
            prop_assert_eq!(idx[0], 0);
            prop_assert_eq!(*idx.last().unwrap(), pts.len() - 1);
            prop_assert!(idx.windows(2).all(|w| w[0] < w[1]));
        }
    }
}
