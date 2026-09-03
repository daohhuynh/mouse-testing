//! Capture session: owns the per-level capture backends and turns their raw
//! samples into the series the analysers consume.
//!
//! The UI thread only ever drains rings and appends to vectors. All timing
//! comes from the samples themselves, so a slow frame delays when a number is
//! displayed but cannot change what was measured.

use crate::core::clock;
use crate::core::polling::{self, PollConfig, PollResult};
use crate::core::ring::Consumer;
use crate::core::sample::{Flags, Kind, Sample};
use crate::platform::Tier;
use std::time::Instant;

#[cfg(target_os = "macos")]
use crate::platform::macos::capture::{HidCapture, Target};
#[cfg(target_os = "macos")]
use crate::platform::macos::system_tier::SystemCapture;
#[cfg(windows)]
use crate::platform::windows::capture::RawInputCapture;
#[cfg(windows)]
use crate::platform::windows::hook::HookCapture;

/// Per-level accumulated series.
#[derive(Default)]
pub struct Series {
    /// Report timestamps in nanoseconds.
    pub times_ns: Vec<u64>,
    /// Motion magnitude in device counts, parallel to `times_ns`.
    pub counts: Vec<i32>,
    pub total: u64,
    /// Samples the ring had to discard because the consumer fell behind.
    pub ring_drops: u64,
    /// True when the level supplies per-event timestamps at all.
    #[allow(dead_code)]
    pub has_timestamps: bool,
    /// True when motion counts are real device counts rather than a proxy.
    #[allow(dead_code)]
    pub has_counts: bool,
}

impl Series {
    pub fn clear(&mut self) {
        self.times_ns.clear();
        self.counts.clear();
        self.total = 0;
        self.ring_drops = 0;
    }

    pub fn reports(&self) -> Vec<polling::Report> {
        self.times_ns
            .iter()
            .zip(&self.counts)
            .map(|(&t, &c)| polling::Report { t_ns: t, counts: c })
            .collect()
    }

    pub fn live_hz(&self, window_ns: u64) -> f64 {
        polling::windowed_rate(&self.times_ns, window_ns)
    }

