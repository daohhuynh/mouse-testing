//! Lift-off distance: how high the mouse can rise before the sensor stops
//! seeing the pad.
//!
//! # The rig, and why this one
//!
//! Two stacks of the same number of cards lie on the pad with a slot of a few
//! millimetres between them, the slot running across the sweep. The mouse's
//! feet ride on the cards and the sensor looks through the slot at the pad.
//! Over the cards the imaged surface is the card top, the same plane the feet
//! are on, so the sensor sits at its normal height. Over the slot the imaged
//! surface is the pad, one stack thickness further away.
//!
//! Three things follow, and they are the whole reason for this rig rather than
//! the obvious one of putting the mouse on a stack and lifting it:
//!
//! - The mouse never leaves the desk, so a cable never moves. A rig that asks
//!   the user to lift the mouse cannot be used on a mouse whose fault is that
//!   lifting it disturbs the cable.
//! - Nothing is climbed, so the mouse never tilts and the height under the
//!   sensor is a step function of position. The height being bracketed is the
//!   stack thickness rather than a smear across the mouse's own footprint.
//! - The sensor lets go at the stack height and re-acquires at zero, which is
//!   below any acquire threshold, so this measures the RELEASE threshold on its
//!   own. A ramped or lifted rig measures release and re-acquire mixed together
//!   and cannot separate them.
//!
//! # The ambiguity this test exists to survive
//!
//! When the sensor is not tracking it reports nothing, and that is exactly what
//! a hand holding still reports. A test that cannot separate those two would
//! report a user who paused as a mouse with a low lift-off distance, which is
//! worse than no test at all.
//!
//! Three properties separate them, and the rig is what makes all three
//! measurable, because it forces every legitimate silence to be entered and
//! left in mid-flight while every natural way of stopping happens somewhere
//! else:
//!
//! 1. SPEED AT BOTH EDGES. A crossing begins and ends at cruise: the sensor
//!    goes blind within one report period and comes back within one. A hand
//!    cannot. Even a snappy 30 ms arrest averages about a third of cruise over
//!    its last 10 ms, and a normal one much less, so the reports either side of
//!    a pause carry a deceleration ramp and an acceleration ramp.
//! 2. SAME DIRECTION ACROSS THE SILENCE. This is what putting the slot in the
//!    middle buys. The two places a hand naturally comes to rest are the two
//!    ends of the sweep, and a rest there reverses direction across the
//!    silence. A crossing does not: the mouse goes in one side of the slot and
//!    comes out the other still heading the same way.
//! 3. THE SAME PLACE EVERY TIME. A slot is a fixed obstacle, so crossings
//!    cluster at one distance into the stroke. A habit of pausing does not hold
//!    still to a few millimetres over dozens of sweeps.
//!
//! The turnarounds at the ends of the sweep are not waste: they are the control
//! population. Every one is a full stop taken with the sensor demonstrably on
//! the surface, which is how the detector learns what this user's stops look
//! like instead of assuming.
//!
//! # What is deliberately not done here
//!
//! A silence's duration should scale as 1/speed if it is a fixed-width
//! obstacle, and fitting that would both confirm the slot and recover its
//! width. It is not fitted, because it needs a speed spread the protocol would
//! have to enforce and because the clustering above already answers the
//! question the fit would answer. It is the obvious next thing if this test
//! ever needs to be stronger.
//!
//! One tempting check is absent for a better reason: it is algebraically
//! empty. Dead reckoning loses the slot's width on every traverse, so a
//! forward crossing found at distance `a` and a reverse one at `L_m - a`
//! satisfy `a + (L_m - a) = L_m` identically, whatever the data. It looks like
//! a geometric consistency test and is an identity.

use super::types::{is_monotonic, Report, Verdict, NOT_MONOTONIC, NS};
use super::util;

/// Millimetres per inch.
const MM_PER_IN: f64 = 25.4;

/// What one run at one height concluded about tracking.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LodState {
    /// Nothing usable; read the note.
    Unknown,
    /// The sensor saw the pad through the slot the whole way.
    Tracked,
    /// The sensor lost the pad over the slot, repeatably and in one place.
    Lost,
    /// It lost the pad on some passes and not others: near the threshold.
    Marginal,
}

