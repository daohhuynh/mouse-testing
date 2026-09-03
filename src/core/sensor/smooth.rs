//! Detector 4: motion smoothing / interpolation (firmware low-pass).
//!
//! MODEL. Firmware smoothing is an EMA on the velocity before quantisation:
//!     y_k = (1-alpha) * y_{k-1} + alpha * x_k
//! which is AR(1) with pole (1-alpha) and time constant
//!     tau = -dt / ln(1-alpha)      <=>     alpha = 1 - exp(-dt/tau)
//!
//! THREE INDEPENDENT ESTIMATORS OF alpha, all validated against a known alpha:
//!
//! (a) STOP DECAY. After the hand stops, x_k = 0 and y_k = y_0 (1-alpha)^k. Regress
//!     ln|d_k| on k over the tail; slope = ln(1-alpha). This is the most interpretable
//!     and the hardest to fake, but it dies for alpha >~ 0.7 because the tail truncates
//!     to < 3 integer counts.
//!
//! (b) LAG-1 AUTOCORRELATION OF THE HIGH-PASS RESIDUAL. Raw sensor noise is white, so
//!     after removing the hand's motion (a moving-average high-pass at ~15-30 Hz) the
//!     residual of a clean device has rho1 <= 0. It is <= 0, not == 0, for two reasons we
//!     correct for: the MA subtraction itself induces rho1 ~ -1/w, and the
//!     residual-accumulator quantiser is a first-order sigma-delta whose error is
//!     high-pass shaped (negatively correlated). Both push the clean case NEGATIVE, so
//!     any positive rho1 is evidence of smoothing and the test is conservative.
//!         alpha_hat = 1 - rho1_corrected
//!
//! (c) SPECTRAL ATTENUATION. Welch PSD of the same residual. For AR(1) driven by white
//!     noise, PSD(f) proportional to alpha^2 / |1 - (1-alpha) e^{-j 2 pi f dt}|^2.
//!     Take the ratio of power in a low band to a high band and invert analytically.
//!     Reported as `hf_attenuation_db`: dB of extra roll-off from the low band to the
//!     Nyquist band relative to flat.
//!
//! THE CONFOUND: genuinely smooth human motion. Handled by only ever looking at the
//! residual ABOVE ~15 Hz. Voluntary human motor control is band-limited to roughly 5 Hz;
//! physiological tremor peaks at 8-12 Hz; nothing a hand does puts correlated structure
//! at 100-500 Hz. So a positive rho1 in that band is firmware, not muscle.
//! Second guard on the stop-decay test: human deceleration takes 50-150 ms and is not
//! log-linear, so we require BOTH tau < `max_human_tau_ms` AND a good log-linear fit
//! (r2 >= 0.8) before calling a tail "exponential".

use super::seg::{dominant_span, MotionGate};
use super::types::{Report, Verdict, NS};
use super::util::{autocorr, band_power, highpass2, ols, percentile, welch_psd};

#[derive(Copy, Clone, Debug)]
pub struct SmoothConfig {
    /// High-pass corner, Hz. 20 Hz sits above tremor (8-12 Hz) with margin.
    pub hp_corner_hz: f64,
    /// A tail this fast cannot be a decelerating hand.
    pub max_human_tau_ms: f64,
    /// rho1 bands on the corrected high-pass residual.
    pub rho1_warn: f64,
    pub rho1_fail: f64,
    /// Number of post-stop reports required to attempt the decay fit.
    pub min_tail: usize,
    /// Post-hard-stop tail bands, milliseconds.
    pub tail_warn_ms: f64,
    pub tail_fail_ms: f64,
    /// Set false when the capture was NOT an abrupt stop; the tail test is then skipped.
    pub expect_hard_stop: bool,
    /// Above this many counts per report the motion signal itself leaks into the
    /// high-frequency band and inflates rho1. Measured clean rho1 at 1 kHz / 1600 CPI:
    /// 10 IPS -> -0.05, 20 IPS -> -0.02, 40 IPS -> +0.10..+0.33. Instruct the user to
    /// glide at a MODERATE speed (15-25 IPS); above the cap we report Inconclusive
    /// rather than a number we know is contaminated.
    pub max_glide_counts_per_report: f64,
}

