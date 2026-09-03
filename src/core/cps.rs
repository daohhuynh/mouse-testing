//! Clicks per second.
//!
//! Sustained rate is the figure that means something: it is what you can hold
//! for the length of the test. Peak is the best one-second window, which on any
//! short run is mostly luck about where the window fell, so it is reported
//! second and never as the headline.

/// Sustained and peak rate from press timestamps.
///
/// `duration_ns` is the length of the timed window, not the span between the
/// first and last press: dividing by the span would reward someone who clicked
/// three times quickly and then stopped.
pub fn rates(press_times_ns: &[u64], duration_ns: u64, peak_window_ns: u64) -> (f64, f64) {
    if duration_ns == 0 {
        return (0.0, 0.0);
    }
    let sustained = press_times_ns.len() as f64 * 1e9 / duration_ns as f64;

    // Peak is the busiest window of its length anywhere in the run, found by
    // sliding over the presses rather than over time, so cost is linear.
    let mut peak_count = 0usize;
    let mut lo = 0usize;
    for hi in 0..press_times_ns.len() {
        while press_times_ns[hi].saturating_sub(press_times_ns[lo]) > peak_window_ns {
            lo += 1;
        }
        peak_count = peak_count.max(hi - lo + 1);
    }
    let peak = peak_count as f64 * 1e9 / peak_window_ns as f64;
    (sustained, peak)
}

/// A finished run, kept in the session history.
#[derive(Clone, Debug)]
pub struct Run {
    /// The technique label. Recorded with the result and nothing else: it does
    /// not change how anything is measured.
    pub mode: String,
    pub button: u8,
    pub duration_s: f64,
    pub presses: usize,
    pub sustained_cps: f64,
    pub peak_cps: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uniform(hz: f64, secs: f64) -> Vec<u64> {
        let n = (hz * secs) as usize;
        (0..n).map(|i| (i as f64 / hz * 1e9) as u64).collect()
    }

    #[test]
    fn a_steady_rate_gives_the_same_sustained_and_peak() {
        let t = uniform(10.0, 10.0);
        let (sus, peak) = rates(&t, 10_000_000_000, 1_000_000_000);
        assert!((sus - 10.0).abs() < 0.01, "sustained {sus}");
        assert!((peak - 10.0).abs() < 1.01, "peak {peak}");
    }

    #[test]
    fn a_short_burst_lifts_peak_but_not_sustained() {
        // Twenty clicks in one second, then nothing for nine.
        let mut t: Vec<u64> = (0..20).map(|i| i * 50_000_000).collect();
        t.extend((0..10).map(|i| 2_000_000_000 + i * 900_000_000));
        let (sus, peak) = rates(&t, 10_000_000_000, 1_000_000_000);
        assert!(sus < 4.0, "sustained {sus} should stay low");
        assert!(peak >= 20.0, "peak {peak} should catch the burst");
    }

    #[test]
    fn rate_is_over_the_whole_window_not_the_span_of_presses() {
        // Three fast clicks then a long pause must not read as a high rate.
        let t = vec![0u64, 100_000_000, 200_000_000];
        let (sus, _) = rates(&t, 10_000_000_000, 1_000_000_000);
        assert!((sus - 0.3).abs() < 0.001, "sustained {sus}");
    }

    #[test]
    fn no_presses_is_zero_not_a_division_by_zero() {
        let (sus, peak) = rates(&[], 5_000_000_000, 1_000_000_000);
        assert_eq!(sus, 0.0);
        assert_eq!(peak, 0.0);
    }
}