#[derive(Clone, Debug)]
pub struct LodConfig {
    /// Height of one stack, in millimetres. 0 for the control run.
    pub height_mm: f64,
    /// The slot's width as measured with a ruler, in millimetres.
    pub slot_mm: f64,
    /// Counts per inch the mouse is configured to, for converting to distance.
    pub cpi: f64,
    /// Bin width for the velocity series.
    pub bin_ns: u64,
    pub min_reports: usize,
    pub min_half_strokes: usize,
    pub min_turnarounds: usize,
    /// A silence's edges must both carry at least this fraction of cruise.
    pub edge_speed_frac: f64,
    /// A bin counts as a stroke rather than a wobble above this fraction.
    pub turn_speed_frac: f64,
    /// Cosine between the headings either side of a silence.
    pub codirectional_dot: f64,
    /// Window either side of a silence used to measure heading and speed.
    pub edge_ns: u64,
    /// Dead band between the silence and the edge window, to keep the sensor's
    /// own let-go and re-acquire transient out of the speed estimate.
    pub guard_ns: u64,
    pub lost_fraction: f64,
    pub tracked_fraction: f64,
    pub min_crossings: usize,
    pub min_crossings_per_direction: usize,
    pub min_passes_for_tracked: usize,
}

impl LodConfig {
    pub fn new(height_mm: f64, slot_mm: f64, cpi: f64) -> Self {
        LodConfig {
            height_mm,
            slot_mm,
            cpi,
            // Four reports at 1 kHz and thirty-two at 8 kHz, and it cuts the
            // shortest voluntary stop transient into a dozen pieces, which is
            // the resolution the turnaround detector needs.
            bin_ns: 4_000_000,
            // Only ever fires on a capture that recorded nothing: a real 20 s
            // sweep is one to two orders of magnitude above this.
            min_reports: 256,
            // About five seconds of sweeping inside a twenty second capture, so
            // it refuses idleness without punishing a fumbled first stroke.
            min_half_strokes: 12,
            // The control population. Without it every edge threshold below
            // would revert to a recalled constant instead of this hand's own
            // stops. Twelve half-strokes give at least eleven turnarounds, so
            // this leaves room for three contaminated ones.
            min_turnarounds: 8,
            // Between a hand's fastest arrest (about a third of cruise over the
            // last 10 ms) and a blind sensor's (essentially all of it).
            edge_speed_frac: 0.45,
            turn_speed_frac: 0.5,
            // 37 degrees. Sweep wander is a few degrees and a reversal is 180.
            codirectional_dot: 0.8,
            edge_ns: 10_000_000,
            guard_ns: 2_000_000,
            // Not 1.0: an edge-clipped crossing legitimately fails to qualify,
            // and demanding perfection turns a measurement into a refusal.
            lost_fraction: 0.80,
            tracked_fraction: 0.20,
            min_crossings: 12,
            min_crossings_per_direction: 3,
            // "It tracked", declared from three sweeps, is not a measurement.
            min_passes_for_tracked: 10,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct LodResult {
    pub verdict: Verdict,
    pub state: LodState,
    pub height_mm: f64,
    pub n_reports: usize,
    pub n_half_strokes: usize,
    pub n_turnarounds: usize,
    pub n_silences: usize,
    pub n_crossings: usize,
    pub n_pauses: usize,
    /// Crossings as a share of the passes that could have carried one.
    pub loss_fraction: f64,
    /// Where the obstacle sits along the stroke, in millimetres, and how much
    /// that wandered between crossings.
    pub slot_at_mm: f64,
    pub slot_spread_mm: f64,
    /// Width of the silence in millimetres, from speed and duration. Reported
    /// for corroboration against the ruler, never used to decide anything.
    pub silence_width_mm: f64,
    pub note: &'static str,
}

impl Default for LodState {
    fn default() -> Self {
        LodState::Unknown
    }
}

/// One silence, classified.
struct Silence {
    crossing: bool,
    /// Distance into the current half-stroke, in counts.
    at_counts: f64,
    /// +1 or -1 along the sweep axis.
    dir: i32,
    width_counts: f64,
}

pub fn analyze_lod(reports: &[Report], cfg: &LodConfig) -> LodResult {
    let mut out = LodResult {
        height_mm: cfg.height_mm,
        n_reports: reports.len(),
        ..Default::default()
    };
    if !is_monotonic(reports) {
        out.note = NOT_MONOTONIC;
        return out;
    }
    if reports.len() < cfg.min_reports {
        out.note = "too few reports in this capture to judge anything";
        return out;
    }
    if !(cfg.cpi > 0.0) || !(cfg.slot_mm > 0.0) {
        out.note = "the configured CPI and the slot width are both needed";
        return out;
    }
    let counts_per_mm = cfg.cpi / MM_PER_IN;

    // The sweep axis, from the cumulative path.
    let mut pts = Vec::with_capacity(reports.len());
    let (mut cx, mut cy) = (0.0f64, 0.0f64);
    for r in reports {
        cx += r.dx as f64;
        cy += r.dy as f64;
        pts.push((cx, cy));
    }
    let (_c, u, _res) = util::tls_line(&pts);
    if !u.0.is_finite() || !u.1.is_finite() {
        out.note = "no consistent sweep direction in this capture";
        return out;
    }
    // Signed position along the axis, per report.
    let s: Vec<f64> = pts.iter().map(|p| p.0 * u.0 + p.1 * u.1).collect();

    // Velocity along the axis, binned. Bins rather than per-report differences
    // because a single report at 8 kHz carries too little displacement to give
    // a stable heading.
    let t0 = reports[0].t_ns;
    let n_bins = (((reports[reports.len() - 1].t_ns - t0) / cfg.bin_ns) + 1) as usize;
    let mut bin_ds = vec![0.0f64; n_bins];
    let mut bin_n = vec![0usize; n_bins];
    for (i, r) in reports.iter().enumerate() {
        let b = ((r.t_ns - t0) / cfg.bin_ns) as usize;
        let prev = if i == 0 { 0.0 } else { s[i - 1] };
        bin_ds[b] += s[i] - prev;
        bin_n[b] += 1;
    }
    let bin_s = cfg.bin_ns as f64 / NS;
    let va: Vec<f64> = bin_ds.iter().map(|d| d / bin_s).collect();

    let speeds: Vec<f64> = va
        .iter()
        .zip(&bin_n)
        .filter(|(_, n)| **n > 0)
        .map(|(v, _)| v.abs())
        .collect();
    if speeds.is_empty() {
        out.note = "no motion in this capture";
        return out;
    }
    // A percentile, not the max, so one bin cannot set the scale; the 75th
    // rather than the median because a shuttle's median includes its turns.
    let v_cruise = util::percentile(&speeds, 0.75);
    if !(v_cruise > 0.0) {
        out.note = "no motion in this capture";
        return out;
    }

    // Turnarounds: a sign change with real speed sustained either side.
    let strong: Vec<i32> = va
        .iter()
        .map(|v| {
            if v.abs() >= cfg.turn_speed_frac * v_cruise {
                if *v > 0.0 {
                    1
                } else {
                    -1
                }
            } else {
                0
            }
        })
        .collect();
    let mut runs: Vec<(i32, usize, usize)> = Vec::new();
    for (i, &d) in strong.iter().enumerate() {
        if d == 0 {
            continue;
        }
        match runs.last_mut() {
            Some(last) if last.0 == d && i == last.2 + 1 => last.2 = i,
            _ => runs.push((d, i, i)),
        }
    }
    // Three bins is twelve milliseconds, so a single noisy bin can neither
    // manufacture a turnaround nor erase one.
    let kept: Vec<(i32, usize, usize)> =
        runs.into_iter().filter(|r| r.2 - r.1 + 1 >= 3).collect();
    // Coalesce same-direction runs. The silence this test is looking for splits
    // a half-stroke in two without turning it round, so counting the pieces
    // would count every crossed stroke twice and halve the loss fraction: a
    // sensor that failed on every pass would be reported as failing on half of
    // them. A half-stroke is a maximal excursion in one direction, silences and
    // all.
    let mut strokes: Vec<(i32, usize, usize)> = Vec::with_capacity(kept.len());
    for st in kept {
        match strokes.last_mut() {
            Some(last) if last.0 == st.0 => last.2 = st.2,
            _ => strokes.push(st),
        }
    }
    let mut n_turn = 0usize;
    for w in strokes.windows(2) {
        if w[0].0 != w[1].0 {
            n_turn += 1;
        }
    }
    out.n_turnarounds = n_turn;
    out.n_half_strokes = strokes.len();
    if strokes.len() < cfg.min_half_strokes {
        out.note = "too few sweeps: keep sweeping back and forth for the whole recording";
        return out;
    }
    if n_turn < cfg.min_turnarounds {
        out.note = "too few turns to learn what your own stops look like; sweep back and \
                    forth rather than in one direction";
        return out;
    }

    // Silences. A gap is interior if motion happened both before and after it.
    let mut dts: Vec<f64> = reports
        .windows(2)
        .map(|w| (w[1].t_ns - w[0].t_ns) as f64)
        .collect();
    dts.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let t_med = util::median(&dts);
    // Five dropped slots at 1 kHz; the 4 ms floor binds at 8 kHz. This is only
    // defensible because the control run at zero height is required to contain
    // no codirectional silence this long at the same speeds.
    let g_min = (5.0 * t_med).max(4.0e6);

    let first_move = reports.iter().position(|r| r.is_moving()).unwrap_or(0);
    let last_move = reports.iter().rposition(|r| r.is_moving()).unwrap_or(0);

    let mut silences: Vec<Silence> = Vec::new();
    let mut worst_gap = 0.0f64;
    for i in 0..reports.len().saturating_sub(1) {
        if i < first_move || i + 1 > last_move {
            continue;
        }
        let gap = (reports[i + 1].t_ns - reports[i].t_ns) as f64;
        if gap < g_min {
            continue;
        }
        worst_gap = worst_gap.max(gap);
        let pre = edge_window(reports, &s, i, cfg, false);
        let post = edge_window(reports, &s, i + 1, cfg, true);
        let (Some(pre), Some(post)) = (pre, post) else {
            // Not enough either side to say anything about it.
            continue;
        };
        let dot = pre.ux * post.ux + pre.uy * post.uy;
        let fast = pre.speed >= cfg.edge_speed_frac * v_cruise
            && post.speed >= cfg.edge_speed_frac * v_cruise;
        let crossing = fast && dot >= cfg.codirectional_dot;
        // Distance into the current half-stroke, so a fixed obstacle lands at
        // the same figure every sweep whatever the sweep's absolute position.
        let dir = if pre.along >= 0.0 { 1 } else { -1 };
        let start = stroke_start(&strokes, reports, &s, t0, cfg, i);
        let at = (s[i] - s[start]).abs();
        silences.push(Silence {
            crossing,
            at_counts: at,
            dir,
            width_counts: 0.5 * (pre.speed + post.speed) * (gap / NS),
        });
    }

    out.n_silences = silences.len();
    out.n_crossings = silences.iter().filter(|x| x.crossing).count();
    out.n_pauses = silences.len() - out.n_crossings;

    // A gap far longer than the others is a disconnect, not a crossing, and it
    // must never be turned into a number. This is the fault the whole session
    // that produced this test kept running into.
    let clean: Vec<f64> = silences
        .iter()
        .filter(|x| x.crossing)
        .map(|x| x.width_counts)
        .collect();
    if worst_gap > 1.5e9 {
        out.note = "the reports stopped for more than a second and a half during this run, \
                    which is a disconnection rather than a lift-off. Nothing is reported \
                    from it";
        return out;
    }

    // Passes that could have carried a crossing. Every half-stroke crosses the
    // slot, because the slot is in the middle and the turns are beyond it.
    let passes = strokes.len();
    out.loss_fraction = (out.n_crossings as f64 / passes as f64).min(1.0);

    if out.n_crossings == 0 {
        if passes >= cfg.min_passes_for_tracked {
            out.state = LodState::Tracked;
            out.verdict = Verdict::Pass;
            out.note = "the sensor saw the pad through the slot on every pass";
        } else {
            out.note = "too few passes over the slot to say it tracked";
        }
        return out;
    }

    // Where the obstacle is, per direction, and how tightly it repeats.
    let fwd: Vec<f64> = silences
        .iter()
        .filter(|x| x.crossing && x.dir > 0)
        .map(|x| x.at_counts / counts_per_mm)
        .collect();
    let rev: Vec<f64> = silences
        .iter()
        .filter(|x| x.crossing && x.dir < 0)
        .map(|x| x.at_counts / counts_per_mm)
        .collect();
    let all: Vec<f64> = fwd.iter().chain(&rev).copied().collect();
    out.slot_at_mm = util::median(&all);
    out.slot_spread_mm = util::mad_sigma(&all);
    if !clean.is_empty() {
        out.silence_width_mm = util::median(&clean) / counts_per_mm;
    }

    if out.loss_fraction <= cfg.tracked_fraction {
        if passes >= cfg.min_passes_for_tracked {
            out.state = LodState::Tracked;
            out.verdict = Verdict::Pass;
            out.note = "the sensor kept the pad on nearly every pass; the few silences did \
                        not repeat in one place";
        } else {
            out.note = "too few passes over the slot to say it tracked";
        }
        return out;
    }

    if out.n_crossings < cfg.min_crossings {
        out.note = "too few clean crossings to conclude the sensor lost the pad; sweep for \
                    the whole recording without stopping in the middle";
        return out;
    }
    if fwd.len() < cfg.min_crossings_per_direction || rev.len() < cfg.min_crossings_per_direction {
        // A fixed obstacle is crossed both ways. Silences in one direction only
        // are what a habit looks like, not what a slot looks like.
        out.note = "the silences appear in one direction only, which is a habit rather than \
                    something fixed on the desk";
        return out;
    }
    // A slot does not move between sweeps; a pause does. This is the check
    // that separates a repeatable obstacle from a repeated habit.
    let spread_limit = (3.0f64).max(0.25 * cfg.slot_mm);
    if out.slot_spread_mm > spread_limit {
        out.note = "the silences did not fall in the same place twice, so they are not a \
                    fixed obstacle on the desk";
        return out;
    }
    if out.n_pauses > out.n_crossings {
        out.note = "more of the silences look like stops than like crossings; sweep without \
                    pausing in the middle";
        return out;
    }

    if out.loss_fraction >= cfg.lost_fraction {
        out.state = LodState::Lost;
        out.verdict = Verdict::Pass;
        out.note = "the sensor lost the pad over the slot, in the same place, both ways";
    } else {
        out.state = LodState::Marginal;
        out.verdict = Verdict::Pass;
        out.note = "the sensor lost the pad on some passes and not others, so this height is \
                    close to the threshold";
    }
    out
}

struct Edge {
    ux: f64,
    uy: f64,
    speed: f64,
    along: f64,
}

/// Heading and speed just before or just after a silence, with a guard band so
/// the sensor's own let-go and re-acquire transient stays out of it.
fn edge_window(
    reports: &[Report],
    s: &[f64],
    idx: usize,
    cfg: &LodConfig,
    forward: bool,
) -> Option<Edge> {
    let anchor = reports[idx].t_ns;
    let (lo, hi) = if forward {
        (
            anchor.saturating_add(cfg.guard_ns),
            anchor.saturating_add(cfg.guard_ns + cfg.edge_ns),
        )
    } else {
        (
            anchor.saturating_sub(cfg.guard_ns + cfg.edge_ns),
            anchor.saturating_sub(cfg.guard_ns),
        )
    };
    let mut dx = 0.0f64;
    let mut dy = 0.0f64;
    let mut path = 0.0f64;
    let mut n = 0usize;
    let mut i0 = usize::MAX;
    let mut i1 = 0usize;
    for (i, r) in reports.iter().enumerate() {
        if r.t_ns < lo || r.t_ns > hi {
            continue;
        }
        dx += r.dx as f64;
        dy += r.dy as f64;
        path += r.mag();
        n += 1;
        i0 = i0.min(i);
        i1 = i1.max(i);
    }
    if n < 3 {
        return None;
    }
    let mag = (dx * dx + dy * dy).sqrt();
    if mag <= 0.0 {
        return None;
    }
    let span = (hi - lo) as f64 / NS;
    Some(Edge {
        ux: dx / mag,
        uy: dy / mag,
        speed: path / span,
        along: s[i1] - s[i0],
    })
}

/// Index of the report where the half-stroke containing `idx` turned round.
///
/// The turn, not the point where the stroke first got up to speed. A stroke
/// accelerates out of its turn, so its first bins fall below the strong-motion
/// threshold and are not part of any run; measuring from the first strong bin
/// would put the obstacle a centimetre closer to the turn than it is. The
/// clustering does not care, since a constant offset cancels, but the figure is
/// shown to someone who has a ruler on the desk and can check it.
fn stroke_start(
    strokes: &[(i32, usize, usize)],
    reports: &[Report],
    s: &[f64],
    t0: u64,
    cfg: &LodConfig,
    idx: usize,
) -> usize {
    let bin = ((reports[idx].t_ns - t0) / cfg.bin_ns) as usize;
    let k = strokes.iter().rposition(|st| st.1 <= bin).unwrap_or(0);
    if k == 0 {
        return 0;
    }
    // The turn is where the position reaches its extreme, not where the
    // previous stroke stopped counting as fast. A minimum-jerk stroke coasts a
    // tenth of its length below the strong-motion threshold on the way in, so
    // taking the last strong bin puts every crossing that much nearer the turn
    // than it is.
    let idx_at = |b: usize| -> usize {
        let want = t0 + b as u64 * cfg.bin_ns;
        reports.iter().position(|r| r.t_ns >= want).unwrap_or(0)
    };
    let a = idx_at(strokes[k - 1].2);
    let b = idx_at(strokes[k].1).max(a).min(reports.len() - 1);
    let prev_dir = strokes[k - 1].0 as f64;
    let mut best = a;
    for j in a..=b {
        if prev_dir * s[j] > prev_dir * s[best] {
            best = j;
        }
    }
    best.min(idx)
}

/// One height that has been run.
#[derive(Clone, Copy, Debug)]
pub struct LodRung {
    pub height_mm: f64,
    pub state: LodState,
}

#[derive(Clone, Debug, Default)]
pub struct LodSummary {
    pub verdict: Verdict,
    /// The tallest stack that still tracked, and the shortest that did not.
    pub tracked_to_mm: f64,
    pub lost_at_mm: f64,
    pub bracket_mm: f64,
    pub note: &'static str,
}

/// The bracket, which is the only thing a ladder of runs can honestly report.
///
/// There is no single number to give: the answer is a pair of heights, and the
/// gap between them can never be narrower than the step between the stacks the
/// user owns. Reporting a midpoint as "the" lift-off distance would invent a
/// precision the cards cannot carry.
pub fn summarize_lod(rungs: &[LodRung], claimed_mm: Option<f64>) -> LodSummary {
    let mut out = LodSummary::default();
    // The control proves the sweep, the link and the speed, and says nothing
    // about the mouse. Without it the silence threshold has nothing behind it.
    let control = rungs
        .iter()
        .find(|r| r.height_mm <= 0.0 && r.state == LodState::Tracked);
    if control.is_none() {
        out.note = "run the control first, with the cards taken away. Until it passes, a \
                    silence at any height has nothing to be compared against";
        return out;
    }
    let tracked = rungs
        .iter()
        .filter(|r| r.state == LodState::Tracked && r.height_mm > 0.0)
        .map(|r| r.height_mm)
        .fold(f64::NEG_INFINITY, f64::max);
    let lost = rungs
        .iter()
        .filter(|r| r.state == LodState::Lost)
        .map(|r| r.height_mm)
        .fold(f64::INFINITY, f64::min);
    if !lost.is_finite() {
        out.note = "no height tested has lost tracking yet; add cards and run it again";
        return out;
    }
    if !tracked.is_finite() {
        out.note = "no height above zero has tracked yet; take cards away and run it again";
        return out;
    }
    if lost <= tracked {
        // A taller stack tracked than a shorter one did not. The rig moved, or
        // the runs were not all measuring the same thing.
        out.note = "these runs disagree: a taller stack tracked than a shorter one did not. \
                    Check the rig has not moved and run them again";
        return out;
    }
    out.tracked_to_mm = tracked;
    out.lost_at_mm = lost;
    out.bracket_mm = lost - tracked;
    out.verdict = Verdict::Pass;
    out.note = "the lift-off distance is between these two heights";

    if let Some(c) = claimed_mm {
        if c > 0.0 {
            // 0.3 mm is dominated by the surface: the same mouse legitimately
            // reads differently on a cloth pad that compresses under the cards
            // than on glass that does not. Inside that, the setting is the same
            // setting on a different surface, and calling it a lie would be a
            // false alarm.
            let tol = 0.3;
            if c + tol < tracked {
                out.verdict = Verdict::Fail;
                out.note = "the mouse tracked well above the height it is set to";
            } else if c - tol > lost {
                out.verdict = Verdict::Fail;
                out.note = "the mouse lost the pad well below the height it is set to";
            }
        }
    }
    out
}