impl Default for SmoothConfig {
    fn default() -> Self {
        SmoothConfig {
            // 60 Hz, NOT 20 Hz. Physiological tremor at 8-12 Hz leaks through any
            // practical zero-phase high-pass placed at 20 Hz, and a leaked 10 Hz
            // sinusoid at 1 kHz has a lag-1 autocorrelation of 0.998, so the leak alone
            // put clean rho1 at +0.31 and failed 100% of clean mice. Measured corner
            // sweep (clean / alpha=0.5): 20 Hz -> +0.49/0.86, 40 Hz -> +0.06/0.84,
            // 60 Hz -> -0.01/0.84, 120 Hz -> -0.05/0.75. 60 Hz kills the leak while
            // keeping essentially all of the AR(1) signal.
            hp_corner_hz: 60.0,
            // 20 ms: a hand cannot arrest a swipe with a 20 ms exponential; measured
            // voluntary stop transients are 50-150 ms and are ramp-shaped, not
            // log-linear. Anything faster than 20 ms and log-linear is a filter.
            max_human_tau_ms: 20.0,
            // rho1 = 0.10 <=> alpha = 0.90 <=> tau = 0.43 report periods: a filter this
            // weak is imperceptible, so it is the Pass/Warn edge.
            // rho1 = 0.30 <=> alpha = 0.70 <=> tau = 0.83 report periods; at 1 kHz that
            // is ~0.8 ms of added lag, and it grows fast below this, so it is Warn/Fail.
            // Measured null across sensor noise 0.25-2.0 counts and glide 10-20 IPS:
            // clean rho1 in [-0.20, +0.10]. Filtered: alpha 0.7 -> 0.35-0.71,
            // alpha 0.3 -> 0.69-0.95. 0.15 / 0.35 sit in the gap.
            rho1_warn: 0.15,
            rho1_fail: 0.35,
            min_tail: 4,
            // At 1 kHz an honest residual accumulator holds < 1 count, so it can emit at
            // most 1-2 extra reports = 1-2 ms. 4 ms is double that. 8 ms of continued
            // motion after the mouse physically stopped is the "floaty/mushy" feel users
            // complain about and is unambiguously stored filter energy.
            tail_warn_ms: 4.0,
            tail_fail_ms: 8.0,
            expect_hard_stop: true,
            max_glide_counts_per_report: 45.0,
        }
    }
}

#[derive(Clone, Debug)]
pub struct SmoothResult {
    pub verdict: Verdict,
    pub report_rate_hz: f64,
    pub n_reports: usize,
    // (a) stop decay
    pub tail_len: usize,
    /// Milliseconds of continued motion after the physical hard stop. THE headline
    /// statistic for this test: an honest device's residual accumulator can hold at most
    /// one count, so it can emit at most one or two extra reports.
    pub tail_ms: f64,
    pub tail_reports: usize,
    /// Magnitude the tail search stopped at, in counts. Reported so a long tail
    /// on a noisy sensor can be told from a long tail on a quiet one.
    pub tail_floor_counts: f64,
    pub decay_tau_ms: f64,
    pub decay_r2: f64,
    pub alpha_from_decay: f64,
    // (b) autocorrelation (see note on the quantisation-noise floor in the module doc)
    pub rho1_raw: f64,
    pub rho1_corrected: f64,
    pub alpha_from_rho1: f64,
    pub hp_window: usize,
    // (c) spectrum
    pub hf_attenuation_db: f64,
    /// Diagnostic only. The AR(1) inversion is defeated by the sigma-delta quantisation
    /// noise floor and pins at 1.0; do not report it as an alpha.
    pub alpha_from_psd: f64,
    /// Median |delta| per report inside the glide. Drives the speed guard.
    pub median_counts_per_report: f64,
    /// Reports in the evenly spaced run the correlation was measured over. Less
    /// than `n_reports` whenever the device dropped reports inside the stroke.
    pub n_uniform: usize,
    pub note: &'static str,
}

/// Estimate the report rate inside the stroke.
///
/// NOT the median. The median interval is the wrong estimator whenever the
/// device drops reports: at a 60% drop rate it lands on twice the true
/// interval, because most surviving gaps really are two slots wide. This value
/// scales both the recovered time constant and the high-pass corner, so a 2x
/// error here is a 2x error in the answer. `nominal_interval_ns` finds the
/// modal slot instead and holds to within 0.03% at the same drop rate.
fn report_rate(seg: &[Report]) -> f64 {
    if seg.len() < 3 { return 0.0; }
    let iv: Vec<f64> = seg
        .windows(2)
        .map(|w| w[1].t_ns.saturating_sub(w[0].t_ns) as f64)
        .collect();
    let ns = crate::core::polling::nominal_interval_ns(&iv);
    if ns > 0.0 { NS / ns } else { 0.0 }
}

