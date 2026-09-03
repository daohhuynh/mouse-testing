//! Capture session: owns the per-level capture backends and turns their raw
//! samples into the series the analysers consume.
//!
//! The UI thread only ever drains rings and appends to vectors. All timing
//! comes from the samples themselves, so a slow frame delays when a number is
//! displayed but cannot change what was measured.

use crate::core::clock;
use crate::core::debounce::ButtonEvent;
use crate::core::polling::{self, PollConfig, PollResult};
use crate::core::ring::Consumer;
#[cfg(target_os = "macos")]
use crate::core::sample::{Flags, Kind};
use crate::core::sample::Sample;
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
    /// Signed per-axis motion, parallel to `times_ns`. The polling analysis only
    /// needs the magnitude, but every sensor test is about direction: a CPI
    /// measurement needs the net displacement, and snapping is a statement
    /// about the axis across the stroke.
    pub dx: Vec<i32>,
    pub dy: Vec<i32>,
    /// Raw wheel counts, parallel to `times_ns`. Raw, not detents: what one
    /// detent is worth is device- and platform-dependent and is inferred by the
    /// scroll analysis rather than assumed here.
    pub wheel: Vec<i32>,
    pub hwheel: Vec<i32>,
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
        self.dx.clear();
        self.dy.clear();
        self.wheel.clear();
        self.hwheel.clear();
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

    /// Scroll reports from `from` onward, so one run analyses only its own
    /// wheel activity rather than everything since the capture started.
    pub fn scroll_from(&self, from: usize) -> Vec<crate::core::sensor::Report> {
        let from = from.min(self.times_ns.len());
        self.times_ns[from..]
            .iter()
            .zip(self.wheel[from..].iter().zip(&self.hwheel[from..]))
            .map(|(&t, (&w, &h))| crate::core::sensor::Report {
                t_ns: t,
                dx: 0,
                dy: 0,
                wheel: w,
                hwheel: h,
            })
            .collect()
    }

    /// Motion reports from `from` onward, for the sensor detectors.
    pub fn motion_from(&self, from: usize) -> Vec<crate::core::sensor::Report> {
        let from = from.min(self.times_ns.len());
        self.times_ns[from..]
            .iter()
            .zip(self.dx[from..].iter().zip(&self.dy[from..]))
            .map(|(&t, (&x, &y))| crate::core::sensor::Report::motion(t, x, y))
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

/// What the device level's state should be after a pump.
///
/// `opened == 0` means "nothing could be opened" only once `running` says the
/// capture thread has finished opening and published its result. The two are
/// written under one lock, and the thread enumerates, opens and schedules every
/// device before taking it, so for the whole of that window `opened` is 0 while
/// nothing is wrong. Treating that as a failure latched the level to Blocked in
/// the first frame after start, and because the recovery to Live fired only
/// from Waiting, the run then collected thousands of reports at a clean 1 kHz
/// underneath a red "blocked". SENSOR and SCROLL, which refuse to run when this
/// state is Blocked, disabled themselves for the same reason.
fn device_state_after_pump(
    prev: LevelState,
    // `status_running` is whether the backend has published the result of
    // opening devices, which only macOS has to wait for; Windows decides
    // registration once at start and passes true. `status_opened` is how many
    // devices it opened, and Windows passes 1 because raw input is a single
    // registration rather than a per-device open.
    status_running: bool,
    status_opened: usize,
    total_reports: u64,
    removed: u64,
) -> LevelState {
    // A device the system took away outranks everything below. The reference
    // this capture holds is dead and a device that comes back is a new one it
    // never opened, so no report will ever arrive again on this capture. It has
    // to be checked before the report count, or the level stays Live forever on
    // the strength of reports that stopped coming.
    if removed > 0 {
        return LevelState::Blocked;
    }
    // A report is proof the device is open, whatever the status said earlier.
    if total_reports > 0 {
        return LevelState::Live;
    }
    if status_running && status_opened == 0 {
        return LevelState::Blocked;
    }
    prev
}

pub struct Session {
    pub device: Series,
    pub system: Series,
    pub app: Series,
    pub started: Option<Instant>,
    pub device_state: LevelState,
    pub system_state: LevelState,
    pub device_note: String,
    /// The device this capture is actually bound to. Selecting a different one
    /// changes only which device the app intends to measure; the capture keeps
    /// streaming whatever it opened, so without this a new selection silently
    /// measured the old device.
    pub bound_key: Option<String>,
    /// The device was taken away mid-capture. Distinct from Blocked, which also
    /// covers a permission that no restart will fix. This one is recoverable:
    /// the capture is dead but a new one will find the device if it came back.
    pub device_removed: bool,
    pub system_note: String,
    /// Windows build number, supplied by the caller because the capture layer
    /// does not enumerate the host. It selects the low-level hook's timeout
    /// budget, which changed at build 16299, so a wrong value here would make
    /// the app quote the wrong deadline for its own hook.
    pub os_build: u32,
    /// Reports whose fields the descriptor parser could decode.
    pub decoded: u64,
    pub undecoded: u64,
    /// Button edges, in time order, from whichever level is supplying them.
    pub buttons: Vec<ButtonEvent>,
    /// Which level the button edges came from, so the interface can say.
    pub button_source: Option<Tier>,
    /// Last known absolute button state per device, for turning the state a HID
    /// report carries into the press and release edges the analysis needs.
    /// Last known absolute button state per device. Only the HID path needs
    /// this: a HID report carries the state of every button, so transitions
    /// have to be derived, whereas Windows raw input reports the transitions
    /// themselves.
    #[cfg(target_os = "macos")]
    button_state: std::collections::BTreeMap<u64, u32>,
    /// Of the system level events, how many were bound for another
    /// application rather than this one.
    pub system_elsewhere: u64,
    /// Events the capture saw while this application was NOT in the foreground.
    /// A nonzero value is the proof that a mouse can be measured without being
    /// the thing driving the interface. Windows only: the macOS system tier
    /// already reports the same fact as `system_elsewhere`.
    pub background_events: u64,
    /// Events the operating system marked as synthesised by software rather
    /// than produced by hardware. Any of these in a measurement window means
    /// the numbers describe a program, not the mouse.
    pub injected_events: u64,
    /// The OS's own count of system-wide motion events over this run. It needs
    /// no permission, so it is the control that distinguishes "nothing moved"
    /// from "this level is not working".
    pub control_motion: u64,
    #[cfg(target_os = "macos")]
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
            bound_key: None,
            device_removed: false,
            system_note: String::new(),
            os_build: 0,
            decoded: 0,
            undecoded: 0,
            system_elsewhere: 0,
            background_events: 0,
            injected_events: 0,
            buttons: Vec::new(),
            button_source: None,
            #[cfg(target_os = "macos")]
            button_state: std::collections::BTreeMap::new(),
            control_motion: 0,
            #[cfg(target_os = "macos")]
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
        self.bound_key = device_key.map(str::to_string);
        self.device_removed = false;
        self.device.clear();
        self.system.clear();
        self.app.clear();
        self.origin_ticks = None;
        self.decoded = 0;
        self.undecoded = 0;
        self.control_motion = 0;
        #[cfg(target_os = "macos")]
        {
            self.control_origin = None;
        }
        self.buttons.clear();
        self.button_source = None;
        #[cfg(target_os = "macos")]
        self.button_state.clear();

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
        self.bound_key = device_key.map(str::to_string);
        self.device_removed = false;
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

        let hook = HookCapture::start(1 << 16, self.os_build);
        let hs = hook.status();
        if hs.installed {
            self.system_state = LevelState::Waiting;
            // Not an error, but the user needs it: Windows silently REMOVES a
            // low-level hook whose procedure overruns this budget, and the
            // symptom is a system level that simply stops delivering rather
            // than one that reports a failure.
            self.system_note = format!(
                "Windows will remove this hook without warning if its handler ever takes \
                 longer than {} ms{}. The handler here only timestamps and enqueues, but \
                 a heavily loaded machine can still overrun it, so watch for the system \
                 level going quiet.",
                hs.timeout_ms,
                if hs.timeout_assumed {
                    ", which is the default for this Windows build because no                      LowLevelHooksTimeout value is set"
                } else {
                    ", set by LowLevelHooksTimeout"
                }
            );
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

    /// Drains everything captured since the last call. Cheap: a memcpy of
    /// plain data plus an append.
    ///
    /// Each backend's ring is drained into a locally owned buffer rather than
    /// while holding a borrow of the backend, so the per-sample work is free to
    /// touch the rest of the session.
    pub fn pump(&mut self) {
        self.pump_control();

        #[cfg(windows)]
        self.pump_windows();

        #[cfg(target_os = "macos")]
        self.pump_macos();
    }

    #[cfg(target_os = "macos")]
    fn pump_macos(&mut self) {
        if let Some(mut consumer) = self.hid_consumer.take() {
            if let Some((ring, decoded, undecoded, status, removed)) = self.hid.as_ref().map(|h| {
                (
                    h.ring.clone(),
                    h.decoded(),
                    h.undecoded(),
                    h.status(),
                    h.removed(),
                )
            }) {
                self.decoded = decoded;
                self.undecoded = undecoded;
                let refusal = || {
                    status
                        .refused
                        .first()
                        .map(|(n, why)| format!("{n}: {why}"))
                        .unwrap_or_else(|| "No matching device could be opened.".into())
                };

                let mut buf = std::mem::take(&mut self.scratch);
                buf.clear();
                self.device.ring_drops += ring.drain(&mut consumer, &mut buf);
                for s in &buf {
                    if s.kind != Kind::Report {
                        continue;
                    }
                    let t = self.to_ns(s.t);
                    self.device.times_ns.push(t);
                    self.device.counts.push(if s.has(Flags::DECODED) {
                        s.dx.unsigned_abs().saturating_add(s.dy.unsigned_abs()) as i32
                    } else {
                        // Unknown, not zero: a zero would read as "not moving"
                        // and would suppress the whole analysis.
                        -1
                    });
                    self.device.dx.push(s.dx);
                    self.device.dy.push(s.dy);
                    self.device.wheel.push(s.wheel);
                    self.device.hwheel.push(s.hwheel);
                    self.device.total += 1;
                    if s.has(Flags::DECODED) {
                        self.push_button_edges(s.device, s.buttons_state, t, Tier::Device);
                    }
                }
                self.scratch = buf;

                // Decided after draining, so a report that arrived in this same
                // pump counts as the proof it is.
                let next = device_state_after_pump(
                    self.device_state,
                    status.running,
                    status.opened,
                    self.device.total,
                    removed,
                );
                if next != self.device_state {
                    self.device_state = next;
                    match next {
                        LevelState::Blocked if removed > 0 => {
                            self.device_removed = true;
                            self.device_note = "The device was taken away while this capture \
                                 was running: unplugged, asleep, or re-enumerated by the \
                                 system. This capture cannot recover, because the connection \
                                 it holds is dead and a device that comes back is a new one. \
                                 Starting a new capture will pick it up again if it has \
                                 reconnected."
                                .into()
                        }
                        LevelState::Blocked => self.device_note = refusal(),
                        // A stale refusal would otherwise sit under a level
                        // that is plainly working.
                        LevelState::Live => self.device_note.clear(),
                        _ => {}
                    }
                }
            }
            self.hid_consumer = Some(consumer);
        }

        if let Some(mut consumer) = self.sys_consumer.take() {
            if let Some((ring, elsewhere)) = self.sys.as_ref().map(|s| {
                (
                    s.ring.clone(),
                    s.elsewhere.load(std::sync::atomic::Ordering::Relaxed),
                )
            }) {
                self.system_elsewhere = elsewhere;
                let mut buf = std::mem::take(&mut self.scratch);
                buf.clear();
                self.system.ring_drops += ring.drain(&mut consumer, &mut buf);
                for s in &buf {
                    let t = self.to_ns(s.t);
                    self.system.times_ns.push(t);
                    self.system
                        .counts
                        .push(s.dx.unsigned_abs().saturating_add(s.dy.unsigned_abs()) as i32);
                    self.system.dx.push(s.dx);
                    self.system.dy.push(s.dy);
                    self.system.wheel.push(s.wheel);
                    self.system.hwheel.push(s.hwheel);
                    self.system.total += 1;
                    if self.button_source != Some(Tier::Device) {
                        self.push_button_transitions(s.buttons_down, s.buttons_up, t, Tier::System);
                    }
                }
                self.scratch = buf;
                if self.system.total > 0 && self.system_state == LevelState::Waiting {
                    self.system_state = LevelState::Live;
                }
            }
            self.sys_consumer = Some(consumer);
        }
    }


    #[cfg(windows)]
    fn pump_windows(&mut self) {
        if let Some(raw) = self.raw.as_ref() {
            self.background_events = raw.background();
            // Raw input delivers every mouse on one stream and the device
            // filter picks one out of it. Without this, choosing the wrong
            // device looks exactly like a mouse that is not reporting: the
            // count is zero either way. `seen` is the count BEFORE filtering,
            // so the two cases can be told apart and said out loud.
            let seen = raw.seen();
            if seen > 0 && self.device.total == 0 && self.filter_device.is_some() {
                self.device_note = format!(
                    "{seen} report(s) arrived, but none from the selected device. Another \
                     pointing device is reporting instead. Pick the right one in DEVICE, \
                     or move the mouse you mean to measure."
                );
            } else if self.device.total > 0 {
                self.device_note.clear();
            }
        }
        if let Some(hook) = self.hook.as_ref() {
            self.injected_events = hook.injected();
        }
        if let Some(mut consumer) = self.raw_consumer.take() {
            if let Some(ring) = self.raw.as_ref().map(|r| r.ring.clone()) {
                let mut buf = std::mem::take(&mut self.scratch);
                buf.clear();
                self.device.ring_drops += ring.drain(&mut consumer, &mut buf);
                for s in &buf {
                    // Raw input delivers every mouse in one stream, so keep
                    // only the device under test.
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
                    self.device.dx.push(s.dx);
                    self.device.dy.push(s.dy);
                    self.device.wheel.push(s.wheel);
                    self.device.hwheel.push(s.hwheel);
                    self.device.total += 1;
                    self.push_button_transitions(s.buttons_down, s.buttons_up, t, Tier::Device);
                }
                self.scratch = buf;

                // Same rule as macOS: a device Windows has taken away outranks
                // the reports already counted, because none will follow them.
                // Registration already asked for these notifications; nothing
                // read them, so an unplugged mouse mid-run just went quiet.
                //
                // A removal counts against this level whenever the level was
                // receiving anything, since raw input has no per-device filter
                // in force (`filter_device` is never assigned), which makes the
                // device level here the sum of every mouse rather than one.
                let removed = self.raw.as_ref().map(|r| r.removed().0).unwrap_or(0);
                let next = device_state_after_pump(
                    self.device_state,
                    true,
                    1,
                    if self.device.total > 0 { 1 } else { 0 },
                    if self.device.total > 0 { removed } else { 0 },
                );
                if next != self.device_state {
                    self.device_state = next;
                    if next == LevelState::Blocked {
                        self.device_removed = true;
                        self.device_note = "A pointing device was unplugged or removed while \
                             this capture was running. Raw input cannot resume for it, and a \
                             device that comes back is a new one. Start a new run."
                            .into();
                    }
                }
            }
            self.raw_consumer = Some(consumer);
        }

        if let Some(mut consumer) = self.hook_consumer.take() {
            if let Some(ring) = self.hook.as_ref().map(|h| h.ring.clone()) {
                let mut buf = std::mem::take(&mut self.scratch);
                buf.clear();
                self.system.ring_drops += ring.drain(&mut consumer, &mut buf);
                for s in &buf {
                    let t = self.to_ns(s.t);
                    self.system.times_ns.push(t);
                    // The hook reports a cursor position, not a delta, so
                    // motion magnitude is not available at this level. The
                    // per-axis vectors are still pushed, because every consumer
                    // indexes them against `times_ns` and a short vector would
                    // silently pair a timestamp with another report's motion.
                    // Nothing reads them: the sensor tests run on the device
                    // tier, which does carry real deltas.
                    self.system.counts.push(0);
                    self.system.dx.push(0);
                    self.system.dy.push(0);
                    self.system.wheel.push(s.wheel);
                    self.system.hwheel.push(s.hwheel);
                    self.system.total += 1;
                    if self.button_source != Some(Tier::Device) {
                        self.push_button_transitions(s.buttons_down, s.buttons_up, t, Tier::System);
                    }
                }
                self.scratch = buf;
                if self.system.total > 0 && self.system_state == LevelState::Waiting {
                    self.system_state = LevelState::Live;
                }
            }
            self.hook_consumer = Some(consumer);
        }
    }


    /// Turns an absolute button state into press and release edges.
    #[cfg(target_os = "macos")]
    fn push_button_edges(&mut self, device: u64, state: u32, t_ns: u64, source: Tier) {
        let prev = *self.button_state.get(&device).unwrap_or(&0);
        if prev == state {
            return;
        }
        let changed = prev ^ state;
        for bit in 0..32u32 {
            if changed & (1 << bit) == 0 {
                continue;
            }
            self.buttons.push(ButtonEvent {
                t_ns,
                // Reported as the raw HID button usage, which is one-based, so
                // an unmapped button shows up as itself rather than as a gap.
                button: (bit + 1) as u8,
                down: state & (1 << bit) != 0,
            });
        }
        self.button_state.insert(device, state);
        // The device level takes precedence once it produces anything, so the
        // two sources can never be interleaved into one series.
        if self.button_source != Some(Tier::Device) {
            self.button_source = Some(source);
        }
    }

    /// Records edges from a source that reports transitions rather than state.
    fn push_button_transitions(&mut self, down: u32, up: u32, t_ns: u64, source: Tier) {
        if down == 0 && up == 0 {
            return;
        }
        for bit in 0..32u32 {
            if down & (1 << bit) != 0 {
                self.buttons.push(ButtonEvent { t_ns, button: (bit + 1) as u8, down: true });
            }
            if up & (1 << bit) != 0 {
                self.buttons.push(ButtonEvent { t_ns, button: (bit + 1) as u8, down: false });
            }
        }
        if self.button_source.is_none() {
            self.button_source = Some(source);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unpublished_status_is_not_a_failure_to_open() {
        // The capture thread has not reported in yet, so `opened == 0` says
        // nothing. Blocking here is what disabled a working device level, and
        // took SENSOR and SCROLL down with it.
        assert_eq!(
            device_state_after_pump(LevelState::Waiting, false, 0, 0, 0),
            LevelState::Waiting
        );
    }

    #[test]
    fn a_thread_that_reported_opening_nothing_does_block() {
        assert_eq!(
            device_state_after_pump(LevelState::Waiting, true, 0, 0, 0),
            LevelState::Blocked
        );
    }

    #[test]
    fn a_report_clears_a_block_rather_than_being_counted_underneath_it() {
        // The original recovery fired only from Waiting, so any Blocked reached
        // during startup survived the whole run no matter what arrived.
        assert_eq!(
            device_state_after_pump(LevelState::Blocked, true, 0, 1, 0),
            LevelState::Live
        );
        assert_eq!(
            device_state_after_pump(LevelState::Waiting, true, 1, 19_156, 0),
            LevelState::Live
        );
    }

    #[test]
    fn a_device_taken_away_beats_the_reports_it_already_delivered() {
        // The failure this exists for: a capture that has been running happily
        // loses its device, and every later pump still sees a report count in
        // the thousands. Ranking that count first left the level Live and the
        // whole app blind, so a run that measured nothing reported it as a
        // mouse that did not move.
        assert_eq!(
            device_state_after_pump(LevelState::Live, true, 1, 6_300, 1),
            LevelState::Blocked
        );
        // And it must not un-block itself on any later pump either.
        assert_eq!(
            device_state_after_pump(LevelState::Blocked, true, 1, 6_300, 1),
            LevelState::Blocked
        );
    }

    #[test]
    fn the_startup_window_resolves_to_live_once_reports_arrive() {
        // The whole sequence a real run goes through, in order.
        let mut st = LevelState::Waiting;
        for _ in 0..5 {
            st = device_state_after_pump(st, false, 0, 0, 0); // thread still opening
        }
        assert_eq!(st, LevelState::Waiting);
        st = device_state_after_pump(st, true, 1, 0, 0); // published, opened one
        assert_eq!(st, LevelState::Waiting);
        st = device_state_after_pump(st, true, 1, 42, 0); // reports arriving
        assert_eq!(st, LevelState::Live);
    }
}
