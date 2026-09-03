//! Core input types. NOTHING in the detector crate depends on any external crate;
//! `rand`/`rand_distr` are used ONLY by the simulator/validation binary.

/// One HID mouse motion report, as captured by the platform backend.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Report {
    /// Monotonic timestamp, nanoseconds. macOS: mach_absolute_time converted via
    /// mach_timebase_info. Windows: QueryPerformanceCounter converted to ns.
    pub t_ns: u64,
    pub dx: i32,
    pub dy: i32,
    /// Vertical wheel, raw device units (may be 1, 120, or high-res sub-units).
    pub wheel: i32,
    /// Horizontal wheel / tilt, raw device units.
    pub hwheel: i32,
}

impl Report {
    pub fn motion(t_ns: u64, dx: i32, dy: i32) -> Self {
        Report { t_ns, dx, dy, wheel: 0, hwheel: 0 }
    }
    pub fn wheel_ev(t_ns: u64, w: i32) -> Self {
        Report { t_ns, dx: 0, dy: 0, wheel: w, hwheel: 0 }
    }
    #[inline]
    pub fn is_moving(&self) -> bool { self.dx != 0 || self.dy != 0 }
    #[inline]
    pub fn mag(&self) -> f64 { ((self.dx as f64).powi(2) + (self.dy as f64).powi(2)).sqrt() }
}

/// The whole app renders one verdict widget, so the detectors use the same
/// verdict the polling section does rather than a parallel one.
pub use crate::core::polling::Verdict;

/// Nanoseconds per second, as f64, to avoid magic numbers everywhere.
pub const NS: f64 = 1.0e9;

/// True when timestamps never go backwards.
///
/// Every detector here differences timestamps, and `t_ns` is unsigned. One
/// out-of-order report in six hundred was enough to panic a debug build with
/// "attempt to subtract with overflow" and, in release, to wrap to 1.8e19 ns
/// and turn a clean stream into a Warn. Two ordinary things produce that:
/// merging streams from two devices, and host-side completion timestamps.
/// The arithmetic is saturating now, but saturating to a zero interval is
/// still a wrong answer, so each entry point refuses instead.
pub fn is_monotonic(r: &[Report]) -> bool {
    r.windows(2).all(|w| w[1].t_ns >= w[0].t_ns)
}

/// The note every detector shows when it refuses a non-monotonic capture.
pub const NOT_MONOTONIC: &str =
    "timestamps go backwards in this capture, so no interval here can be trusted. \
     Capture one device at a time.";
