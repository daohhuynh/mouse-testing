#![allow(dead_code)]

//! Synthetic report streams with known ground truth.
//!
//! Detectors are validated against these rather than against intuition. The
//! generator models the two behaviours that make real streams hard: a mouse
//! emits nothing while it is not moving, and a sensor that accumulates
//! sub-count motion emits nothing for a slot whose motion rounded to zero.
//! Both look exactly like a dropped report unless the detector guards for them.

use super::polling::Report;

/// xorshift64*, so the tests are deterministic and need no dependency.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed | 1)
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    pub fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Standard normal, Box-Muller.
    pub fn normal(&mut self) -> f64 {
        let u1 = self.unit().max(1e-12);
        let u2 = self.unit();
        (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()
    }
}

#[derive(Clone, Debug)]
pub struct StreamSpec {
    pub rate_hz: f64,
    /// Spans during which the hand is moving, in seconds.
    pub motion: Vec<(f64, f64)>,
    pub total_s: f64,
    /// Probability that a report which would have been sent is lost.
    pub drop_prob: f64,
    /// Standard deviation of timestamp noise, nanoseconds. Device timestamps
    /// are a few microseconds; host timestamps are tens to hundreds.
    pub jitter_ns: f64,
    /// Mean motion counts produced per polling slot while moving.
    pub counts_per_slot: f64,
}

impl Default for StreamSpec {
    fn default() -> Self {
        StreamSpec {
            rate_hz: 1000.0,
            motion: vec![(0.0, 2.0)],
            total_s: 2.0,
            drop_prob: 0.0,
            jitter_ns: 2_000.0,
            counts_per_slot: 5.0,
        }
    }
}

pub struct Truth {
    pub reports: Vec<Report>,
    /// Slots that would have produced a report but were dropped.
    pub dropped: usize,
    /// Slots that produced a report.
    pub emitted: usize,
}

impl Truth {
    pub fn drop_rate(&self) -> f64 {
        let total = self.dropped + self.emitted;
        if total == 0 {
            0.0
        } else {
            self.dropped as f64 / total as f64
        }
    }
}

pub fn generate(spec: &StreamSpec, rng: &mut Rng) -> Truth {
    let slot_ns = 1e9 / spec.rate_hz;
    let n_slots = (spec.total_s * spec.rate_hz) as usize;
    let mut reports = Vec::with_capacity(n_slots);
    let mut dropped = 0usize;
    let mut emitted = 0usize;
    // Sub-count motion is carried forward, as a real sensor does.
    let mut residual = 0.0f64;

    for i in 0..n_slots {
        let t_s = i as f64 / spec.rate_hz;
        let moving = spec.motion.iter().any(|&(a, b)| t_s >= a && t_s < b);
        if !moving {
            residual = 0.0;
            continue;
        }
        residual += spec.counts_per_slot * (1.0 + 0.25 * rng.normal());
        let counts = residual.floor().max(0.0);
        residual -= counts;
        if counts <= 0.0 {
            // Nothing to report. Indistinguishable from a lost report unless
            // the detector requires motion on both sides of the interval.
            continue;
        }
        if rng.unit() < spec.drop_prob {
            dropped += 1;
            continue;
        }
        emitted += 1;
        let t = (i as f64 * slot_ns + spec.jitter_ns * rng.normal()).max(0.0);
        reports.push(Report {
            t_ns: t as u64,
            counts: counts as i32,
        });
    }

    // Jitter can reorder adjacent reports; a real capture sorts by timestamp.
    reports.sort_by_key(|r| r.t_ns);
    Truth {
        reports,
        dropped,
        emitted,
    }
}
