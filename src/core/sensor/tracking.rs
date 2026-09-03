//! Detector 5: malfunction speed / maximum tracking speed.
//!
//! PROTOCOL: user makes ~10 swipes of increasing speed, slow to as fast as they can, on
//! the surface they actually use. Everything is captured as one stream.
//!
//! We never know the COMMANDED speed, so "the speed where it breaks" must be defined
//! operationally: the highest speed at which no degradation signature is active, measured
//! in the window immediately BEFORE the first sustained failure. Measuring at or after
//! the failure is wrong, because at failure the reported speed is itself corrupted
//! (usually an underestimate).
//!
//! FIELD WIDTH IS DETECTED FROM THE DATA, not assumed. HID relative axes are commonly
//! 8-bit (-127..127), 12-bit (-2047..2047) or 16-bit (-32767..32767). A clipped
//! distribution has an ATOM at its maximum; an unclipped one does not. Statistic:
//!     clip_atom = #{|d| == M} / #{|d| >= 0.9 M},  M = max observed |d|
//! For any smooth speed distribution this is small (the top decile of magnitudes spreads
//! over ~0.1 M distinct integer values). For a clipped stream it approaches 1. We require
//! clip_atom > 0.25 AND M within 1 of a standard field bound before declaring saturation,
//! so a merely fast-but-healthy mouse is never accused.
//!
//! FOUR SIGNATURES, evaluated per 5 ms window, each needing 3 consecutive windows to fire
//! (debounce -- a single anomalous window is noise, not a malfunction):
//!   S1 CLIP     fraction of reports at the detected field bound > 0.20
//!   S2 REVERSE  >= 2 sign changes of the dominant-axis delta while |median delta| > 5.
//!               A hand mid-swipe never reverses; a sensor that loses correlation does.
//!   S3 DROPOUT  an inter-report interval > 2.5 x nominal inside a moving span
//!   S4 COLLAPSE speed falls by > 50% within 5 ms while it was still accelerating.
//!               Hand deceleration of a swipe takes tens of ms; a 5 ms collapse is the
//!               sensor giving up, not the arm.

// The full API of this module is kept, not just the part the interface reads
// today. These are self-contained numerical routines checked as a whole against
// an outside reference, and trimming them to the current call sites would make
// that check harder to repeat than the unused functions are worth.
#![allow(dead_code)]

use super::seg::{motion_spans, MotionGate};
use super::types::{Report, Verdict, NS};

pub const STD_FIELD_BOUNDS: [i32; 4] = [127, 2047, 4095, 32767];

#[derive(Copy, Clone, Debug)]
pub struct TrackConfig {
    pub cpi: f64,
    pub window_ns: u64,
    pub debounce_windows: usize,
    pub clip_atom_thresh: f64,
    pub clip_frac_thresh: f64,
    /// Bands on the recovered max tracking speed, inches/second.
    pub mts_fail_ips: f64,
    pub mts_warn_ips: f64,
}

impl Default for TrackConfig {
    fn default() -> Self {
        TrackConfig {
            cpi: 1600.0,
            window_ns: 5_000_000, // 5 ms
            debounce_windows: 3,
            clip_atom_thresh: 0.25,
            clip_frac_thresh: 0.20,
            // Anchors: entry-level sensors spec 20-30 IPS; mid sensors 150 IPS;
            // current flagship optical sensors spec 400-750 IPS. A mouse that breaks
            // below 100 IPS will be hit by ordinary low-sensitivity FPS flicks.
            mts_fail_ips: 100.0,
            mts_warn_ips: 200.0,
        }
    }
}

#[derive(Clone, Debug)]
pub struct FieldWidth {
    pub observed_max: i32,
    pub matched_bound: Option<i32>,
    pub clip_atom: f64,
    pub saturating: bool,
}

#[derive(Clone, Debug)]
pub struct TrackWindow {
    pub t_ns: u64,
    pub speed_ips: f64,
    pub clip_frac: f64,
    pub sign_flips: usize,
    pub max_interval_ratio: f64,
    pub failed: bool,
    pub reason: &'static str,
}

#[derive(Clone, Debug)]
pub struct TrackResult {
    pub verdict: Verdict,
    pub field: FieldWidth,
    /// Highest clean speed observed, IPS. If `bounded_below` is true this is a LOWER
    /// BOUND on the real max tracking speed, not a measurement of it.
    pub max_tracking_ips: f64,
    /// True when no signature ever fired: the user simply never swiped fast enough.
    pub bounded_below: bool,
    pub peak_observed_ips: f64,
    pub first_failure_ips: f64,
    pub first_failure_reason: &'static str,
    pub n_windows: usize,
    pub n_failed_windows: usize,
    pub note: &'static str,
}