/// Magnitude below which a report is indistinguishable from the sensor's own
/// idle noise, taken from the capture's own stationary lead-in where there is
/// one. Without this the tail measurement has no floor and any sensor that
/// keeps talking while the hand is still (which is every sensor) reads as an
/// arbitrarily long interpolation tail.
fn idle_floor(reports: &[Report], span_start: usize) -> f64 {
    let mags: Vec<f64> = reports[..span_start]
        .iter()
        .filter(|r| r.is_moving())
        .map(|r| r.mag())
        .collect();
    if mags.len() < 8 {
        2.0
    } else {
        // p90 of the idle chatter, so an occasional larger sample does not
        // extend the tail, with the same 2-count hard floor underneath.
        (1.5 * percentile(&mags, 0.90)).max(2.0)
    }
}

pub fn analyze_smoothing(reports: &[Report], cfg: &SmoothConfig) -> SmoothResult {
    let mut out = SmoothResult {
        verdict: Verdict::Inconclusive, report_rate_hz: 0.0, n_reports: 0,
        tail_len: 0, tail_ms: 0.0, tail_reports: 0, tail_floor_counts: f64::NAN, decay_tau_ms: f64::NAN, decay_r2: f64::NAN, alpha_from_decay: f64::NAN,
        rho1_raw: f64::NAN, rho1_corrected: f64::NAN, alpha_from_rho1: f64::NAN, hp_window: 0,
        hf_attenuation_db: f64::NAN, alpha_from_psd: f64::NAN,
        median_counts_per_report: f64::NAN, n_uniform: 0, note: "",
    };
    if !super::types::is_monotonic(reports) {
        out.note = super::types::NOT_MONOTONIC; return out;
    }
    let Some((a, b)) = dominant_span(reports, MotionGate::default()) else {
        out.note = "no motion span"; return out;
    };
    let seg = &reports[a..b];
    out.n_reports = seg.len();
    out.report_rate_hz = report_rate(seg);
    if seg.len() < 256 || out.report_rate_hz <= 0.0 {
        out.note = "need >= 256 reports in one continuous motion span"; return out;
    }
    let dt_s = 1.0 / out.report_rate_hz;

    // ---------- (a) stop decay ----------
    // PROTOCOL: this test is only meaningful after an ABRUPT stop -- glide at a steady
    // speed into the edge of the mousepad or a book, so the true motion goes to zero in
    // one report period. That removes the "was that the hand or the firmware
    // decelerating?" confound PHYSICALLY. Trying to remove it statistically does not
    // work: on a minimum-jerk (soft) stop the hand's own decay fits a log-linear model
    // with r2 = 0.94 and tau = 3.6 ms, which is indistinguishable from a real alpha=0.5
    // filter. Measured, both at 3.4-3.6 ms. So we require the hard stop.
    //
    // The tail must be measured on the POLL-SLOT GRID, not on the array index, because a
    // residual-accumulator quantiser dribbles isolated +-1 counts for a long time after
    // the filter state has decayed below one count, and those dribble reports are
    // separated by multi-slot gaps. Indexing the array instead of the clock made the
    // measured tail length collapse to 1 report for alpha <= 0.1 -- i.e. it failed
    // hardest on the most heavily filtered devices.
    {
        let mags: Vec<f64> = seg.iter().map(|r| r.mag()).collect();
        let peak = percentile(&mags, 0.95);
        if peak >= 6.0 {
            // Last report still at half the cruise magnitude is the physical
            // stop. Searched inside the stroke, not the whole capture: `peak`
            // comes from the stroke, so a later movement (repositioning the
            // mouse, a second swipe) would otherwise re-anchor the stop to
            // motion that is not the one under test.
            let t_stop = seg.iter().rev()
                .find(|r| r.mag() >= 0.5 * peak)
                .map(|r| r.t_ns);
            if let Some(t_stop) = t_stop {
                // The tail is the CONTIGUOUS decaying run after the stop. Two
                // things end it, and without both this statistic is unbounded:
                //
                //   a gap, because stored filter energy comes out on every
                //   slot for as long as it lasts, whereas a stationary
                //   sensor's sub-count dribble is sparse and irregular; and
                //
                //   the idle floor, because once the emitted motion is down in
                //   the sensor's own noise there is no filter energy left to
                //   measure.
                //
                // Neither guard was here originally, and the cost was not
                // subtle: with idle jitter of 2 counts/second, which this
                // app's own drift detector calls healthy, an UNFILTERED mouse
                // measured a 47.7 ms tail and failed. At 40 counts/second it
                // failed every single time.
                let floor = idle_floor(reports, a);
                let max_gap_ns = (5.0 * dt_s * NS) as u64;
                let mut after: Vec<&super::types::Report> = Vec::new();
                let mut last_t = t_stop;
                for r in reports.iter().filter(|r| r.t_ns > t_stop && r.mag() > 0.0) {
                    if r.t_ns.saturating_sub(last_t) > max_gap_ns { break; }
                    if r.mag() < floor { break; }
                    last_t = r.t_ns;
                    after.push(r);
                }
                out.tail_floor_counts = floor;
                out.tail_reports = after.len();
                out.tail_ms = after.last()
                    .map(|r| r.t_ns.saturating_sub(t_stop) as f64 / 1.0e6).unwrap_or(0.0);
                // log-linear fit on the slot grid, excluding the +-1 dribble
                let fit: Vec<(f64, f64)> = after.iter()
                    .filter(|r| r.mag() >= 2.0)
                    .map(|r| ((r.t_ns.saturating_sub(t_stop) as f64 / NS) / dt_s, r.mag().ln()))
                    .collect();
                if fit.len() >= cfg.min_tail {
                    let xs: Vec<f64> = fit.iter().map(|p| p.0).collect();
                    let ys: Vec<f64> = fit.iter().map(|p| p.1).collect();
                    let (_a0, slope, r2) = ols(&xs, &ys);
                    out.decay_r2 = r2;
                    if slope < 0.0 && r2 >= 0.90 {
                        out.alpha_from_decay = 1.0 - slope.exp();
                        out.decay_tau_ms = -dt_s / slope * 1000.0;
                        out.tail_len = fit.len();
                    } else { out.tail_len = fit.len(); }
                }
            }
        }
    }

    // ---------- (b) lag-1 autocorrelation of the high-pass residual ----------
    //
    // ON A UNIFORM GRID ONLY. Both the high-pass and the autocorrelation index
    // the array, which silently assumes every neighbouring pair of reports is
    // one report period apart. A motion span does not guarantee that: the
    // device drops reports, and the span's own edges can straddle a stop, so a
    // few sparse post-stop reports end up index-adjacent to the last fast
    // reports of the glide. The moving average then averages 30-count reports
    // together with +-1 chatter, and the step that produces dominates the
    // correlation. Measured on an UNFILTERED mouse whose span overran the stop
    // by 30 ms: rho1 jumped from its -0.30 baseline to +0.64, a clear Fail on
    // hardware with nothing wrong with it.
    let uni = uniform_run(seg, NS / out.report_rate_hz);
    let seg = &seg[uni.0..uni.1];
    out.n_uniform = seg.len();
    if seg.len() < 128 {
        out.note = "no long enough run of evenly spaced reports to measure correlation";
        out.verdict = Verdict::Inconclusive;
        return out;
    }

    // Use the dominant axis; it carries the signal and therefore the filter's effect.
    let sx: f64 = seg.iter().map(|r| r.dx as f64).sum::<f64>().abs();
    let sy: f64 = seg.iter().map(|r| r.dy as f64).sum::<f64>().abs();
    let d: Vec<f64> = if sx >= sy { seg.iter().map(|r| r.dx as f64).collect() }
                      else { seg.iter().map(|r| r.dy as f64).collect() };
    // Moving-average length whose first null sits at hp_corner_hz.
    let w = ((out.report_rate_hz / cfg.hp_corner_hz).round() as usize).clamp(3, 501);

    let (res, weff) = highpass2(&d, w);
    out.hp_window = weff;
    let r1 = autocorr(&res, 1);
    out.rho1_raw = r1;
    // Remove the correlation the high-pass itself imposes on white input, computed
    // EXACTLY from the filter kernel rather than approximated.
    out.rho1_corrected = r1 - super::util::highpass2_white_rho1(weff);
    out.alpha_from_rho1 = (1.0 - out.rho1_corrected).clamp(0.0, 1.0);

    // ---------- (c) spectral attenuation ----------
    {
        let seglen = 512usize.min(res.len().next_power_of_two() / 2).max(64);
        let (f, p) = welch_psd(&res, out.report_rate_hz, seglen);
        if !f.is_empty() {
            let nyq = out.report_rate_hz / 2.0;
            // "low" band = just above the high-pass corner, "high" band = top quarter.
            let lo0 = (cfg.hp_corner_hz * 1.5).min(nyq * 0.10);
            let lo1 = nyq * 0.20;
            let hi0 = nyq * 0.75;
            let hi1 = nyq * 0.98;
            let pl = band_power(&f, &p, lo0, lo1) / (lo1 - lo0).max(1e-9);
            let ph = band_power(&f, &p, hi0, hi1) / (hi1 - hi0).max(1e-9);
            if pl > 0.0 && ph > 0.0 {
                let ratio = ph / pl; // < 1 means high frequencies are attenuated
                out.hf_attenuation_db = -10.0 * ratio.log10();
                // Invert the AR(1) shape at the band centres to get alpha.
                let fc_l = 0.5 * (lo0 + lo1);
                let fc_h = 0.5 * (hi0 + hi1);
                out.alpha_from_psd = solve_alpha_from_band_ratio(ratio, fc_l, fc_h, dt_s);
            }
        }
    }

    // ---------- verdict ----------
    out.median_counts_per_report = super::util::median(
        &seg.iter().map(|r| r.mag()).collect::<Vec<_>>());
    if out.median_counts_per_report > cfg.max_glide_counts_per_report {
        out.verdict = Verdict::Inconclusive;
        out.note = "glide too fast: the motion signal leaks into the high-frequency band. Repeat at 15-25 inches/second.";
        return out;
    }
    let mut v = Verdict::Pass;
    let mut note = "no firmware smoothing signature";
    if out.rho1_corrected > cfg.rho1_fail {
        v = Verdict::Fail;
        note = "high-frequency deltas are strongly serially correlated: firmware low-pass";
    } else if out.rho1_corrected > cfg.rho1_warn {
        v = v.worst(Verdict::Warn);
        note = "mild positive correlation in the high-frequency residual";
    }
    if cfg.expect_hard_stop {
        if out.tail_ms >= cfg.tail_fail_ms {
            v = v.worst(Verdict::Fail);
            note = "motion continued for many ms after the mouse physically stopped (interpolation tail)";
        } else if out.tail_ms >= cfg.tail_warn_ms {
            v = v.worst(Verdict::Warn);
            if note == "no firmware smoothing signature" {
                note = "motion continued briefly after the mouse physically stopped";
            }
        }
    }
    let _human_guard = cfg.max_human_tau_ms;
    out.verdict = v;
    out.note = note;
    out
}

