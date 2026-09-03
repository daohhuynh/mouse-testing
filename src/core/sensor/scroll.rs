//! Detector 7: scroll detents, reversed steps and skipped steps.
//!
//! THE QUANTUM PROBLEM. "Counts per detent" is device- and OS-dependent:
//!   * classic HID wheel: +-1 per detent
//!   * Windows WM_MOUSEWHEEL / RI_MOUSE_WHEEL: +-120 (WHEEL_DELTA) per detent, and
//!     high-resolution wheels emit fractions of 120 (e.g. 30, 40, 15)
//!   * Logitech HID++ 0x2121 high-res wheel: 8 or 15 sub-counts per detent, delivered as
//!     several reports inside one detent click
//!   * macOS CGEvent with kCGScrollWheelEventIsContinuous == 1 (trackpad / free-spin
//!     mode): no quantum at all
//! So the quantum MUST be inferred from the data, and it must be inferred from
//! TIME-CLUSTERED SUMS, not from individual reports -- a device that splits one detent
//! across 8 reports of +1 has a per-report quantum of 1 and a per-detent quantum of 8.
//!
//! ALGORITHM
//!   1. Cluster wheel reports: a new cluster starts when the gap since the previous wheel
//!      report exceeds `cluster_gap_ns` (default 12 ms) or the sign changes.
//!      Justification: a mechanical detent's sub-reports arrive inside the few ms the
//!      wheel takes to snap over the notch, while human wheel ticking tops out around
//!      15-20 detents/s, i.e. >= 50 ms apart. 12 ms sits an order of magnitude below the
//!      human rate and above the intra-detent burst.
//!   2. c_j = signed sum of each cluster.
//!   3. Quantum q: candidate set = the distinct |c_j| plus the pairwise GCDs of the most
//!      common values plus {1, 120}. Score each candidate by the fraction of |c_j| within
//!      2% (or 0.25 absolute, whichever is larger) of an integer multiple of q. Choose the
//!      LARGEST q whose coverage >= 0.90. Plain GCD is not used on its own because a
//!      single stray count collapses it to 1.
//!   4. If no candidate >= 2 reaches 0.90 coverage AND the values are broadly spread,
//!      report CONTINUOUS (high-res / inertial) instead of guessing.
//!   5. detents(dir) = sum of round(|c_j|/q) over clusters of that sign.
//!
//! DEFECTS
//!   REVERSED: a cluster whose sign is opposite to BOTH neighbours, with |c_j| <= 1 q.
//!             This is quadrature-decode error / encoder chatter. A deliberate direction
//!             change has a run of same-sign clusters after it, so the both-neighbours
//!             rule excludes it.
//!   SKIPPED:  round(|c_j|/q) >= 2 in a context where single steps are the norm. Guarded
//!             against legitimate fast scrolling: a genuine flick MERGES detents, so it
//!             also SHORTENS the gap. We require the preceding gap to be >= 0.7 x the
//!             median gap, i.e. the user was scrolling at a normal cadence and the device
//!             still emitted a double.

// The full API of this module is kept, not just the part the interface reads
// today. These are self-contained numerical routines checked as a whole against
// an outside reference, and trimming them to the current call sites would make
// that check harder to repeat than the unused functions are worth.
#![allow(dead_code)]

use super::types::{Report, Verdict};

#[derive(Copy, Clone, Debug)]
pub struct ScrollConfig {
    pub cluster_gap_ns: u64,
    pub quantum_coverage: f64,
    pub rev_warn: f64,
    pub rev_fail: f64,
    pub skip_warn: f64,
    pub skip_fail: f64,
}

