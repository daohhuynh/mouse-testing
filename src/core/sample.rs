//! The unit of capture.
//!
//! One plain-old-data record per event, small and `Copy`, so a capture callback
//! can write it into a ring with no allocation and no branching on type.


/// What a record represents.
///
/// The device tier on macOS delivers two interleaved streams for the same
/// physical report: a report callback that fires exactly once per report and
/// carries the driver's timestamp, and per-element value callbacks that carry
/// the decoded fields. They are joined on the timestamp rather than merged at
/// capture time, so counting and timing stay anchored to the stream that is
/// guaranteed one-to-one with the wire.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Kind {
    /// A whole input report. Authoritative for report rate.
    Report = 0,
    /// One decoded HID element belonging to the report with the same timestamp.
    Value = 1,
    /// A decoded event that already carries its own fields (Windows raw input,
    /// the system tier on both platforms).
    Event = 2,
}

impl Default for Kind {
    fn default() -> Self {
        Kind::Report
    }
}

bitflags_lite! {
    /// Per-sample facts that change how a sample may be used.
    pub struct Flags: u32 {
        /// The OS says this event was injected by software, not hardware.
        const INJECTED = 1 << 0;
        /// Delivered while this application was not in the foreground.
        const BACKGROUND = 1 << 1;
        /// Timestamp came from the driver rather than from this process.
        const KERNEL_TIME = 1 << 2;
        /// Came from a touch surface rather than a mouse.
        const TOUCH = 1 << 3;
        /// Scroll is continuous (pixel-based), so it has no detent quantum.
        const CONTINUOUS_SCROLL = 1 << 4;
        /// Motion and button fields were decoded from the report descriptor.
        const DECODED = 1 << 5;
    }
}

#[derive(Clone, Copy, Debug, Default)]
#[repr(C)]
pub struct Sample {
    /// Platform clock ticks. Never nanoseconds: HID timestamps arrive in mach
    /// ticks on macOS and converting early loses the ability to compare them
    /// with anything else on the same clock.
    pub t: u64,
    /// Per-tier device identity. 0 means the tier cannot attribute a device,
    /// which is the case for the system tier on both platforms.
    pub device: u64,
    pub kind: Kind,
    pub page: u16,
    pub usage: u16,
    /// The element value for `Value`; unused for `Report`.
    pub value: i32,
    /// Decoded fields, for `Event`.
    pub dx: i32,
    pub dy: i32,
    pub wheel: i32,
    pub hwheel: i32,
    /// Absolute button state, bit n = button n+1 is down. Filled when the
    /// source reports state (HID reports do); transitions are derived from it.
    pub buttons_state: u32,
    /// Bit set = that button changed to pressed in this sample. Filled when the
    /// source reports transitions directly (Windows raw input does).
    pub buttons_down: u32,
    /// Bit set = that button changed to released in this sample.
    pub buttons_up: u32,
    pub flags: u32,
}

impl Sample {
    pub fn report(t: u64, device: u64, kernel_time: bool) -> Self {
        Sample {
            t,
            device,
            kind: Kind::Report,
            flags: if kernel_time { Flags::KERNEL_TIME.bits() } else { 0 },
            ..Default::default()
        }
    }

    pub fn value(t: u64, device: u64, page: u16, usage: u16, value: i32) -> Self {
        Sample {
            t,
            device,
            kind: Kind::Value,
            page,
            usage,
            value,
            flags: Flags::KERNEL_TIME.bits(),
            ..Default::default()
        }
    }

    pub fn has(&self, f: Flags) -> bool {
        self.flags & f.bits() != 0
    }
}

