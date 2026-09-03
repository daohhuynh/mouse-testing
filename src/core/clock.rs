//! One clock, in the platform's native ticks.
//!
//! Ticks are converted to nanoseconds once, at the point of display. The
//! reason is macOS: HID report timestamps arrive in mach absolute time, and on
//! Apple Silicon one tick is 41.667 ns rather than 1 ns, so mixing a converted
//! and an unconverted value in one subtraction is wrong by a factor of 41.

#[cfg(target_os = "macos")]
mod imp {
    use crate::platform::macos::ffi::{mach_absolute_time, mach_timebase_info, MachTimebase};
    use std::sync::OnceLock;

    fn timebase() -> &'static MachTimebase {
        static TB: OnceLock<MachTimebase> = OnceLock::new();
        TB.get_or_init(|| {
            let mut tb = MachTimebase::default();
            unsafe { mach_timebase_info(&mut tb) };
            if tb.denom == 0 {
                MachTimebase { numer: 1, denom: 1 }
            } else {
                tb
            }
        })
    }

    /// Used by capture paths that must stamp an event themselves, and by the
    /// Windows backend where the OS attaches no timestamp at all.
    #[allow(dead_code)]
    #[inline(always)]
    pub fn now() -> u64 {
        unsafe { mach_absolute_time() }
    }

    pub fn ticks_to_ns(t: u64) -> u64 {
        let tb = timebase();
        // u128 so a machine up for weeks cannot overflow the multiply.
        (t as u128 * tb.numer as u128 / tb.denom as u128) as u64
    }

    // Used by the macOS system tier, which gets a seconds-based timestamp from
    // NSEvent and has to put it back on the same tick scale as the HID path,
    // and by the self test's report. Neither exists on Windows.
    #[allow(dead_code)]
    pub fn ns_to_ticks(ns: u64) -> u64 {
        let tb = timebase();
        (ns as u128 * tb.denom as u128 / tb.numer as u128) as u64
    }

    #[allow(dead_code)]
    pub fn name() -> &'static str {
        "mach_absolute_time"
    }
}

#[cfg(windows)]
mod imp {
    use std::sync::OnceLock;
    use windows_sys::Win32::System::Performance::{
        QueryPerformanceCounter, QueryPerformanceFrequency,
    };

    fn freq() -> u64 {
        static F: OnceLock<u64> = OnceLock::new();
        *F.get_or_init(|| {
            let mut f: i64 = 0;
            unsafe { QueryPerformanceFrequency(&mut f) };
            if f <= 0 {
                10_000_000
            } else {
                f as u64
            }
        })
    }

    #[inline(always)]
    pub fn now() -> u64 {
        let mut t: i64 = 0;
        unsafe { QueryPerformanceCounter(&mut t) };
        t as u64
    }

    pub fn ticks_to_ns(t: u64) -> u64 {
        let f = freq();
        // Split so a long session cannot overflow, per the QPC guidance.
        (t / f) * 1_000_000_000 + ((t % f) * 1_000_000_000) / f
    }

    // Used by the macOS system tier, which gets a seconds-based timestamp from
    // NSEvent and has to put it back on the same tick scale as the HID path,
    // and by the self test's report. Neither exists on Windows.
    #[allow(dead_code)]
    pub fn ns_to_ticks(ns: u64) -> u64 {
        let f = freq();
        (ns / 1_000_000_000) * f + ((ns % 1_000_000_000) * f) / 1_000_000_000
    }

    #[allow(dead_code)]
    pub fn name() -> &'static str {
        "QueryPerformanceCounter"
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
mod imp {
    use std::time::Instant;
    use std::sync::OnceLock;

    fn origin() -> &'static Instant {
        static O: OnceLock<Instant> = OnceLock::new();
        O.get_or_init(Instant::now)
    }

    #[inline(always)]
    pub fn now() -> u64 {
        origin().elapsed().as_nanos() as u64
    }

    pub fn ticks_to_ns(t: u64) -> u64 {
        t
    }

    // Used by the macOS system tier, which gets a seconds-based timestamp from
    // NSEvent and has to put it back on the same tick scale as the HID path,
    // and by the self test's report. Neither exists on Windows.
    #[allow(dead_code)]
    pub fn ns_to_ticks(ns: u64) -> u64 {
        ns
    }

    #[allow(dead_code)]
    pub fn name() -> &'static str {
        "std::time::Instant"
    }
}

#[allow(unused_imports)]
pub use imp::{name, now, ns_to_ticks, ticks_to_ns};