fn detect_field_width(reports: &[Report]) -> FieldWidth {
    let mut m = 0i32;
    for r in reports { m = m.max(r.dx.abs()).max(r.dy.abs()); }
    let mut at_max = 0usize;
    let mut near_max = 0usize;
    let thr = ((m as f64) * 0.9).floor() as i32;
    for r in reports {
        for v in [r.dx.abs(), r.dy.abs()] {
            if v >= thr && v > 0 { near_max += 1; }
            if v == m && m > 0 { at_max += 1; }
        }
    }
    let atom = if near_max > 0 { at_max as f64 / near_max as f64 } else { 0.0 };
    let matched = STD_FIELD_BOUNDS.iter().copied().find(|b| (m - b).abs() <= 1);
    FieldWidth {
        observed_max: m,
        matched_bound: matched,
        clip_atom: atom,
        // Require BOTH a pile-up at the max and a max that looks like a protocol bound,
        // and at least a handful of samples so a 3-report swipe cannot trigger it.
        saturating: matched.is_some() && atom > 0.25 && at_max >= 8,
    }
}

pub fn analyze_tracking(reports: &[Report], cfg: &TrackConfig) -> TrackResult {
    let field = detect_field_width(reports);
    let mut out = TrackResult {
        verdict: Verdict::Inconclusive, field: field.clone(),
        max_tracking_ips: 0.0, bounded_below: true, peak_observed_ips: 0.0,
        first_failure_ips: f64::NAN, first_failure_reason: "", n_windows: 0,
        n_failed_windows: 0, note: "",
    };
    if !super::types::is_monotonic(reports) {
        out.note = super::types::NOT_MONOTONIC; return out;
    }
    if reports.len() < 32 { out.note = "too few reports"; return out; }

    // Nominal interval from the robust lower-mode estimator (see polling.rs).
    let ivs: Vec<f64> = reports.windows(2).map(|w| w[1].t_ns.saturating_sub(w[0].t_ns) as f64).collect();
    let nominal_ns = crate::core::polling::nominal_interval_ns(&ivs);
    if !(nominal_ns > 0.0) { out.note = "cannot estimate nominal interval"; return out; }

    let bound = field.matched_bound.unwrap_or(i32::MAX);
    let spans = motion_spans(reports, MotionGate::default());
    let mut wins: Vec<TrackWindow> = Vec::new();

    for (a, b) in spans {
        let seg = &reports[a..b];
        if seg.len() < 4 { continue; }
        let t0 = seg[0].t_ns;
        let mut i = 0usize;
        while i < seg.len() {
            let wstart = seg[i].t_ns;
            let wend = wstart + cfg.window_ns;
            let mut j = i;
            while j < seg.len() && seg[j].t_ns < wend { j += 1; }
            if j - i >= 3 {
                let sub = &seg[i..j];
                let dt = (sub[sub.len() - 1].t_ns.saturating_sub(sub[0].t_ns) as f64 / NS)
                    .max(cfg.window_ns as f64 / NS * 0.5);
                let path: f64 = sub.iter().map(|r| r.mag()).sum();
                let speed = path / dt / cfg.cpi;

                let n_clip = sub.iter()
                    .filter(|r| r.dx.abs() >= bound - 1 || r.dy.abs() >= bound - 1).count();
                let clip_frac = n_clip as f64 / sub.len() as f64;

                // dominant axis inside the window
                let sx: f64 = sub.iter().map(|r| r.dx as f64).sum::<f64>().abs();
                let sy: f64 = sub.iter().map(|r| r.dy as f64).sum::<f64>().abs();
                let dv: Vec<i32> = if sx >= sy { sub.iter().map(|r| r.dx).collect() }
                                   else { sub.iter().map(|r| r.dy).collect() };
                let mut mags: Vec<f64> = dv.iter().map(|v| (*v as f64).abs()).collect();
                mags.sort_by(|a, b| a.partial_cmp(b).unwrap());
                let med = mags[mags.len() / 2];
                let mut flips = 0usize;
                if med > 5.0 {
                    let mut last = 0i32;
                    for v in &dv {
                        let s = v.signum();
                        if s != 0 {
                            if last != 0 && s != last { flips += 1; }
                            last = s;
                        }
                    }
                }
                // Dropout: only count a gap as a dropped report when BOTH bracketing
                // reports carry >= 2 counts. Below that the device is legitimately
                // allowed to stay silent (sub-count motion in that slot), and counting
                // those as drops fires this detector on a perfectly healthy mouse at low
                // speed -- which it did, on 3/120 clean trials, before this guard.
                let mut mir: f64 = 0.0;
                for k in 1..sub.len() {
                    let a_ok = sub[k-1].dx.abs().max(sub[k-1].dy.abs()) >= 2;
                    let b_ok = sub[k].dx.abs().max(sub[k].dy.abs()) >= 2;
                    if a_ok && b_ok {
                        mir = mir.max(sub[k].t_ns.saturating_sub(sub[k - 1].t_ns) as f64 / nominal_ns);
                    }
                }

                let mut failed = false;
                let mut reason = "";
                if field.saturating && clip_frac > cfg.clip_frac_thresh {
                    failed = true; reason = "report-field saturation";
                }
                if flips >= 2 { failed = true; reason = "direction reversal mid-swipe"; }
                if mir > 2.5 { failed = true; reason = "dropped reports"; }
                wins.push(TrackWindow { t_ns: wstart, speed_ips: speed, clip_frac,
                    sign_flips: flips, max_interval_ratio: mir, failed, reason });
            }
            // advance by half a window (50% overlap)
            let next_t = wstart + cfg.window_ns / 2;
            let mut k = i;
            while k < seg.len() && seg[k].t_ns < next_t { k += 1; }
            i = if k > i { k } else { i + 1 };
            let _ = t0;
        }
    }
    out.n_windows = wins.len();
    if wins.len() < 4 { out.note = "not enough analysis windows"; return out; }

    // S4 COLLAPSE: speed drops > 50% in <= 5 ms while it had been rising.
    for k in 2..wins.len() {
        let prev = wins[k - 1].speed_ips;
        let prev2 = wins[k - 2].speed_ips;
        let cur = wins[k].speed_ips;
        let rising = prev > prev2 * 1.10;
        let gap_ms = (wins[k].t_ns.saturating_sub(wins[k - 1].t_ns)) as f64 / 1.0e6;
        if rising && gap_ms <= 6.0 && cur < 0.5 * prev && prev > 5.0 {
            wins[k].failed = true;
            if wins[k].reason.is_empty() { wins[k].reason = "implausible speed collapse"; }
        }
    }

    out.peak_observed_ips = wins.iter().map(|w| w.speed_ips).fold(0.0, f64::max);
    out.n_failed_windows = wins.iter().filter(|w| w.failed).count();

    // First SUSTAINED failure: `debounce_windows` consecutive failing windows.
    let mut first_fail: Option<usize> = None;
    let mut run = 0usize;
    for (k, w) in wins.iter().enumerate() {
        if w.failed { run += 1; if run >= cfg.debounce_windows {
            first_fail = Some(k + 1 - cfg.debounce_windows); break; } }
        else { run = 0; }
    }

    match first_fail {
        None => {
            // Never broke. Report a LOWER BOUND: the 95th percentile of clean speeds.
            let s: Vec<f64> = wins.iter().map(|w| w.speed_ips).collect();
            out.max_tracking_ips = super::util::percentile(&s, 0.95);
            out.bounded_below = true;
            out.note = "no degradation observed; value is a LOWER BOUND, swipe faster to bound it above";
            out.verdict = if out.max_tracking_ips >= cfg.mts_warn_ips { Verdict::Pass }
                          else { Verdict::Inconclusive };
            if out.verdict == Verdict::Inconclusive {
                out.note = "no degradation observed, but the user never exceeded the warn threshold";
            }
        }
        Some(k) => {
            out.bounded_below = false;
            out.first_failure_ips = wins[k].speed_ips;
            out.first_failure_reason = wins[k].reason;
            // Highest clean speed strictly before the failure, taken as the 90th
            // percentile of the clean windows in the 40 windows preceding it so a single
            // over-read window cannot inflate the answer.
            let lo = k.saturating_sub(40);
            let clean: Vec<f64> = wins[lo..k].iter()
                .filter(|w| !w.failed).map(|w| w.speed_ips).collect();
            out.max_tracking_ips = if clean.is_empty() { wins[k].speed_ips }
                                   else { super::util::percentile(&clean, 0.90) };
            out.note = out.first_failure_reason;
            out.verdict = if out.max_tracking_ips < cfg.mts_fail_ips { Verdict::Fail }
                          else if out.max_tracking_ips < cfg.mts_warn_ips { Verdict::Warn }
                          else { Verdict::Pass };
        }
    }
    out
}