impl Default for ScrollConfig {
    fn default() -> Self {
        ScrollConfig {
            cluster_gap_ns: 12_000_000,
            quantum_coverage: 0.90,
            // A quadrature decoder that is working emits zero reversals. 1 in 200 detents
            // is already a visible backwards jump while reading, so 0.5% is Warn and 2%
            // (1 in 50) is unusable.
            rev_warn: 0.005,
            rev_fail: 0.02,
            skip_warn: 0.01,
            skip_fail: 0.05,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Cluster {
    pub t_ns: u64,
    pub sum: i64,
    pub n_reports: usize,
    pub duration_ns: u64,
    pub steps: i64,
}

#[derive(Clone, Debug)]
pub struct ScrollResult {
    pub verdict: Verdict,
    pub continuous: bool,
    pub quantum: f64,
    pub quantum_coverage: f64,
    pub n_clusters: usize,
    pub detents_up: i64,
    pub detents_down: i64,
    pub reversals: usize,
    pub skips: usize,
    pub reversal_rate: f64,
    pub skip_rate: f64,
    pub median_gap_ms: f64,
    /// The cluster gap threshold that was actually used, ms (data-derived).
    pub cluster_gap_ms: f64,
    /// Which wheel this result describes.
    pub axis: Axis,
    pub note: &'static str,
}

/// Choose the intra-detent / inter-detent gap threshold FROM THE DATA.
/// A high-resolution wheel that splits one detent across 15 reports 1 ms apart has an
/// intra-burst gap of ~1 ms and an inter-detent gap of ~90 ms; a fixed 12 ms threshold
/// works there but fails as soon as the burst is long enough, or the user scrolls fast
/// enough, that the two populations approach each other. So we split log(gap) with
/// 1-D Otsu (two-class variance minimisation) and put the threshold at the geometric
/// midpoint of the two class means. If the classes are within a factor of 3 the
/// distribution is unimodal (one report per detent) and we fall back to the default.
pub fn adaptive_cluster_gap(reports: &[Report], axis: Axis, default_ns: u64) -> u64 {
    let ts: Vec<u64> = reports.iter().filter(|r| axis.of(r) != 0).map(|r| r.t_ns).collect();
    if ts.len() < 20 { return default_ns; }
    let mut lg: Vec<f64> = ts.windows(2)
        .map(|w| (w[1].saturating_sub(w[0])).max(1) as f64)
        .map(|g| g.ln()).collect();
    lg.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = lg.len();
    // Two prefix sums, of the values and of their squares, so each candidate
    // split's within-class scatter is O(1) instead of O(n). Recomputing the
    // sums inside the loop made this quadratic and it ran on the interface
    // thread: measured 68 ms at 10k wheel reports, 665 ms at 40k and 2.72 s at
    // 80k, which a free-spinning high-resolution wheel reaches in under two
    // minutes of scrolling.
    let (pre, pre2) = {
        let mut a = vec![0.0; n + 1];
        let mut b = vec![0.0; n + 1];
        for i in 0..n {
            a[i + 1] = a[i] + lg[i];
            b[i + 1] = b[i] + lg[i] * lg[i];
        }
        (a, b)
    };
    let (mut best_k, mut best_score) = (0usize, f64::INFINITY);
    for k in (n / 10).max(1)..(n - (n / 10).max(1)) {
        // sum (x - mean)^2 == sum x^2 - (sum x)^2 / count, per class.
        let (s1, q1) = (pre[k], pre2[k]);
        let (s2, q2) = (pre[n] - pre[k], pre2[n] - pre2[k]);
        let ss = (q1 - s1 * s1 / k as f64) + (q2 - s2 * s2 / (n - k) as f64);
        if ss < best_score { best_score = ss; best_k = k; }
    }
    if best_k == 0 { return default_ns; }
    let m1 = pre[best_k] / best_k as f64;
    let m2 = (pre[n] - pre[best_k]) / (n - best_k) as f64;
    if (m2 - m1) < 3.0f64.ln() { return default_ns; }   // unimodal
    let thr = (0.5 * (m1 + m2)).exp();
    (thr as u64).clamp(1_500_000, 60_000_000)
}

pub fn cluster_wheel(reports: &[Report], axis: Axis, gap_ns: u64) -> Vec<Cluster> {
    let ev: Vec<&Report> = reports.iter().filter(|r| axis.of(r) != 0).collect();
    let mut out: Vec<Cluster> = Vec::new();
    let mut i = 0usize;
    while i < ev.len() {
        let start = i;
        let sign = axis.of(ev[i]).signum();
        let mut sum = axis.of(ev[i]) as i64;
        let mut j = i + 1;
        while j < ev.len()
            && axis.of(ev[j]).signum() == sign
            && ev[j].t_ns.saturating_sub(ev[j - 1].t_ns) <= gap_ns
        {
            sum += axis.of(ev[j]) as i64;
            j += 1;
        }
        out.push(Cluster {
            t_ns: ev[start].t_ns,
            sum,
            n_reports: j - start,
            duration_ns: ev[j - 1].t_ns.saturating_sub(ev[start].t_ns),
            steps: 0,
        });
        i = j;
    }
    out
}

fn gcd(a: i64, b: i64) -> i64 { if b == 0 { a.abs() } else { gcd(b, a % b) } }

/// Infer the counts-per-detent quantum from cluster sums. Returns (q, coverage).
pub fn infer_quantum(sums: &[i64], min_cov: f64) -> (f64, f64) {
    let mags: Vec<i64> = sums.iter().map(|s| s.abs()).filter(|s| *s > 0).collect();
    if mags.is_empty() { return (1.0, 0.0); }
    let mut cands: Vec<i64> = Vec::new();
    cands.push(1);
    cands.push(120);
    for m in &mags { cands.push(*m); }
    // pairwise gcds of the 12 most common magnitudes
    let mut freq: std::collections::BTreeMap<i64, usize> = Default::default();
    for m in &mags { *freq.entry(*m).or_insert(0) += 1; }
    let mut top: Vec<(i64, usize)> = freq.into_iter().collect();
    top.sort_by(|a, b| b.1.cmp(&a.1));
    top.truncate(12);
    for i in 0..top.len() {
        for j in i + 1..top.len() {
            let g = gcd(top[i].0, top[j].0);
            if g > 0 { cands.push(g); }
        }
    }
    cands.sort_unstable();
    cands.dedup();
    let cover = |q: i64| -> f64 {
        if q <= 0 { return 0.0; }
        let qf = q as f64;
        let ok = mags.iter().filter(|m| {
            let r = **m as f64 / qf;
            let tol = (0.02 * r).max(0.25 / qf);
            (r - r.round()).abs() <= tol && r.round() >= 1.0
        }).count();
        ok as f64 / mags.len() as f64
    };
    let mut best = (1i64, cover(1));
    for q in cands.iter().rev() {
        let c = cover(*q);
        if c >= min_cov { best = (*q, c); break; }
    }
    (best.0 as f64, best.1)
}

/// Which wheel to analyse. A tilt wheel is a second encoder with its own
/// detents and its own faults, so it gets the same analysis rather than being
/// folded into the vertical figures.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Axis {
    Vertical,
    Horizontal,
}

impl Axis {
    fn of(self, r: &Report) -> i32 {
        match self {
            Axis::Vertical => r.wheel,
            Axis::Horizontal => r.hwheel,
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Axis::Vertical => "wheel",
            Axis::Horizontal => "tilt / horizontal wheel",
        }
    }
}

pub fn analyze_scroll(reports: &[Report], cfg: &ScrollConfig) -> ScrollResult {
    analyze_axis(reports, Axis::Vertical, cfg)
}

pub fn analyze_axis(reports: &[Report], axis: Axis, cfg: &ScrollConfig) -> ScrollResult {
    let mut out = ScrollResult {
        verdict: Verdict::Inconclusive, continuous: false, quantum: f64::NAN,
        quantum_coverage: 0.0, n_clusters: 0, detents_up: 0, detents_down: 0,
        reversals: 0, skips: 0, reversal_rate: f64::NAN, skip_rate: f64::NAN,
        median_gap_ms: f64::NAN, cluster_gap_ms: f64::NAN, axis, note: "",
    };
    if !super::types::is_monotonic(reports) {
        out.note = super::types::NOT_MONOTONIC;
        return out;
    }
    let gap_ns = adaptive_cluster_gap(reports, axis, cfg.cluster_gap_ns);
    let mut cl = cluster_wheel(reports, axis, gap_ns);
    out.cluster_gap_ms = gap_ns as f64 / 1.0e6;
    out.n_clusters = cl.len();
    if cl.len() < 10 { out.note = "fewer than 10 scroll clusters; scroll more"; return out; }

    let sums: Vec<i64> = cl.iter().map(|c| c.sum).collect();
    let (q, cov) = infer_quantum(&sums, cfg.quantum_coverage);
    out.quantum = q;
    out.quantum_coverage = cov;

    // CONTINUOUS if we could not find any quantum >= 2 that explains the data and the
    // magnitudes are broadly spread (a real 1-count-per-detent wheel has a tight
    // distribution around 1, which is NOT continuous).
    let mags: Vec<f64> = sums.iter().map(|s| s.abs() as f64).collect();
    let spread = super::util::percentile(&mags, 0.9) / super::util::percentile(&mags, 0.1).max(1.0);
    let med_mag = super::util::median(&mags);
    if q <= 1.0 && (spread > 4.0 || med_mag > 3.0) {
        out.continuous = true;
        out.note = "no detent quantum found and magnitudes are broadly spread: high-resolution/continuous scrolling";
    }

    let gaps: Vec<f64> = cl.windows(2)
        .map(|w| (w[1].t_ns.saturating_sub(w[0].t_ns)) as f64 / 1.0e6).collect();
    out.median_gap_ms = super::util::median(&gaps);

    for c in cl.iter_mut() {
        c.steps = ((c.sum as f64 / q).round()) as i64;
        if c.steps > 0 { out.detents_up += c.steps } else { out.detents_down += -c.steps }
    }

    // REVERSALS
    for j in 1..cl.len().saturating_sub(1) {
        let s = cl[j].sum.signum();
        if s != 0
            && cl[j - 1].sum.signum() == -s
            && cl[j + 1].sum.signum() == -s
            && cl[j].steps.abs() <= 1
        { out.reversals += 1; }
    }
    // SKIPS
    let median_step = {
        let st: Vec<f64> = cl.iter().map(|c| c.steps.abs() as f64).collect();
        super::util::median(&st)
    };
    if median_step <= 1.5 {
        for j in 1..cl.len() {
            let gap_ms = (cl[j].t_ns.saturating_sub(cl[j - 1].t_ns)) as f64 / 1.0e6;
            if cl[j].steps.abs() >= 2 && gap_ms >= 0.7 * out.median_gap_ms { out.skips += 1; }
        }
    }
    let n = cl.len() as f64;
    out.reversal_rate = out.reversals as f64 / n;
    out.skip_rate = out.skips as f64 / n;

    if out.continuous {
        // Reversal/skip logic is not meaningful without detents.
        out.verdict = Verdict::Inconclusive;
        return out;
    }
    let mut v = Verdict::Pass;
    v = v.worst(if out.reversal_rate >= cfg.rev_fail { Verdict::Fail }
                else if out.reversal_rate >= cfg.rev_warn { Verdict::Warn } else { Verdict::Pass });
    v = v.worst(if out.skip_rate >= cfg.skip_fail { Verdict::Fail }
                else if out.skip_rate >= cfg.skip_warn { Verdict::Warn } else { Verdict::Pass });
    out.verdict = v;
    if out.note.is_empty() {
        out.note = if v == Verdict::Pass { "detents clean" } else { "encoder errors present" };
    }
    out
}
