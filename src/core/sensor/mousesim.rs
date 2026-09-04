//! Ground-truth simulator for the sensor detectors. Test builds only.
//!
//! This is the reference every threshold in this module is checked against,
//! because the alternative is asserting that a detector agrees with itself.
//! It models the chain in the order the hardware actually has it:
//!
//!   hand trajectory (continuous, with wander and physiological tremor)
//!     -> sensor noise
//!     -> firmware angle snap
//!     -> firmware EMA on velocity
//!     -> tracking loss above a maximum speed
//!     -> residual-accumulator quantiser to integer counts
//!     -> report-field clipping
//!     -> dropped polling slots (motion coalesced into the next report)
//!     -> timestamp jitter
//!
//! THE ORDER IS THE WHOLE POINT and it is easy to get wrong. Firmware is
//! downstream of the sensor, so it snaps and smooths an estimate that is
//! ALREADY noisy. Put the snap before the noise and the perpendicular noise
//! survives the projection, so the anisotropy detector reads 1.00 for clean
//! and snapped alike and has exactly zero power while appearing to work.
//!
//! SENSOR NOISE IS A POSITION ERROR, NOT A DELTA ERROR. An optical sensor
//! estimates displacement by correlating successive images, so what carries the
//! error is the position estimate, and the delta on the wire is the DIFFERENCE
//! of two of them. Getting this structure right settles two questions at once.
//!
//! It explains why a still mouse is quiet WITHOUT switching the sensor off.
//! The surface under a stationary sensor does not change, so consecutive
//! estimates share their error and the difference nearly cancels. The earlier
//! version of this file wrote that as `if moving_truth { ...add noise... }`,
//! which produces the right silence for the wrong reason, and that one
//! condition hid a real defect: with the sensor switched off at rest, an
//! unfiltered mouse showed a 0 ms interpolation tail, so a tail statistic with
//! no termination rule at all looked correct. Modelling it as white noise
//! straight into the quantiser is wrong in the other direction, and worse: the
//! accumulated position then random-walks away, inventing 43 counts of travel
//! over 15 stationary seconds at 1 kHz, which is a defect no real mouse has.
//!
//! It also explains why the emitted noise is ANTI-correlated at lag 1, which
//! pushes a clean mouse's smoothing statistic negative and makes that test
//! conservative rather than trigger-happy.
//!
//! The occasional spurious +-1 a real sensor genuinely does emit while still is
//! a separate mechanism, modelled explicitly by `idle_jitter_cps`. That is the
//! knob the smoothing regression test turns.

use super::types::{Report, NS};
use crate::core::sim::Rng;

#[derive(Clone, Debug)]
pub struct MouseSim {
    pub cpi: f64,
    pub poll_hz: f64,
    /// Per-report sensor noise at 1 kHz, in counts, per axis. Scaled by sqrt(dt*1000).
    pub noise_counts_1khz: f64,
    /// Constant velocity bias while stationary, counts/second: the drift defect.
    pub drift_cps: (f64, f64),
    /// Whole-count spurious reports while still, counts/second: the jitter defect.
    pub idle_jitter_cps: f64,
    /// EMA alpha applied to velocity. 1.0 = no smoothing.
    pub ema_alpha: f64,
    /// Angle snapping: fraction of the perpendicular component that survives.
    /// 1.0 = none, 0.0 = perfectly straight output.
    pub snap_perp_keep: f64,
    /// Snap onto the nearest 45-degree octant instead of the stroke's direction.
    pub snap_to_octant: bool,
    /// Report-field bound, e.g. 127. None = unlimited.
    pub field_bound: Option<i32>,
    /// Speed above which tracking degrades, inches/second. None = perfect.
    pub max_track_ips: Option<f64>,
    /// Probability a polling slot is missed (motion coalesces into the next report).
    pub drop_prob: f64,
    /// 1-sigma timestamp jitter, seconds.
    pub ts_jitter_s: f64,
    /// Devices only report when there is motion. True is realistic.
    pub suppress_idle_reports: bool,
}