    pub fn sustained_hz(&self) -> f64 {
        if self.times_ns.len() < 2 {
            return 0.0;
        }
        let span = self.times_ns[self.times_ns.len() - 1].saturating_sub(self.times_ns[0]);
        if span == 0 {
            0.0
        } else {
            (self.times_ns.len() - 1) as f64 * 1e9 / span as f64
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LevelState {
    /// Not started.
    Idle,
    /// Running and receiving.
    Live,
    /// Running but nothing has arrived yet.
    Waiting,
    /// Cannot run, with a reason.
    Blocked,
}

pub struct Session {
    pub device: Series,
    pub system: Series,
    pub app: Series,
    pub started: Option<Instant>,
    pub device_state: LevelState,
    pub system_state: LevelState,
    pub device_note: String,
    pub system_note: String,
    /// Reports whose fields the descriptor parser could decode.
    pub decoded: u64,
    pub undecoded: u64,
    /// The OS's own count of system-wide motion events over this run. It needs
    /// no permission, so it is the control that distinguishes "nothing moved"
    /// from "this level is not working".
    pub control_motion: u64,
    control_origin: Option<u64>,

    #[cfg(target_os = "macos")]
    hid: Option<HidCapture>,
    #[cfg(target_os = "macos")]
    hid_consumer: Option<Consumer>,
    #[cfg(target_os = "macos")]
    sys: Option<SystemCapture>,
    #[cfg(target_os = "macos")]
    sys_consumer: Option<Consumer>,

    #[cfg(windows)]
    raw: Option<RawInputCapture>,
    #[cfg(windows)]
    raw_consumer: Option<Consumer>,
    #[cfg(windows)]
    hook: Option<HookCapture>,
    #[cfg(windows)]
    hook_consumer: Option<Consumer>,

    scratch: Vec<Sample>,
    /// Origin so displayed times start at zero.
    origin_ticks: Option<u64>,
    /// Enumeration key of the device under test, where the platform needs it
    /// to filter a shared stream.
    #[allow(dead_code)]
    filter_key: Option<String>,
    /// Platform device handle of the device under test, resolved from the key.
    #[allow(dead_code)]
    filter_device: Option<u64>,
}

impl Default for Session {
    fn default() -> Self {
        Session {
            device: Series {
                has_timestamps: true,
                has_counts: true,
                ..Default::default()
            },
            system: Series {
                has_timestamps: true,
                has_counts: false,
                ..Default::default()
            },
            app: Series {
                // egui exposes no per-event timestamp, only a per-frame one, so
                // this level can report a rate but not an interval distribution.
                has_timestamps: false,
                has_counts: false,
                ..Default::default()
            },
            started: None,
            device_state: LevelState::Idle,
            system_state: LevelState::Idle,
            device_note: String::new(),
            system_note: String::new(),
            decoded: 0,
            undecoded: 0,
            control_motion: 0,
            control_origin: None,
            #[cfg(target_os = "macos")]
            hid: None,
            #[cfg(target_os = "macos")]
            hid_consumer: None,
            #[cfg(target_os = "macos")]
            sys: None,
            #[cfg(target_os = "macos")]
            sys_consumer: None,
            #[cfg(windows)]
            raw: None,
            #[cfg(windows)]
            raw_consumer: None,
            #[cfg(windows)]
            hook: None,
            #[cfg(windows)]
            hook_consumer: None,
            scratch: Vec::with_capacity(4096),
            origin_ticks: None,
            filter_key: None,
            filter_device: None,
        }
    }
}

impl Session {
    pub fn running(&self) -> bool {
        self.started.is_some()
    }

    pub fn elapsed_s(&self) -> f64 {
        self.started.map(|s| s.elapsed().as_secs_f64()).unwrap_or(0.0)
    }

    #[cfg(target_os = "macos")]
    pub fn start(&mut self, device_key: Option<&str>) {
        self.stop();
        self.device.clear();
        self.system.clear();
        self.app.clear();
        self.origin_ticks = None;
        self.decoded = 0;
        self.undecoded = 0;
        self.control_motion = 0;
        self.control_origin = None;

        match device_key {
            Some(k) => {
                let cap = HidCapture::start(Target::Key(k.to_string()), 1 << 17);
                self.hid_consumer = cap.take_consumer();
                self.hid = Some(cap);
                self.device_state = LevelState::Waiting;
            }
            None => {
                self.device_state = LevelState::Blocked;
                self.device_note = "No device selected.".into();
            }
        }

        let sys = SystemCapture::start(1 << 16);
        if sys.installed() {
            self.system_state = LevelState::Waiting;
            self.system_note.clear();
        } else {
            self.system_state = LevelState::Blocked;
            self.system_note = "macOS refused to install a global event monitor.".into();
        }
        self.sys_consumer = sys.ring.take_consumer();
        self.sys = Some(sys);

        self.started = Some(Instant::now());
    }

    #[cfg(windows)]
    pub fn start(&mut self, device_key: Option<&str>) {
        self.stop();
        self.device.clear();
        self.system.clear();
        self.app.clear();
        self.origin_ticks = None;
        // Raw input is registered per process rather than per device, so the
        // stream carries every mouse and is filtered by device handle here.
        self.filter_key = device_key.map(str::to_string);

        let raw = RawInputCapture::start(1 << 17);
        let st = raw.status();
        if st.registered {
            self.device_state = LevelState::Waiting;
            self.device_note.clear();
        } else {
            self.device_state = LevelState::Blocked;
            self.device_note = st
                .error
                .unwrap_or_else(|| "Raw input registration failed.".into());
        }
        self.raw_consumer = raw.take_consumer();
        self.raw = Some(raw);

        let build = 0;
        let hook = HookCapture::start(1 << 16, build);
        let hs = hook.status();
        if hs.installed {
            self.system_state = LevelState::Waiting;
            self.system_note.clear();
        } else {
            self.system_state = LevelState::Blocked;
            self.system_note = hs
                .error
                .unwrap_or_else(|| "Low-level mouse hook could not be installed.".into());
        }
        self.hook_consumer = hook.take_consumer();
        self.hook = Some(hook);

        self.started = Some(Instant::now());
    }

    #[cfg(not(any(target_os = "macos", windows)))]
    pub fn start(&mut self, _device_key: Option<&str>) {
        self.device_state = LevelState::Blocked;
        self.device_note = "No capture backend on this platform.".into();
        self.system_state = LevelState::Blocked;
        self.started = Some(Instant::now());
    }

    pub fn stop(&mut self) {
        #[cfg(target_os = "macos")]
        {
            if let Some(mut h) = self.hid.take() {
                h.stop();
            }
            self.hid_consumer = None;
            self.sys = None;
            self.sys_consumer = None;
        }
        #[cfg(windows)]
        {
            if let Some(mut r) = self.raw.take() {
                r.stop();
            }
            self.raw_consumer = None;
            if let Some(mut h) = self.hook.take() {
                h.stop();
            }
            self.hook_consumer = None;
        }
        self.started = None;
        if self.device_state != LevelState::Blocked {
            self.device_state = LevelState::Idle;
        }
        if self.system_state != LevelState::Blocked {
            self.system_state = LevelState::Idle;
        }
    }

    fn to_ns(&mut self, ticks: u64) -> u64 {
        let origin = *self.origin_ticks.get_or_insert(ticks);
        clock::ticks_to_ns(ticks.saturating_sub(origin))
    }

    /// Drains everything captured since the last call. Cheap: a memcpy of plain
    /// data plus an append.
    pub fn pump(&mut self) {
        self.pump_control();

        #[cfg(windows)]
        self.pump_windows();

        #[cfg(target_os = "macos")]
        {
            if let (Some(hid), Some(c)) = (self.hid.as_ref(), self.hid_consumer.as_mut()) {
                self.scratch.clear();
                let drops = hid.ring.drain(c, &mut self.scratch);
                self.device.ring_drops += drops;
                self.decoded = hid.decoded();
                self.undecoded = hid.undecoded();
                let st = hid.status();
                if st.opened == 0 {
                    self.device_state = LevelState::Blocked;
                    self.device_note = st
                        .refused
                        .first()
                        .map(|(n, why)| format!("{n}: {why}"))
                        .unwrap_or_else(|| "No matching device could be opened.".into());
                }
                let taken = std::mem::take(&mut self.scratch);
                for s in &taken {
                    if s.kind != Kind::Report {
                        continue;
                    }
                    let t = self.to_ns(s.t);
                    self.device.times_ns.push(t);
                    self.device.counts.push(
                        if s.has(Flags::DECODED) {
                            s.dx.unsigned_abs().saturating_add(s.dy.unsigned_abs()) as i32
                        } else {
                            // Unknown rather than zero: a zero would be read as
                            // "not moving" and would suppress the whole analysis.
                            -1
                        },
                    );
                    self.device.total += 1;
                }
                self.scratch = taken;
                if self.device.total > 0 && self.device_state == LevelState::Waiting {
                    self.device_state = LevelState::Live;
                }
            }

            if let (Some(sys), Some(c)) = (self.sys.as_ref(), self.sys_consumer.as_mut()) {
                self.scratch.clear();
                let drops = sys.ring.drain(c, &mut self.scratch);
                self.system.ring_drops += drops;
                let taken = std::mem::take(&mut self.scratch);
                for s in &taken {
                    let t = self.to_ns(s.t);
                    self.system.times_ns.push(t);
                    self.system
                        .counts
                        .push(s.dx.unsigned_abs().saturating_add(s.dy.unsigned_abs()) as i32);
                    self.system.total += 1;
                }
                self.scratch = taken;
                if self.system.total > 0 && self.system_state == LevelState::Waiting {
                    self.system_state = LevelState::Live;
                }
            }
        }
    }

    #[cfg(windows)]
    fn pump_windows(&mut self) {
        if let (Some(raw), Some(c)) = (self.raw.as_ref(), self.raw_consumer.as_mut()) {
            self.scratch.clear();
            self.device.ring_drops += raw.ring.drain(c, &mut self.scratch);
            let taken = std::mem::take(&mut self.scratch);
            for s in &taken {
                // Raw input delivers every mouse, so keep only the selected one.
                if let Some(want) = self.filter_device {
                    if s.device != want {
                        continue;
                    }
                }
                let t = self.to_ns(s.t);
                self.device.times_ns.push(t);
                self.device
                    .counts
                    .push(s.dx.unsigned_abs().saturating_add(s.dy.unsigned_abs()) as i32);
                self.device.total += 1;
            }
            self.scratch = taken;
            if self.device.total > 0 && self.device_state == LevelState::Waiting {
                self.device_state = LevelState::Live;
            }
        }

        if let (Some(hook), Some(c)) = (self.hook.as_ref(), self.hook_consumer.as_mut()) {
            self.scratch.clear();
            self.system.ring_drops += hook.ring.drain(c, &mut self.scratch);
            let taken = std::mem::take(&mut self.scratch);
            for s in &taken {
                let t = self.to_ns(s.t);
                self.system.times_ns.push(t);
                // The hook reports a cursor position, not a delta, so motion
                // magnitude is not available at this level.
                self.system.counts.push(0);
                self.system.total += 1;
            }
            self.scratch = taken;
            if self.system.total > 0 && self.system_state == LevelState::Waiting {
                self.system_state = LevelState::Live;
            }
        }
    }

    /// Samples the OS's own motion counter, which needs no permission.
    fn pump_control(&mut self) {
        #[cfg(target_os = "macos")]
        {
            if !self.running() {
                return;
            }
            let now = crate::platform::macos::access::motion_event_count();
            match self.control_origin {
                None => self.control_origin = Some(now),
                Some(o) => self.control_motion = now.wrapping_sub(o),
            }
        }
    }

    /// Counts events this application actually received in the current frame.
    pub fn pump_app(&mut self, ctx: &egui::Context) {
        if !self.running() {
            return;
        }
        let moves = ctx.input(|i| {
            i.raw
                .events
                .iter()
                .filter(|e| matches!(e, egui::Event::PointerMoved(_) | egui::Event::MouseWheel { .. }))
                .count()
        });
        self.app.total += moves as u64;
    }

    pub fn app_hz(&self) -> f64 {
        let e = self.elapsed_s();
        if e <= 0.0 {
            0.0
        } else {
            self.app.total as f64 / e
        }
    }

    pub fn analyze_device(&self, cfg: &PollConfig) -> PollResult {
        let reports = self.device.reports();
        // A report whose fields could not be decoded has unknown motion, and
        // the drop detector depends on knowing motion. Say so instead of
        // guessing.
        if reports.iter().any(|r| r.counts < 0) {
            let mut r = PollResult::default();
            r.note = "Reports are arriving, but their fields could not be decoded from the \
                      device's report descriptor, so motion is unknown and dropped reports \
                      cannot be told apart from a device that simply had nothing to send. \
                      The rate below is still real.";
            return r;
        }
        polling::analyze(&reports, cfg)
    }

    pub fn tier_series(&self, tier: Tier) -> &Series {
        match tier {
            Tier::Device => &self.device,
            Tier::System => &self.system,
            Tier::App => &self.app,
        }
    }
}
