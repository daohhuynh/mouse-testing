//! Report-interval analysis.
//!
//! Two things make this harder than "count reports and divide".
//!
//! A mouse does not report when it has nothing to report. Idle gaps and slots
//! whose true motion rounded to zero counts are indistinguishable from dropped
//! reports, so an interval is only judged when both reports bracketing it carry
//! enough counts that the slots around them must have produced something.
//! Without that guard a perfectly clean 1 kHz stream reads as several percent
//! dropped, and a pause between two swipes reads as a burst of drops.
//!
//! And the interval distribution is a mixture: the nominal cluster, integer
//! multiples of it where slots were lost, and long idle gaps. The mean is
//! hopeless and the median breaks once drops or idle dominate. Drops and idle
//! only ever add mass above the nominal, never below, so the mode is the right
//! target and is found from the bottom of the distribution upward.

use std::f64;

/// One input report, reduced to what interval analysis needs.
#[derive(Clone, Copy, Debug)]
pub struct Report {
    pub t_ns: u64,
    /// Motion magnitude carried by this report, in device counts.
    pub counts: i32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum IntervalClass {
    Normal,
    /// Arrived sooner than the nominal interval allows.
    Fast,
    /// `k` slots' worth of time passed, so `k - 1` reports were lost.
    Drop(u32),
    /// Late, but not by a clean multiple of the interval.
    Slow,
    /// Not judgeable: outside a motion span, or longer than `k_max` slots.
    Idle,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Verdict {
    Pass,
    Warn,
    Fail,
    /// The measurement cannot support a conclusion. Never a failure of the
    /// mouse, and never reported as one.
    Inconclusive,
}

impl Verdict {
    /// Combine two verdicts, keeping the one that most demands attention.
    ///
    /// `Inconclusive` deliberately beats `Pass`: a section that could not
    /// measure one of its parts has not passed, and letting a `Pass` mask it
    /// would be exactly the silent zero the whole app is built to avoid.
    pub fn worst(self, other: Verdict) -> Verdict {
        use Verdict::*;
        match (self, other) {
            (Fail, _) | (_, Fail) => Fail,
            (Warn, _) | (_, Warn) => Warn,
            (Inconclusive, _) | (_, Inconclusive) => Inconclusive,
            _ => Pass,
        }
    }

    pub fn level(self) -> crate::ui::theme::Level {
        use crate::ui::theme::Level;
        match self {
            Verdict::Pass => Level::Pass,
            Verdict::Warn => Level::Warn,
            Verdict::Fail => Level::Fail,
            Verdict::Inconclusive => Level::Off,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct PollConfig {
    /// Largest run of lost slots still classified as a drop rather than idle.
    pub k_max: u32,
    /// Minimum relative tolerance, whatever the measured jitter.
    pub tol_floor: f64,
    /// Counts both bracketing reports must carry for an interval to be judged.
    /// Three guarantees the local motion rate is at least one count per slot,
    /// so every slot in that neighbourhood had something to report.
    pub min_counts_per_report: i32,
    pub drop_warn: f64,
    pub drop_fail: f64,
    pub slow_warn: f64,
    pub slow_fail: f64,
    /// Below this many judgeable intervals, refuse to give a verdict.
    pub min_analyzable: usize,
}

impl Default for PollConfig {
    fn default() -> Self {
        PollConfig {
            k_max: 5,
            tol_floor: 0.15,
            min_counts_per_report: 3,
            // At 1 kHz, 0.1% is one lost report per second and invisible; 1% is
            // ten per second, which is the micro-stutter people actually notice.
            drop_warn: 0.001,
            drop_fail: 0.01,
            slow_warn: 0.005,
            slow_fail: 0.02,
            min_analyzable: 200,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct PollResult {
    pub verdict: Verdict,
    pub nominal_ns: f64,
    pub nominal_hz: f64,
    /// Nominal snapped to a standard rate, for display only.
    pub snapped_hz: Option<f64>,
    pub jitter_sigma_ns: f64,
    pub tol_rel: f64,
    /// False when jitter is so large that an interval cannot be told apart from
    /// two intervals, which makes drop counting meaningless.
    pub multiple_classification_valid: bool,
    pub nominal_reliable: bool,
    pub n_intervals: usize,
    pub n_analyzable: usize,
    pub n_normal: usize,
    pub n_fast: usize,
    pub n_slow: usize,
    pub n_idle: usize,
    pub n_drop_events: usize,
    pub n_dropped_slots: usize,
    pub drop_rate: f64,
    pub slow_rate: f64,
    /// Slots that had to carry a report: the denominator of both rates,
    /// and the sample size any statement about them rests on.
    pub expected_slots: f64,
    /// Judgeable reports per second of judgeable time.
    pub effective_hz: f64,
    pub p50_ns: f64,
    pub p99_ns: f64,
    pub p999_ns: f64,
    pub max_ns: f64,
    pub min_ns: f64,
    pub note: &'static str,
    /// Interval histogram for display: (bin centre ns, count).
    pub histogram: Vec<(f64, u32)>,
}

impl Default for Verdict {
    fn default() -> Self {
        Verdict::Inconclusive
    }
}

/// 95% two-sided normal quantile.
const Z_95: f64 = 1.959_963_984_540_054;

/// Wilson score interval for a proportion.
///
/// Wilson rather than the textbook normal interval because both rates here sit
/// at or very near zero on working hardware, and the normal interval has zero
/// width at exactly zero. That would declare a run settled after a handful of
/// reports, which is the opposite of what an early stop is for.
pub fn wilson_bounds(hits: f64, n: f64, z: f64) -> (f64, f64) {
    if n <= 0.0 {
        return (0.0, 1.0);
    }
    let p = (hits / n).clamp(0.0, 1.0);
    let z2 = z * z;
    let denom = 1.0 + z2 / n;
    let centre = (p + z2 / (2.0 * n)) / denom;
    let half = (z / denom) * (p * (1.0 - p) / n + z2 / (4.0 * n * n)).sqrt();
    ((centre - half).max(0.0), (centre + half).min(1.0))
}

fn band(x: f64, warn: f64, fail: f64) -> u8 {
    if x >= fail {
        2
    } else if x >= warn {
        1
    } else {
        0
    }
}

/// Whether more measuring could still change the verdict.
///
/// The question this section answers is not "what is the drop rate" but "which
/// side of the thresholds is it on", and that can be settled long before the
/// rate itself is known precisely. So a run is finished once the whole
/// confidence interval for each rate lies inside one band: no continuation of
/// it could cross a threshold, and swiping on is asking a question that is
/// already answered.
///
/// It is deliberately conservative. It refuses while the analysis itself is
/// refusing, so an early stop can never manufacture a verdict the section
/// would not otherwise give, and a rate sitting on a threshold keeps the run
/// going rather than freezing an arbitrary side of it.
pub fn verdict_settled(r: &PollResult, cfg: &PollConfig) -> bool {
    if r.n_analyzable < cfg.min_analyzable
        || !r.nominal_reliable
        || !r.multiple_classification_valid
        || r.expected_slots <= 0.0
    {
        return false;
    }
    let pinned = |hits: f64, warn: f64, fail: f64| {
        let (lo, hi) = wilson_bounds(hits, r.expected_slots, Z_95);
        band(lo, warn, fail) == band(hi, warn, fail)
    };
    pinned(r.n_dropped_slots as f64, cfg.drop_warn, cfg.drop_fail)
        && pinned(r.n_slow as f64, cfg.slow_warn, cfg.slow_fail)
}

pub const STANDARD_RATES_HZ: [f64; 9] = [
    125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0, 32000.0,
];

pub fn snap_rate(hz: f64, tol: f64) -> Option<f64> {
    STANDARD_RATES_HZ
        .iter()
        .copied()
        .find(|&r| (hz - r).abs() / r <= tol)
}

fn percentile(sorted: &[f64], q: f64) -> f64 {
    if sorted.is_empty() {
        return f64::NAN;
    }
    if sorted.len() == 1 {
        return sorted[0];
    }
    // Linear interpolation between order statistics, the same convention numpy
    // calls type 7.
    let pos = q * (sorted.len() - 1) as f64;
    let lo = pos.floor() as usize;
    let hi = pos.ceil() as usize;
    if lo == hi {
        sorted[lo]
    } else {
        sorted[lo] + (sorted[hi] - sorted[lo]) * (pos - lo as f64)
    }
}

fn median_of(v: &[f64]) -> f64 {
    if v.is_empty() {
        return f64::NAN;
    }
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    percentile(&s, 0.5)
}

/// Median absolute deviation, scaled so it estimates the standard deviation of
/// a normal distribution.
fn mad_sigma(v: &[f64]) -> f64 {
    if v.len() < 2 {
        return 0.0;
    }
    let m = median_of(v);
    let dev: Vec<f64> = v.iter().map(|x| (x - m).abs()).collect();
    1.4826 * median_of(&dev)
}

/// The nominal interval, found as the mode of the low end of the distribution.
///
/// The fifth percentile is guaranteed to sit inside the nominal cluster unless
/// more than 95% of intervals are drops, which cannot happen, and that is what
/// makes the rest robust.
pub fn nominal_interval_ns(intervals: &[f64]) -> f64 {
    if intervals.is_empty() {
        return f64::NAN;
    }
    let mut sorted = intervals.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let p5 = percentile(&sorted, 0.05);
    if !(p5 > 0.0) {
        return f64::NAN;
    }

    let lo = 0.5 * p5;
    let hi = 3.0 * p5;
    let bin_w = 0.02 * p5;
    let nbins = (((hi - lo) / bin_w).ceil() as usize).clamp(1, 4096);
    let mut bins = vec![0u32; nbins];
    for &iv in intervals {
        if iv >= lo && iv < hi {
            let b = (((iv - lo) / bin_w) as usize).min(nbins - 1);
            bins[b] += 1;
        }
    }
    let modal = bins
        .iter()
        .enumerate()
        .max_by_key(|(_, &c)| c)
        .map(|(i, _)| i)
        .unwrap_or(0);
    let centre = lo + (modal as f64 + 0.5) * bin_w;

    let near: Vec<f64> = intervals
        .iter()
        .copied()
        .filter(|&iv| (iv - centre).abs() <= 0.20 * centre)
        .collect();
    if near.is_empty() {
        centre
    } else {
        median_of(&near)
    }
}

/// Spans in which the device was moving hard enough that every polling slot
/// must have carried at least one count.
fn judgeable(reports: &[Report], min_counts: i32) -> Vec<bool> {
    reports.iter().map(|r| r.counts >= min_counts).collect()
}

pub fn analyze(reports: &[Report], cfg: &PollConfig) -> PollResult {
    let mut out = PollResult {
        note: "",
        ..Default::default()
    };
    if reports.len() < 2 {
        out.note = "not enough reports";
        return out;
    }

    // Timestamps must be monotonic. Rather than trusting that, drop any
    // out-of-order pair: an unguarded subtraction here panics in debug and
    // produces an astronomically large interval in release.
    let mut intervals: Vec<f64> = Vec::with_capacity(reports.len());
    let mut judge: Vec<bool> = Vec::with_capacity(reports.len());
    let hot = judgeable(reports, cfg.min_counts_per_report);
    for i in 1..reports.len() {
        let (a, b) = (reports[i - 1].t_ns, reports[i].t_ns);
        if b <= a {
            continue;
        }
        intervals.push((b - a) as f64);
        judge.push(hot[i - 1] && hot[i]);
    }
    if intervals.is_empty() {
        out.note = "no usable intervals";
        return out;
    }
    out.n_intervals = intervals.len();

    let nominal = nominal_interval_ns(&intervals);
    if !(nominal > 0.0) {
        out.note = "could not establish a nominal interval";
        return out;
    }
    out.nominal_ns = nominal;
    out.nominal_hz = 1e9 / nominal;
    out.snapped_hz = snap_rate(out.nominal_hz, 0.02);

    let judged: Vec<f64> = intervals
        .iter()
        .zip(&judge)
        .filter(|(_, &j)| j)
        .map(|(&iv, _)| iv)
        .collect();
    out.n_analyzable = judged.len();

    // Tolerance derived from the data, two passes: estimate jitter from what
    // currently looks normal, widen, re-estimate.
    let mut tol = cfg.tol_floor;
    let mut sigma = 0.0;
    for _ in 0..2 {
        let normal: Vec<f64> = judged
            .iter()
            .copied()
            .filter(|&iv| (iv / nominal - 1.0).abs() <= tol)
            .collect();
        sigma = mad_sigma(&normal);
        tol = cfg.tol_floor.max(3.0 * sigma / nominal);
    }
    out.jitter_sigma_ns = sigma;
    out.tol_rel = tol;
    // Jitter over k skipped slots adds in quadrature, so the window for k = 2
    // is the first one that can overlap its neighbour.
    out.multiple_classification_valid = tol * 2f64.sqrt() < 0.5;
    out.nominal_reliable = sigma < 0.10 * nominal;

    let mut expected_slots = 0f64;
    let mut judged_time = 0f64;
    for (&iv, &j) in intervals.iter().zip(&judge) {
        let r = iv / nominal;
        let class = classify(r, tol, cfg.k_max, j);
        match class {
            IntervalClass::Normal => {
                out.n_normal += 1;
                expected_slots += 1.0;
                judged_time += iv;
            }
            IntervalClass::Fast => {
                out.n_fast += 1;
                expected_slots += 1.0;
                judged_time += iv;
            }
            IntervalClass::Drop(k) => {
                out.n_drop_events += 1;
                out.n_dropped_slots += (k - 1) as usize;
                expected_slots += k as f64;
                judged_time += iv;
            }
            IntervalClass::Slow => {
                out.n_slow += 1;
                expected_slots += 1.0;
                judged_time += iv;
            }
            IntervalClass::Idle => out.n_idle += 1,
        }
    }

    out.expected_slots = expected_slots;
    out.drop_rate = if expected_slots > 0.0 {
        out.n_dropped_slots as f64 / expected_slots
    } else {
        0.0
    };
    out.slow_rate = if expected_slots > 0.0 {
        out.n_slow as f64 / expected_slots
    } else {
        0.0
    };
    out.effective_hz = if judged_time > 0.0 {
        (out.n_normal + out.n_fast + out.n_slow + out.n_drop_events) as f64 * 1e9 / judged_time
    } else {
        0.0
    };

    let mut sorted = intervals.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    out.min_ns = sorted[0];
    out.p50_ns = percentile(&sorted, 0.50);
    out.p99_ns = percentile(&sorted, 0.99);
    out.p999_ns = percentile(&sorted, 0.999);
    out.max_ns = *sorted.last().unwrap();
    out.histogram = histogram(&intervals, nominal, cfg.k_max);

    out.verdict = verdict(&out, cfg);
    out.note = note_for(&out, cfg);
    out
}

fn classify(r: f64, tol: f64, k_max: u32, judgeable: bool) -> IntervalClass {
    if !judgeable {
        return IntervalClass::Idle;
    }
    if (r - 1.0).abs() <= tol {
        return IntervalClass::Normal;
    }
    if r < 1.0 - tol {
        return IntervalClass::Fast;
    }
    for k in 2..=k_max {
        let tol_k = tol * (k as f64).sqrt();
        // Once the window is half a slot wide the multiples overlap and the
        // classification stops meaning anything.
        if tol_k >= 0.5 {
            break;
        }
        if (r - k as f64).abs() <= tol_k {
            return IntervalClass::Drop(k);
        }
    }
    if r > k_max as f64 {
        IntervalClass::Idle
    } else {
        IntervalClass::Slow
    }
}

fn histogram(intervals: &[f64], nominal: f64, k_max: u32) -> Vec<(f64, u32)> {
    let hi = nominal * (k_max as f64 + 1.0);
    let nbins = 120usize;
    let w = hi / nbins as f64;
    let mut bins = vec![0u32; nbins];
    for &iv in intervals {
        let b = ((iv / w) as usize).min(nbins - 1);
        bins[b] += 1;
    }
    bins.iter()
        .enumerate()
        .map(|(i, &c)| ((i as f64 + 0.5) * w, c))
        .collect()
}

fn verdict(r: &PollResult, cfg: &PollConfig) -> Verdict {
    if r.n_analyzable < cfg.min_analyzable
        || !r.multiple_classification_valid
        || !r.nominal_reliable
    {
        return Verdict::Inconclusive;
    }
    if r.drop_rate >= cfg.drop_fail || r.slow_rate >= cfg.slow_fail {
        Verdict::Fail
    } else if r.drop_rate >= cfg.drop_warn || r.slow_rate >= cfg.slow_warn {
        Verdict::Warn
    } else {
        Verdict::Pass
    }
}

fn note_for(r: &PollResult, cfg: &PollConfig) -> &'static str {
    if r.n_analyzable < cfg.min_analyzable {
        return "Not enough motion to judge. Swipe faster and for longer, so that every \
                polling slot has to carry at least one count: a mouse sends nothing when \
                it has nothing to send, and a silent slot cannot be told apart from a \
                lost one.";
    }
    if !r.nominal_reliable {
        return "Interval jitter is too large a fraction of the interval to establish a \
                nominal rate. Read the distribution, not the rate.";
    }
    if !r.multiple_classification_valid {
        return "Timing jitter is comparable to one polling interval, so a late report \
                cannot be told apart from a lost one. Read the distribution, not the drop \
                count. This is expected when timestamps come from the OS rather than the \
                device, and at very high report rates.";
    }
    if r.drop_rate >= cfg.drop_warn && r.drop_rate < 0.002 {
        return "A drop rate this close to the threshold is within the detector's own \
                noise on clean hardware. Re-run before concluding anything.";
    }
    ""
}

/// Reports per second over a trailing window, for a live readout.
pub fn windowed_rate(times_ns: &[u64], window_ns: u64) -> f64 {
    let last = match times_ns.last() {
        Some(&t) => t,
        None => return 0.0,
    };
    let cutoff = last.saturating_sub(window_ns);
    let n = times_ns.iter().rev().take_while(|&&t| t >= cutoff).count();
    if n < 2 {
        return 0.0;
    }
    let span = last.saturating_sub(times_ns[times_ns.len() - n]);
    if span == 0 {
        0.0
    } else {
        (n - 1) as f64 * 1e9 / span as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::sim::{generate, Rng, StreamSpec};

    fn run(spec: &StreamSpec, seed: u64) -> (PollResult, f64) {
        let mut rng = Rng::new(seed);
        let truth = generate(spec, &mut rng);
        let res = analyze(&truth.reports, &PollConfig::default());
        (res, truth.drop_rate())
    }

    #[test]
    fn nominal_interval_survives_what_the_median_does_not() {
        // A per-slot drop probability p makes an interval of k slots appear
        // with probability p^(k-1)(1-p), so the nominal cluster stays the
        // single largest even when most reports are lost. That is precisely
        // why the mode works here and the median does not.
        let truth_ns = 1_000_000.0;
        let p = 0.60;
        let mut rng = Rng::new(7);
        let mut ivs = Vec::new();
        for _ in 0..20_000 {
            let mut k = 1.0;
            while rng.unit() < p {
                k += 1.0;
            }
            ivs.push(k * truth_ns + 3_000.0 * rng.normal());
        }
        let mode = nominal_interval_ns(&ivs);
        let median = median_of(&ivs);
        let mean = ivs.iter().sum::<f64>() / ivs.len() as f64;
        assert!(
            (mode - truth_ns).abs() / truth_ns < 0.01,
            "robust mode {mode} missed the truth {truth_ns}"
        );
        // Recorded so the choice of estimator rests on evidence, not assertion.
        assert!(
            (median - truth_ns).abs() / truth_ns > 0.5,
            "median {median} was expected to be badly wrong at a 60% drop rate"
        );
        assert!(
            (mean - truth_ns).abs() / truth_ns > 0.5,
            "mean {mean} was expected to be badly wrong"
        );
    }

    #[test]
    fn a_clean_stream_reports_no_drops() {
        for (rate, seed) in [(125.0, 1u64), (1000.0, 2), (8000.0, 3)] {
            let spec = StreamSpec {
                rate_hz: rate,
                motion: vec![(0.0, 3.0)],
                total_s: 3.0,
                drop_prob: 0.0,
                jitter_ns: 2_000.0f64.min(0.02 * 1e9 / rate),
                counts_per_slot: 5.0,
            };
            let (r, _) = run(&spec, seed);
            assert!(
                (r.nominal_hz - rate).abs() / rate < 0.02,
                "{rate} Hz measured as {}",
                r.nominal_hz
            );
            assert_eq!(r.snapped_hz, Some(rate));
            assert!(
                r.drop_rate < 0.001,
                "clean {rate} Hz stream reported {:.4}% drops",
                r.drop_rate * 100.0
            );
            assert_eq!(r.verdict, Verdict::Pass, "{rate} Hz: {}", r.note);
        }
    }

    fn settled_fixture(dropped: usize, slow: usize, slots: f64, judged: usize) -> PollResult {
        PollResult {
            n_analyzable: judged,
            nominal_reliable: true,
            multiple_classification_valid: true,
            expected_slots: slots,
            n_dropped_slots: dropped,
            n_slow: slow,
            ..Default::default()
        }
    }

    #[test]
    fn a_clean_run_settles_only_once_the_interval_clears_the_threshold() {
        let cfg = PollConfig::default();
        // Nothing dropped or late, but too few slots for the interval to
        // exclude the 0.1% warning line, so the answer is not yet pinned.
        assert!(!verdict_settled(&settled_fixture(0, 0, 1_000.0, 1_000), &cfg));
        // The same clean run, carried far enough that it is.
        assert!(verdict_settled(&settled_fixture(0, 0, 20_000.0, 20_000), &cfg));
    }

    #[test]
    fn an_early_stop_can_never_invent_a_verdict_the_section_would_refuse() {
        let cfg = PollConfig::default();
        let mut r = settled_fixture(0, 0, 20_000.0, 20_000);
        assert!(verdict_settled(&r, &cfg));

        // Every reason the analysis itself refuses must also stop the clock
        // from stopping, or the run would end on a result that is not offered.
        r.n_analyzable = cfg.min_analyzable - 1;
        assert!(!verdict_settled(&r, &cfg));
        r.n_analyzable = 20_000;
        r.nominal_reliable = false;
        assert!(!verdict_settled(&r, &cfg));
        r.nominal_reliable = true;
        r.multiple_classification_valid = false;
        assert!(!verdict_settled(&r, &cfg));
    }

    #[test]
    fn a_rate_sitting_on_a_threshold_keeps_the_run_going() {
        let cfg = PollConfig::default();
        // Exactly the warn line. The interval straddles it however long this
        // runs, so it must never freeze an arbitrary side of the boundary.
        let slots = 100_000.0;
        let on_the_line = (slots * cfg.drop_warn) as usize;
        assert!(!verdict_settled(
            &settled_fixture(on_the_line, 0, slots, 100_000),
            &cfg
        ));
    }

    #[test]
    fn a_clearly_failing_run_settles_too_rather_than_swiping_forever() {
        let cfg = PollConfig::default();
        // 5% dropped is far above the 1% failure line: the answer is known,
        // and a bad mouse should not cost more swiping than a good one.
        assert!(verdict_settled(&settled_fixture(500, 0, 10_000.0, 10_000), &cfg));
    }

    #[test]
    fn lateness_alone_can_hold_the_run_open() {
        let cfg = PollConfig::default();
        // Drops pinned at zero, but the late rate sits on its own warn line,
        // so the combined verdict is still undecided.
        let slots = 100_000.0;
        let on_the_line = (slots * cfg.slow_warn) as usize;
        assert!(!verdict_settled(
            &settled_fixture(0, on_the_line, slots, 100_000),
            &cfg
        ));
    }

    #[test]
    fn wilson_upper_bound_shrinks_with_the_sample_and_never_leaves_zero_flat() {
        // The whole point of Wilson here: at zero observed events the interval
        // must still have width, or a one-second run would look conclusive.
        let (lo, hi) = wilson_bounds(0.0, 100.0, Z_95);
        // Exactly zero in algebra, a rounding crumb above it in floating point.
        assert!(lo < 1e-12, "lo was {lo}");
        assert!(hi > 0.03, "hi was {hi}");
        let (_, hi_big) = wilson_bounds(0.0, 20_000.0, Z_95);
        assert!(hi_big < 0.001, "hi was {hi_big}");
        assert!(hi_big < hi);
    }

    #[test]
    fn measured_drop_rate_tracks_the_truth() {
        for (p, seed) in [
            (0.001, 11u64),
            (0.01, 12),
            (0.05, 13),
            (0.30, 14),
        ] {
            let spec = StreamSpec {
                rate_hz: 1000.0,
                motion: vec![(0.0, 20.0)],
                total_s: 20.0,
                drop_prob: p,
                jitter_ns: 2_000.0,
                counts_per_slot: 5.0,
            };
            let (r, truth) = run(&spec, seed);
            let err = (r.drop_rate - truth).abs();
            assert!(
                err < 0.15 * truth.max(0.002) + 0.0015,
                "truth {:.4}% measured {:.4}%",
                truth * 100.0,
                r.drop_rate * 100.0
            );
        }
    }

    #[test]
    fn an_idle_gap_between_swipes_is_not_a_burst_of_drops() {
        let spec = StreamSpec {
            rate_hz: 1000.0,
            // Two swipes with three seconds of stillness between them.
            motion: vec![(0.0, 2.0), (5.0, 7.0)],
            total_s: 7.0,
            drop_prob: 0.0,
            jitter_ns: 2_000.0,
            // Fast enough that no slot can round to zero counts, so this test
            // isolates the idle gap from the sub-count effect measured below.
            counts_per_slot: 20.0,
        };
        let (r, _) = run(&spec, 21);
        assert_eq!(
            r.n_drop_events, 0,
            "the three second pause between swipes was counted as {} drop events",
            r.n_drop_events
        );
        assert_eq!(r.verdict, Verdict::Pass, "{}", r.note);
    }

    #[test]
    fn a_three_second_gap_classifies_as_idle_not_as_a_drop() {
        assert_eq!(
            classify(3000.0, 0.15, 5, true),
            IntervalClass::Idle,
            "a gap of three thousand slots must never be called a drop"
        );
    }

    #[test]
    fn sub_count_slots_leave_a_small_bounded_false_positive_rate() {
        // A sensor that carries sub-count motion forward emits nothing for a
        // slot whose motion rounded to zero, and the reports on either side can
        // still be moving fast enough to pass the motion gate. That silence is
        // genuinely indistinguishable from a lost report, so the detector has
        // an irreducible false positive rate near its own warning threshold.
        // Measuring it here keeps the limit honest instead of implied.
        let mut worst = 0.0f64;
        for seed in 0..8u64 {
            let spec = StreamSpec {
                rate_hz: 1000.0,
                motion: vec![(0.0, 10.0)],
                total_s: 10.0,
                drop_prob: 0.0,
                jitter_ns: 2_000.0,
                counts_per_slot: 5.0,
            };
            let (r, _) = run(&spec, 900 + seed);
            worst = worst.max(r.drop_rate);
        }
        assert!(
            worst < 0.002,
            "false positive drop rate on a clean stream reached {:.4}%, which is \
             high enough to mislead",
            worst * 100.0
        );
        assert!(
            worst > 0.0,
            "expected a small nonzero false positive rate; if this is now zero the \
             simulator stopped modelling sub-count slots"
        );
    }

    #[test]
    fn slow_motion_is_refused_rather_than_called_a_failure() {
        // Under one count per slot, the sensor legitimately stays silent for
        // whole slots. Those silences must not be counted as drops.
        let spec = StreamSpec {
            rate_hz: 1000.0,
            motion: vec![(0.0, 5.0)],
            total_s: 5.0,
            drop_prob: 0.0,
            jitter_ns: 2_000.0,
            counts_per_slot: 0.4,
        };
        let (r, _) = run(&spec, 31);
        assert_ne!(
            r.verdict,
            Verdict::Fail,
            "a slow clean swipe was reported as a failure: {:.3}% drops",
            r.drop_rate * 100.0
        );
        assert!(
            r.n_analyzable < PollConfig::default().min_analyzable,
            "expected too little judgeable motion, got {}",
            r.n_analyzable
        );
        assert!(r.note.contains("Swipe faster"));
    }

    #[test]
    fn host_timestamp_jitter_makes_it_refuse_instead_of_inventing_a_number() {
        for (rate, seed) in [(1000.0, 41u64), (8000.0, 42)] {
            let spec = StreamSpec {
                rate_hz: rate,
                motion: vec![(0.0, 5.0)],
                total_s: 5.0,
                drop_prob: 0.0,
                // What a timestamp taken in userspace under load looks like.
                jitter_ns: 300_000.0,
                counts_per_slot: 5.0,
            };
            let (r, _) = run(&spec, seed);
            assert_eq!(
                r.verdict,
                Verdict::Inconclusive,
                "{rate} Hz with 300 us of host jitter produced a verdict anyway \
                 (drop rate {:.3}%)",
                r.drop_rate * 100.0
            );
        }
    }

    #[test]
    fn out_of_order_timestamps_do_not_panic() {
        let reports = vec![
            Report { t_ns: 1_000_000, counts: 5 },
            Report { t_ns: 3_000_000, counts: 5 },
            // Goes backwards, as a badly behaved source could.
            Report { t_ns: 2_000_000, counts: 5 },
            Report { t_ns: 4_000_000, counts: 5 },
        ];
        let r = analyze(&reports, &PollConfig::default());
        assert_eq!(r.verdict, Verdict::Inconclusive);
    }

    #[test]
    fn windowed_rate_matches_a_known_cadence() {
        let times: Vec<u64> = (0..1000).map(|i| i as u64 * 1_000_000).collect();
        let hz = windowed_rate(&times, 200_000_000);
        assert!((hz - 1000.0).abs() < 1.0, "windowed rate {hz}");
    }
}