impl Default for MouseSim {
    fn default() -> Self {
        MouseSim {
            cpi: 1600.0,
            poll_hz: 1000.0,
            noise_counts_1khz: 0.35,
            drift_cps: (0.0, 0.0),
            idle_jitter_cps: 0.0,
            ema_alpha: 1.0,
            snap_perp_keep: 1.0,
            snap_to_octant: false,
            field_bound: None,
            max_track_ips: None,
            drop_prob: 0.0,
            ts_jitter_s: 20.0e-6,
            suppress_idle_reports: true,
        }
    }
}

/// A continuous hand trajectory: position in INCHES against time in seconds.
pub trait Traj {
    fn pos(&self, t: f64) -> (f64, f64);
    fn duration(&self) -> f64;
}

/// Back-and-forth sweeping along one axis, for the lift-off rig.
///
/// The lift-off test needs many traversals of a fixed point on the desk rather
/// than one stroke, and it needs the turns to be real stops, because the turns
/// are the control population the detector learns the user's own stops from.
/// Each half-stroke is a minimum-jerk move, so the ends decelerate and
/// accelerate the way a hand does.
#[derive(Clone, Debug)]
pub struct ShuttleTraj {
    pub len_in: f64,
    pub dir_rad: f64,
    /// Seconds per half-stroke.
    pub half_s: f64,
    pub n_half: usize,
    pub tremor_in: f64,
    pub tremor_hz: f64,
    /// Where each turn actually happens, in inches from the nominal end. A hand
    /// does not reverse in the same place twice, and a fixture that does hides
    /// every defect in a detector that measures position from the turn.
    pub end_jitter_in: f64,
    pub ph: [f64; 2],
}

impl ShuttleTraj {
    pub fn new(rng: &mut Rng, len_in: f64, half_s: f64, n_half: usize) -> Self {
        ShuttleTraj {
            len_in,
            dir_rad: 0.0,
            half_s,
            n_half,
            tremor_in: 0.012,
            tremor_hz: 10.0,
            end_jitter_in: 0.0,
            ph: [rng.unit() * std::f64::consts::TAU, rng.unit() * std::f64::consts::TAU],
        }
    }
}

impl Traj for ShuttleTraj {
    fn duration(&self) -> f64 {
        self.half_s * self.n_half as f64
    }
    fn pos(&self, t: f64) -> (f64, f64) {
        let t = t.clamp(0.0, self.duration());
        let k = ((t / self.half_s) as usize).min(self.n_half.saturating_sub(1));
        let local = (t - k as f64 * self.half_s) / self.half_s;
        // Minimum-jerk: still at both ends, so every turn is a genuine stop.
        let f = 10.0 * local.powi(3) - 15.0 * local.powi(4) + 6.0 * local.powi(5);
        // Each end wanders, deterministically per stroke so a run is repeatable.
        let jit = |i: usize| -> f64 {
            if self.end_jitter_in == 0.0 {
                return 0.0;
            }
            self.end_jitter_in
                * ((i as f64 * 12.9898 + self.ph[1]).sin() * 43758.5453).fract().mul_add(2.0, -1.0)
        };
        let (a, b) = if k % 2 == 0 {
            (jit(k), self.len_in + jit(k + 1))
        } else {
            (self.len_in + jit(k), jit(k + 1))
        };
        let along = a + f * (b - a);
        let tr = self.tremor_in
            * (std::f64::consts::TAU * self.tremor_hz * t + self.ph[0]).sin();
        let (c, s) = (self.dir_rad.cos(), self.dir_rad.sin());
        (along * c - tr * s, along * s + tr * c)
    }
}