/// Longest run of reports whose spacing never exceeds 2.5 nominal periods, as a
/// half-open index range into `seg`.
///
/// Index-based filtering is only valid on an evenly sampled series. Rather than
/// resample, which would invent data, this finds the longest stretch that
/// already is one and measures there, reporting how much of the stroke that
/// was.
fn uniform_run(seg: &[Report], nominal_ns: f64) -> (usize, usize) {
    if seg.len() < 2 || !(nominal_ns > 0.0) {
        return (0, seg.len());
    }
    let max_gap = (2.5 * nominal_ns) as u64;
    let (mut best_a, mut best_b) = (0usize, 1usize);
    let mut a = 0usize;
    for i in 1..seg.len() {
        if seg[i].t_ns.saturating_sub(seg[i - 1].t_ns) > max_gap {
            if i - a > best_b - best_a {
                best_a = a;
                best_b = i;
            }
            a = i;
        }
    }
    if seg.len() - a > best_b - best_a {
        best_a = a;
        best_b = seg.len();
    }
    (best_a, best_b)
}

/// Solve for alpha given the measured PSD ratio between two frequencies under the AR(1)
/// model. |H(f)|^2 = a^2 / (1 + b^2 - 2 b cos(2 pi f dt)), b = 1 - a.
/// Monotone in a, so a plain bisection on [1e-4, 1] is enough and cannot fail to converge.
fn solve_alpha_from_band_ratio(ratio: f64, f_lo: f64, f_hi: f64, dt: f64) -> f64 {
    let model = |a: f64| -> f64 {
        let b = 1.0 - a;
        let h = |f: f64| {
            let w = 2.0 * std::f64::consts::PI * f * dt;
            a * a / (1.0 + b * b - 2.0 * b * w.cos())
        };
        h(f_hi) / h(f_lo)
    };
    if ratio >= model(1.0 - 1e-9) { return 1.0; } // flat or rising: no filtering
    let (mut lo, mut hi) = (1.0e-4f64, 1.0f64);
    for _ in 0..80 {
        let mid = 0.5 * (lo + hi);
        if model(mid) < ratio { lo = mid } else { hi = mid }
    }
    0.5 * (lo + hi)
}
