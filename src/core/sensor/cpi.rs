//! Detector 1: CPI verification from a measured-distance swipe.
//!
//! PROTOCOL: user places the mouse on a ruler at mark A, swipes in a straight line to
//! mark B, stops. They type the physical distance A->B. Repeat >= 3 times.
//!
//! WHICH LENGTH ESTIMATOR?  Three candidates over the trimmed swipe:
//!   L_path  = sum_i |d_i|                       (arc length)
//!   L_net   = |sum_i d_i|                       (net displacement magnitude)
//!   L_axis  = |(sum_i d_i) . u|, u = PCA axis   (projection onto the stroke's own axis)
//!
//! The physical measurement the user makes is the STRAIGHT-LINE distance between two
//! marks. The unbiased estimator of that in counts is L_net. L_path is biased HIGH and
//! the bias never averages out: with per-report sensor noise of sigma counts per axis
//! and a true per-report step of mu counts along travel, E|d_i| >= |mu| by Jensen, with
//! the excess ~ sigma^2/(2|mu|) per report. Summed over N reports that is a systematic
//! inflation of N*sigma^2/(2|mu|) counts -- at 8 kHz the per-report step mu is small and
//! sigma^2/(2mu) can be a LARGE fraction of mu, so L_path can overestimate CPI by tens
//! of percent. It also inflates with any hand curvature. NEVER use L_path here.
//!
//! L_net vs L_axis: identical when the stroke is straight. If the stroke curves, L_axis
//! <= L_net, and L_net is still the correct answer, because the user measured A->B, and
//! sum d_i IS the A->B vector. So: USE L_NET. L_axis is reported only as a straightness
//! diagnostic (L_axis/L_net, and the off-axis component).
//!
//! We additionally report L_path/L_net so a wobbly swipe is visible to the user.

// The full API of this module is kept, not just the part the interface reads
// today. These are self-contained numerical routines checked as a whole against
// an outside reference, and trimming them to the current call sites would make
// that check harder to repeat than the unused functions are worth.
#![allow(dead_code)]

use super::seg::{dominant_span, MotionGate};
use super::types::{Report, Verdict, NS};

#[derive(Clone, Debug)]
pub struct CpiConfig {
    /// User's claimed / advertised CPI.
    pub claimed_cpi: f64,
    /// Physical distance the user swiped, inches.
    pub distance_in: f64,
    /// 1-sigma uncertainty of the user's distance measurement, inches.
    /// Default 0.040 in (~1.0 mm): +-1 mm placement error at each of two ends,
    /// modelled as uniform +-1 mm -> sigma = 1/sqrt(3) = 0.577 mm per end,
    /// two independent ends -> 0.816 mm = 0.032 in; rounded up to 1.0 mm for the
    /// finite width of a pencil mark and the mouse's own reference point ambiguity.
    pub distance_sigma_in: f64,
    pub gate: MotionGate,
}

impl CpiConfig {
    pub fn new(claimed_cpi: f64, distance_in: f64) -> Self {
        CpiConfig { claimed_cpi, distance_in, distance_sigma_in: 0.040, gate: MotionGate::default() }
    }
    pub fn from_mm(claimed_cpi: f64, distance_mm: f64) -> Self {
        let mut c = Self::new(claimed_cpi, distance_mm / 25.4);
        c.distance_sigma_in = 1.0 / 25.4;
        c
    }
}

#[derive(Clone, Debug)]
pub struct CpiResult {
    pub verdict: Verdict,
    pub measured_cpi: f64,
    /// Combined 1-sigma uncertainty on measured_cpi, same units.
    pub cpi_sigma: f64,
    /// (measured - claimed)/claimed, as a fraction (0.031 = +3.1%).
    pub deviation: f64,
    /// deviation expressed in units of the combined sigma.
    pub deviation_z: f64,
    pub l_net: f64,
    pub l_path: f64,
    pub l_axis: f64,
    /// l_path / l_net. 1.0 = perfectly straight+clean; > 1.02 means wobble or noise.
    pub wobble: f64,
    /// Perpendicular excursion of the path from the A->B chord, in counts (max).
    pub max_off_axis_counts: f64,
    pub n_reports: usize,
    pub duration_s: f64,
    pub peak_ips: f64,
    pub note: &'static str,
}