/// Straight swipe with a minimum-jerk speed profile, plus wander and tremor.
#[derive(Clone, Debug)]
pub struct SwipeTraj {
    pub len_in: f64,
    pub dir_rad: f64,
    pub dur_s: f64,
    /// RMS perpendicular hand wander as a fraction of length (0.015 = 1.5%).
    pub wander_frac: f64,
    /// Physiological tremor amplitude, inches (0.3 mm = 0.012 in is typical).
    pub tremor_in: f64,
    pub tremor_hz: f64,
    pub ph: [f64; 4],
    /// Dead time at each end where the hand is not moving.
    pub pad_s: f64,
}

impl SwipeTraj {
    pub fn new(rng: &mut Rng, len_in: f64, dur_s: f64, dir_rad: f64) -> Self {
        let mut ph = [0.0f64; 4];
        for p in ph.iter_mut() {
            *p = rng.unit() * std::f64::consts::TAU;
        }
        SwipeTraj {
            len_in,
            dir_rad,
            dur_s,
            wander_frac: 0.015,
            tremor_in: 0.012,
            tremor_hz: 10.0,
            ph,
            pad_s: 0.15,
        }
    }
}

impl Traj for SwipeTraj {
    fn pos(&self, t: f64) -> (f64, f64) {
        let tm = (t - self.pad_s).clamp(0.0, self.dur_s);
        let s = (tm / self.dur_s).clamp(0.0, 1.0);
        // minimum-jerk: 10 s^3 - 15 s^4 + 6 s^5
        let f = 10.0 * s.powi(3) - 15.0 * s.powi(4) + 6.0 * s.powi(5);
        let along = f * self.len_in;
        // Perpendicular hand wander: three low-frequency lobes over the stroke,
        // with amplitudes set so the RMS is about wander_frac * len.
        let a = self.wander_frac * self.len_in * std::f64::consts::SQRT_2;
        let w = a
            * (0.62 * (std::f64::consts::PI * s + self.ph[0]).sin()
                + 0.30 * (2.0 * std::f64::consts::PI * s + self.ph[1]).sin()
                + 0.16 * (3.0 * std::f64::consts::PI * s + self.ph[2]).sin());
        // Tremor rides on top and does not stop at the ends.
        let trem = self.tremor_in
            * (std::f64::consts::TAU * self.tremor_hz * t + self.ph[3]).sin()
            * if tm > 0.0 && tm < self.dur_s { 1.0 } else { 0.3 };
        let perp = w + trem;
        let (c, si) = (self.dir_rad.cos(), self.dir_rad.sin());
        (along * c - perp * si, along * si + perp * c)
    }
    fn duration(&self) -> f64 {
        self.dur_s + 2.0 * self.pad_s
    }
}

/// Constant-velocity glide ended by an ABRUPT stop: the mouse hits the edge of
/// the pad or a book. This is the protocol the smoothing test asks for, because
/// it removes the "was that the hand or the firmware decelerating?" confound
/// physically rather than statistically. After the stop the true motion is
/// exactly zero, so every remaining count is firmware or sensor noise.
#[derive(Clone, Debug)]
pub struct HardStopTraj {
    pub ips: f64,
    pub glide_s: f64,
    pub after_s: f64,
    pub dir_rad: f64,
    pub tremor_in: f64,
    pub ph: f64,
}

impl Traj for HardStopTraj {
    fn pos(&self, t: f64) -> (f64, f64) {
        let tm = t.min(self.glide_s);
        // Ease in over the first 20% so the start is not a delta function.
        let ramp = (tm / (0.2 * self.glide_s)).min(1.0);
        let d = self.ips * (tm - 0.1 * self.glide_s * (1.0 - (1.0 - ramp).powi(2)));
        let trem = if t < self.glide_s {
            self.tremor_in * (std::f64::consts::TAU * 10.0 * t + self.ph).sin()
        } else {
            0.0
        };
        (
            d * self.dir_rad.cos() - trem * self.dir_rad.sin(),
            d * self.dir_rad.sin() + trem * self.dir_rad.cos(),
        )
    }
    fn duration(&self) -> f64 {
        self.glide_s + self.after_s
    }
}

/// A stationary hand: the mouse does not move at all.
#[derive(Clone, Debug)]
pub struct StillTraj {
    pub dur_s: f64,
}

