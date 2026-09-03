//! Motion segmentation: trimming leading/trailing idle and finding motion spans.
//! Shared by the CPI, snapping, smoothing, tracking-speed and polling detectors.

// The full API of this module is kept, not just the part the interface reads
// today. These are self-contained numerical routines checked as a whole against
// an outside reference, and trimming them to the current call sites would make
// that check harder to repeat than the unused functions are worth.
#![allow(dead_code)]

use super::types::Report;

/// Parameters for "is the mouse moving here?".
#[derive(Copy, Clone, Debug)]
pub struct MotionGate {
    /// Sliding window used to smooth the activity decision.
    pub window_ns: u64,
    /// Minimum total path length (counts) inside the window to call it "moving".
    pub min_counts: f64,
}

impl Default for MotionGate {
    fn default() -> Self {
        // 20 ms is long enough that a single dropped/zero report never breaks a span,
        // and short enough to localise the ends of a swipe to ~1% of a 200 ms swipe.
        // 3 counts in 20 ms at 1600 CPI = 0.094 in/s, ~2 orders of magnitude below any
        // deliberate swipe and ~1 order above worst-case sensor idle jitter.
        MotionGate { window_ns: 20_000_000, min_counts: 3.0 }
    }
}

/// Maximal index ranges [start, end) over which the gate says "moving".
pub fn motion_spans(r: &[Report], g: MotionGate) -> Vec<(usize, usize)> {
    let n = r.len();
    if n == 0 { return vec![]; }
    // prefix sum of path length so a window sum is O(1)
    let mut pre = vec![0.0f64; n + 1];
    for i in 0..n { pre[i + 1] = pre[i] + r[i].mag(); }

    let mut active = vec![false; n];
    let (mut lo, mut hi) = (0usize, 0usize);
    for i in 0..n {
        let t = r[i].t_ns;
        let half = g.window_ns / 2;
        let t0 = t.saturating_sub(half);
        let t1 = t.saturating_add(half);
        while lo < n && r[lo].t_ns < t0 { lo += 1; }
        if hi < lo { hi = lo; }
        while hi < n && r[hi].t_ns <= t1 { hi += 1; }
        active[i] = (pre[hi] - pre[lo]) >= g.min_counts;
    }
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < n {
        if active[i] {
            let s = i;
            while i < n && active[i] { i += 1; }
            out.push((s, i));
        } else { i += 1; }
    }
    out
}

/// The single longest motion span, by path length. Use for a "one swipe"/"one stroke" test.
pub fn dominant_span(r: &[Report], g: MotionGate) -> Option<(usize, usize)> {
    motion_spans(r, g).into_iter().max_by(|a, b| {
        let pa: f64 = r[a.0..a.1].iter().map(|x| x.mag()).sum();
        let pb: f64 = r[b.0..b.1].iter().map(|x| x.mag()).sum();
        pa.partial_cmp(&pb).unwrap_or(std::cmp::Ordering::Equal)
    })
}

/// Trim leading and trailing zero-motion reports from a slice (hard trim, no window).
pub fn trim_zeros(r: &[Report]) -> (usize, usize) {
    let mut a = 0usize;
    let mut b = r.len();
    while a < b && !r[a].is_moving() { a += 1; }
    while b > a && !r[b - 1].is_moving() { b -= 1; }
    (a, b)
}

/// Resample a delta stream onto a uniform grid of `bin_ns`, summing counts per bin.
/// Returns (t_center_ns, sum_dx, sum_dy, n_reports) per bin, dropping empty tail bins.
/// This is the workhorse that removes per-report quantisation from angle statistics.
pub fn bin_deltas(r: &[Report], bin_ns: u64) -> Vec<(u64, f64, f64, u32)> {
    if r.is_empty() || bin_ns == 0 { return vec![]; }
    let t0 = r[0].t_ns;
    let t1 = r[r.len() - 1].t_ns;
    let nb = (((t1 - t0) / bin_ns) + 1) as usize;
    let mut out: Vec<(u64, f64, f64, u32)> =
        (0..nb).map(|i| (t0 + (i as u64) * bin_ns + bin_ns / 2, 0.0, 0.0, 0)).collect();
    for rep in r {
        let k = (rep.t_ns.saturating_sub(t0) / bin_ns) as usize;
        if k < nb {
            out[k].1 += rep.dx as f64;
            out[k].2 += rep.dy as f64;
            out[k].3 += 1;
        }
    }
    out
}