pub fn analyze_cpi(reports: &[Report], cfg: &CpiConfig) -> CpiResult {
    let mut out = CpiResult {
        verdict: Verdict::Inconclusive,
        measured_cpi: f64::NAN, cpi_sigma: f64::NAN, deviation: f64::NAN, deviation_z: f64::NAN,
        l_net: 0.0, l_path: 0.0, l_axis: 0.0, wobble: f64::NAN, max_off_axis_counts: 0.0,
        n_reports: 0, duration_s: 0.0, peak_ips: 0.0, note: "",
    };
    if !super::types::is_monotonic(reports) {
        out.note = super::types::NOT_MONOTONIC; return out;
    }
    if cfg.distance_in <= 0.0 || cfg.claimed_cpi <= 0.0 {
        out.note = "invalid claimed CPI or distance"; return out;
    }
    let Some((a, b)) = dominant_span(reports, cfg.gate) else {
        out.note = "no motion span found"; return out;
    };
    let seg = &reports[a..b];
    if seg.len() < 8 { out.note = "swipe too short (<8 reports)"; return out; }

    let sx: f64 = seg.iter().map(|r| r.dx as f64).sum();
    let sy: f64 = seg.iter().map(|r| r.dy as f64).sum();
    let l_net = (sx * sx + sy * sy).sqrt();
    let l_path: f64 = seg.iter().map(|r| r.mag()).sum();
    out.l_net = l_net;
    out.l_path = l_path;
    out.n_reports = seg.len();
    out.duration_s = seg[seg.len() - 1].t_ns.saturating_sub(seg[0].t_ns) as f64 / NS;

    if l_net < 1.0 { out.note = "net displacement ~0 (did the mouse return?)"; return out; }

    // PCA axis of the cumulative path, for diagnostics only.
    let mut pts = Vec::with_capacity(seg.len());
    let (mut cx, mut cy) = (0.0f64, 0.0f64);
    for r in seg { cx += r.dx as f64; cy += r.dy as f64; pts.push((cx, cy)); }
    let (_c, u, _res) = super::util::tls_line(&pts);
    out.l_axis = (sx * u.0 + sy * u.1).abs();
    out.wobble = l_path / l_net;

    // max perpendicular excursion from the straight A->B chord
    let (ex, ey) = (sx / l_net, sy / l_net);
    let (px, py) = (-ey, ex);
    out.max_off_axis_counts = pts.iter().map(|p| (p.0 * px + p.1 * py).abs()).fold(0.0, f64::max);

    // peak speed, for the "was this a real swipe" guard
    if out.duration_s > 0.0 {
        let mut peak = 0.0f64;
        let w = 8usize;
        for i in 0..seg.len().saturating_sub(w) {
            let dt = seg[i + w].t_ns.saturating_sub(seg[i].t_ns) as f64 / NS;
            if dt > 0.0 {
                let p: f64 = seg[i..i + w].iter().map(|r| r.mag()).sum();
                peak = peak.max(p / dt);
            }
        }
        out.peak_ips = peak / cfg.claimed_cpi;
    }

    out.measured_cpi = l_net / cfg.distance_in;

    // ---- uncertainty budget ----
    // sigma_L: end-point count quantisation is +-0.5 count at each end of the *span*
    //          (uniform -> 0.289 each, 2 ends -> 0.41 counts) plus the gate's
    //          start/stop localisation, bounded by the counts inside one gate window at
    //          the threshold, i.e. min_counts. Take them in quadrature.
    let sigma_l = (0.41f64.powi(2) + cfg.gate.min_counts.powi(2)).sqrt();
    let rel_l = sigma_l / l_net;
    let rel_d = cfg.distance_sigma_in / cfg.distance_in;
    let rel = (rel_l * rel_l + rel_d * rel_d).sqrt();
    out.cpi_sigma = out.measured_cpi * rel;

    out.deviation = (out.measured_cpi - cfg.claimed_cpi) / cfg.claimed_cpi;
    // z uses the combined sigma of the MEASUREMENT only; the claimed value is exact by
    // definition of the test ("does it match the number on the box").
    out.deviation_z = out.deviation / rel;

    // ---- bands ----
    // The floor of what a single ruler swipe can resolve is ~2*rel (95%). Sensor CPI
    // trim on commodity optical sensors (PixArt PAW/PMW families) is routinely 1-3% off
    // nominal and is not specified tightly in datasheets, so 2% is inside "normal part
    // variation" and must not be called a failure. 5%+ is user-perceptible when swapping
    // between two mice configured to the "same" CPI and is the usual review threshold.
    let ad = out.deviation.abs();
    let pass_band = 0.02f64.max(2.0 * rel);
    let warn_band = 0.05f64.max(3.0 * rel);
    out.verdict = if ad <= pass_band { Verdict::Pass }
                  else if ad <= warn_band { Verdict::Warn }
                  else { Verdict::Fail };

    // Guards that force Inconclusive regardless of the number.
    //
    // The gap guard comes first because it is the only one that can see a
    // swipe with a hole in it, and a hole is the one fault that corrupts the
    // number while leaving every other guard content. Reports that stop partway
    // remove distance from the count, and they remove it from the path and the
    // chord alike, so the wobble ratio stays at 1.00, the line stays straight,
    // the peak speed stays sane and the count stays large. The result is a CPI
    // that reads low by exactly the fraction of the swipe that went missing,
    // with nothing anywhere to say so. A mouse that drops off the bus for a
    // moment mid-swipe therefore reported itself as a mouse whose sensor
    // undercounts, over and over, which is a different fault entirely.
    //
    // The threshold has to clear the gaps that are not faults. A mouse sends
    // nothing when it has nothing to send, so slow moments legitimately leave
    // holes, and ordinary dropped polling slots leave small ones. Only a stall
    // far longer than either counts, and it does not need to be sensitive to be
    // useful: at a typical swipe speed 25 ms is well under one percent of the
    // distance, while the deficit that prompted this guard was eleven.
    let gap_limit_ns = (20.0 * median_interval_ns(seg)).max(25.0 * 1e6);
    let worst_gap_ns = seg
        .windows(2)
        .map(|w| w[1].t_ns.saturating_sub(w[0].t_ns))
        .max()
        .unwrap_or(0) as f64;
    if worst_gap_ns > gap_limit_ns {
        out.verdict = Verdict::Inconclusive;
        out.note = "reports stopped partway through this swipe, so distance is missing from \
                    the count and the CPI would read low. Check the connection and redo it";
    } else if out.wobble > 1.15 {
        out.verdict = Verdict::Inconclusive;
        out.note = "path length exceeds chord by >15%: swipe was curved or the mouse lifted; redo";
    } else if out.max_off_axis_counts > 0.10 * l_net {
        out.verdict = Verdict::Inconclusive;
        out.note = "path deviates >10% of its length off the A->B chord; redo straighter";
    } else if out.peak_ips > 0.0 && out.peak_ips > 40.0 {
        out.verdict = Verdict::Inconclusive;
        out.note = "swipe peaked above 40 IPS; may be clipping tracking. Swipe slowly.";
    } else if l_net < 500.0 {
        out.verdict = Verdict::Inconclusive;
        out.note = "fewer than 500 counts; use a longer distance for a useful CPI estimate";
    }
    out
}