impl Traj for StillTraj {
    fn pos(&self, _t: f64) -> (f64, f64) {
        (0.0, 0.0)
    }
    fn duration(&self) -> f64 {
        self.dur_s
    }
}

/// Several swipes of increasing speed, back and forth, for the tracking test.
#[derive(Clone, Debug)]
pub struct RampTraj {
    /// (length inches, duration seconds) per swipe.
    pub swipes: Vec<(f64, f64)>,
    pub gap_s: f64,
    pub ph: f64,
}

impl Traj for RampTraj {
    fn pos(&self, t: f64) -> (f64, f64) {
        let mut acc = 0.0f64;
        let mut x = 0.0f64;
        for (i, (l, d)) in self.swipes.iter().enumerate() {
            let dir = if i % 2 == 0 { 1.0 } else { -1.0 };
            let t0 = acc;
            let t1 = acc + d;
            if t < t0 {
                return (x, 0.0);
            }
            if t <= t1 {
                let s = ((t - t0) / d).clamp(0.0, 1.0);
                let f = 10.0 * s.powi(3) - 15.0 * s.powi(4) + 6.0 * s.powi(5);
                let y = 0.010 * (std::f64::consts::TAU * 9.0 * t + self.ph).sin();
                return (x + dir * f * l, y);
            }
            x += dir * l;
            acc = t1 + self.gap_s;
        }
        (x, 0.0)
    }
    fn duration(&self) -> f64 {
        self.swipes.iter().map(|(_, d)| d + self.gap_s).sum::<f64>() + 0.2
    }
}

struct SnapState {
    ax: f64,
    ay: f64,
    lx: f64,
    ly: f64,
}

impl MouseSim {
    /// Render a trajectory to a report stream.
    pub fn render<T: Traj>(&self, traj: &T, rng: &mut Rng) -> Vec<Report> {
        let dt = 1.0 / self.poll_hz;
        let n = (traj.duration() / dt).ceil() as usize;
        let noise_scale = self.noise_counts_1khz * (dt * 1000.0).sqrt();

        // quantiser residual accumulator
        let (mut resx, mut resy) = (0.0f64, 0.0f64);
        // Sensor position-estimate error, per axis. Persistent across frames.
        let (mut ex, mut ey) = (0.0f64, 0.0f64);
        // Emitted noise is a first difference of this, so scale it to keep
        // `noise_counts_1khz` meaning the per-report noise that reaches the wire.
        let sigma_pos = noise_scale / std::f64::consts::SQRT_2;
        // EMA state, counts per report
        let (mut vx, mut vy) = (0.0f64, 0.0f64);
        let mut snap = SnapState { ax: 0.0, ay: 0.0, lx: 0.0, ly: 0.0 };
        // motion held over a dropped slot
        let (mut pend_x, mut pend_y) = (0i32, 0i32);
        let mut pending = false;

        let mut out: Vec<Report> = Vec::with_capacity(n);
        let mut prev = traj.pos(0.0);
        for k in 1..=n {
            let t = k as f64 * dt;
            let cur = traj.pos(t);
            let mut mx = (cur.0 - prev.0) * self.cpi;
            let mut my = (cur.1 - prev.1) * self.cpi;
            prev = cur;

            let moving_truth = mx.abs() + my.abs() > 1e-9;
            // Whole-count idle jitter, added after the quantiser.
            let (mut jx, mut jy) = (0i32, 0i32);

            if !moving_truth {
                mx += self.drift_cps.0 * dt;
                my += self.drift_cps.1 * dt;
                // Idle jitter is emitted as WHOLE counts, because that is what a
                // real sensor does: it occasionally reports a spurious +-1.
                // Modelling it as sub-count noise into the accumulator would be
                // wrong, since the accumulator would swallow it and nothing
                // would ever reach the wire.
                if self.idle_jitter_cps > 0.0 {
                    let p = (self.idle_jitter_cps * dt / 2.0).min(1.0);
                    if rng.unit() < p {
                        jx += if rng.unit() < 0.5 { 1 } else { -1 };
                    }
                    if rng.unit() < p {
                        jy += if rng.unit() < 0.5 { 1 } else { -1 };
                    }
                }
            }

            // Sensor noise, BEFORE both firmware stages, because firmware is
            // downstream of the sensor and smooths an already-noisy estimate.
            // Modelled as a position error that the wire sees differenced; see
            // the module header. The correlation is high while the image under
            // the sensor is unchanged and drops to nothing once it moves.
            let rho: f64 = if moving_truth { 0.0 } else { 0.98 };
            let drive = (1.0 - rho * rho).sqrt() * sigma_pos;
            let nx = rho * ex + drive * rng.normal();
            let ny = rho * ey + drive * rng.normal();
            mx += nx - ex;
            my += ny - ey;
            ex = nx;
            ey = ny;

            // Firmware angle snapping. Real implementations accumulate the
            // stroke vector and force each new increment onto that direction.
            if self.snap_perp_keep < 1.0 && (mx.abs() + my.abs()) > 1e-9 {
                snap.ax += mx;
                snap.ay += my;
                let amag = (snap.ax * snap.ax + snap.ay * snap.ay).sqrt();
                let d = if self.snap_to_octant {
                    let a = if amag > 5.0 {
                        snap.ay.atan2(snap.ax)
                    } else {
                        my.atan2(mx)
                    };
                    let q = (a / std::f64::consts::FRAC_PI_4).round() * std::f64::consts::FRAC_PI_4;
                    Some((q.cos(), q.sin()))
                } else if amag > 200.0 {
                    // Latch. Real snappers fix a reference direction once the
                    // stroke is long enough for it to be stable; a continuously
                    // rotating reference would trace a curve, which is not what
                    // they do.
                    if snap.lx == 0.0 && snap.ly == 0.0 {
                        snap.lx = snap.ax / amag;
                        snap.ly = snap.ay / amag;
                    }
                    Some((snap.lx, snap.ly))
                } else {
                    None
                };
                if let Some((ux, uy)) = d {
                    let along = mx * ux + my * uy;
                    let (px, py) = (-uy, ux);
                    let perp = mx * px + my * py;
                    let kept = perp * self.snap_perp_keep;
                    mx = along * ux + kept * px;
                    my = along * uy + kept * py;
                }
            }

            // Firmware EMA on velocity.
            if self.ema_alpha < 1.0 {
                vx = (1.0 - self.ema_alpha) * vx + self.ema_alpha * mx;
                vy = (1.0 - self.ema_alpha) * vy + self.ema_alpha * my;
                mx = vx;
                my = vy;
            }

            // Tracking loss.
            if let Some(vmax) = self.max_track_ips {
                let speed_ips = (mx * mx + my * my).sqrt() / self.cpi / dt;
                if speed_ips > vmax {
                    // Beyond the correlation window the sensor recovers a
                    // shrinking fraction of the true motion and sometimes
                    // inverts. This shape is a model, not a measurement; the
                    // recovered speed is only good to about +-15% because of it.
                    let over = speed_ips / vmax;
                    let frac = (1.0 / over).powf(2.0);
                    mx *= frac;
                    my *= frac;
                    if rng.unit() < 0.25 {
                        mx = -mx;
                        my = -my;
                    }
                }
            }

            // Residual-accumulator quantiser.
            resx += mx;
            resy += my;
            let mut qx = resx.trunc() as i32;
            let mut qy = resy.trunc() as i32;
            resx -= qx as f64;
            resy -= qy as f64;
            qx += jx;
            qy += jy;
            jx = 0;
            jy = 0;
            let _ = (jx, jy);

            // Report-field clipping.
            if let Some(b) = self.field_bound {
                if qx > b {
                    resx += (qx - b) as f64;
                    qx = b;
                }
                if qx < -b {
                    resx += (qx + b) as f64;
                    qx = -b;
                }
                if qy > b {
                    resy += (qy - b) as f64;
                    qy = b;
                }
                if qy < -b {
                    resy += (qy + b) as f64;
                    qy = -b;
                }
                // A real device that clips usually LOSES the excess. Pushing it
                // back into the residual is the harder case for the detector, so
                // passing here is a conservative result.
            }

            // Dropped slot: coalesce into the next report.
            if self.drop_prob > 0.0 && rng.unit() < self.drop_prob {
                pend_x += qx;
                pend_y += qy;
                pending = true;
                continue;
            }
            if pending {
                qx += pend_x;
                qy += pend_y;
                pend_x = 0;
                pend_y = 0;
                pending = false;
            }

            if self.suppress_idle_reports && qx == 0 && qy == 0 {
                continue;
            }
            let tj = (t + self.ts_jitter_s * rng.normal()).max(0.0);
            out.push(Report::motion((tj * NS) as u64, qx, qy));
        }
        // Timestamp jitter can reorder adjacent reports; a real capture layer
        // delivers them in arrival order, so restore that here.
        out.sort_by_key(|r| r.t_ns);
        out
    }
}