/// Median gap between consecutive reports in a segment, in nanoseconds.
fn median_interval_ns(seg: &[super::Report]) -> f64 {
    if seg.len() < 2 {
        return 0.0;
    }
    let mut d: Vec<u64> = seg
        .windows(2)
        .map(|w| w[1].t_ns.saturating_sub(w[0].t_ns))
        .collect();
    d.sort_unstable();
    d[d.len() / 2] as f64
}

/// Combine >= 3 repeated swipes. Uses the MEDIAN of per-trial CPI (robust to one bad
/// swipe) and reports the spread; the spread is the honest reproducibility figure and is
/// usually larger than the per-trial analytic sigma.
#[derive(Clone, Debug)]
pub struct CpiSummary {
    pub verdict: Verdict,
    pub median_cpi: f64,
    /// Standard error of the median across trials, = 1.253 * sd / sqrt(n).
    pub se_cpi: f64,
    pub deviation: f64,
    pub n_trials: usize,
    pub per_trial: Vec<f64>,
}

pub fn summarize_cpi(trials: &[CpiResult], claimed_cpi: f64) -> CpiSummary {
    let good: Vec<f64> = trials.iter()
        .filter(|t| t.verdict != Verdict::Inconclusive && t.measured_cpi.is_finite())
        .map(|t| t.measured_cpi).collect();
    if good.is_empty() {
        return CpiSummary { verdict: Verdict::Inconclusive, median_cpi: f64::NAN,
            se_cpi: f64::NAN, deviation: f64::NAN, n_trials: 0, per_trial: vec![] };
    }
    let m = super::util::median(&good);
    let sd = super::util::stddev(&good);
    let se = if good.len() > 1 { 1.2533 * sd / (good.len() as f64).sqrt() } else { f64::NAN };
    let dev = (m - claimed_cpi) / claimed_cpi;
    // With n trials the resolvable band shrinks; keep 2% as a hard floor for part variation.
    let rel = if se.is_finite() { (se / m).max(0.005) } else { 0.02 };
    let v = if dev.abs() <= 0.02f64.max(2.0 * rel) { Verdict::Pass }
            else if dev.abs() <= 0.05f64.max(3.0 * rel) { Verdict::Warn }
            else { Verdict::Fail };
    CpiSummary { verdict: v, median_cpi: m, se_cpi: se, deviation: dev,
        n_trials: good.len(), per_trial: good }
}