// ---------------------------------------------------------------- scroll

/// Wheel simulator. The vertical and horizontal encoders behave the same way,
/// so one model covers both.
#[derive(Clone, Debug)]
pub struct ScrollSim {
    /// Counts emitted per detent in total.
    pub counts_per_detent: i32,
    /// Split each detent across this many reports, as a high-resolution wheel
    /// does.
    pub reports_per_detent: usize,
    /// Mean time between detents, ms.
    pub cadence_ms: f64,
    pub cadence_jitter_ms: f64,
    /// Probability a detent comes out with the wrong sign.
    pub reverse_prob: f64,
    /// Probability a detent comes out as two steps.
    pub skip_prob: f64,
    /// Free-spin: a smoothly varying magnitude with no quantum at all.
    pub continuous: bool,
    /// Emit on the horizontal wheel instead of the vertical one.
    pub horizontal: bool,
}

impl Default for ScrollSim {
    fn default() -> Self {
        ScrollSim {
            counts_per_detent: 1,
            reports_per_detent: 1,
            cadence_ms: 90.0,
            cadence_jitter_ms: 25.0,
            reverse_prob: 0.0,
            skip_prob: 0.0,
            continuous: false,
            horizontal: false,
        }
    }
}

impl ScrollSim {
    pub fn render(&self, n_detents: usize, dir: i32, rng: &mut Rng) -> Vec<Report> {
        let mut t = 10.0f64; // ms
        let mut out = Vec::new();
        let emit = |t_ms: f64, v: i32, out: &mut Vec<Report>| {
            let t_ns = (t_ms * 1.0e6) as u64;
            out.push(if self.horizontal {
                Report { t_ns, dx: 0, dy: 0, wheel: 0, hwheel: v }
            } else {
                Report::wheel_ev(t_ns, v)
            });
        };
        for _ in 0..n_detents {
            t += (self.cadence_ms + self.cadence_jitter_ms * rng.normal()).max(20.0);
            let mut sign = dir;
            if rng.unit() < self.reverse_prob {
                sign = -sign;
            }
            let mult = if rng.unit() < self.skip_prob { 2 } else { 1 };
            if self.continuous {
                let m = (3.0 + 60.0 * rng.unit()).round() as i32;
                emit(t, sign * m, &mut out);
                continue;
            }
            let total = self.counts_per_detent * mult;
            let n = self.reports_per_detent.max(1);
            let per = (total / n as i32).max(1);
            let mut left = total;
            for i in 0..n {
                let v = if i == n - 1 { left } else { per };
                left -= v;
                if v == 0 {
                    continue;
                }
                // Sub-reports inside one detent arrive about a millisecond
                // apart, well inside the time the wheel takes to snap over the
                // notch.
                emit(t + i as f64, sign * v, &mut out);
            }
        }
        out.sort_by_key(|r| r.t_ns);
        out
    }
}
