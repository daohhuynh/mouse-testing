use crate::capture::Session;
use crate::core::ab;
use crate::core::cps;
use crate::core::sensor;
use crate::core::polling::{verdict_settled, PollConfig, PollResult};
use crate::platform::{self, AccessReport, DeviceInfo, HostEnv};
use crate::ui::{sections, theme, widgets};
use std::time::Instant;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Device,
    Polling,
    Clicks,
    Cps,
    Ab,
    Sensor,
    Scroll,
    Session,
}

impl Section {
    pub const ALL: [Section; 8] = [
        Section::Device,
        Section::Polling,
        Section::Clicks,
        Section::Cps,
        Section::Ab,
        Section::Sensor,
        Section::Scroll,
        Section::Session,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Section::Device => "DEVICE",
            Section::Polling => "POLLING",
            Section::Clicks => "CLICKS",
            Section::Cps => "CPS",
            Section::Ab => "A/B",
            Section::Sensor => "SENSOR",
            Section::Scroll => "SCROLL",
            Section::Session => "SESSION",
        }
    }
}

/// Everything the platform told us, refreshed together so the device list and
/// the environment warnings can never disagree about which device is selected.
pub struct Survey {
    pub devices: Vec<DeviceInfo>,
    pub access: AccessReport,
    pub env: HostEnv,
}

impl Survey {
    pub fn run(selected: Option<&str>) -> Self {
        let devices = platform::backend::enumerate();
        let access = platform::backend::access_report();
        let env = platform::backend::host_env(&devices, selected);
        Survey {
            devices,
            access,
            env,
        }
    }
}

pub struct App {
    pub section: Section,
    pub screenshot: Option<crate::screenshot::Job>,
    pub survey: Survey,
    pub selected: Option<String>,
    pub session: Session,
    pub poll_result: PollResult,
    pub claimed_hz: String,
    /// Stop a polling run by itself once more data cannot change the verdict.
    pub poll_auto_stop: bool,
    /// True only for a run the POLLING section started. The same capture backs
    /// CLICKS, CPS and A/B, and ending one of those early would throw away the
    /// run the user is actually in the middle of.
    poll_armed: bool,
    /// Set when a run ended on its own, so the section can say so rather than
    /// leaving the user wondering who pressed stop.
    pub poll_auto_stopped: bool,
    /// Set while a delayed start is pending, so the user can put this machine's
    /// own pointer down and pick up the mouse under test.
    countdown: Option<(Instant, f64)>,
    last_analysis: Option<Instant>,
    /// Unattended verification: start a capture, run for a fixed time, write a
    /// report and exit. Used to check the capture path against real hardware
    /// without needing anyone to drive the interface.
    pub auto_capture: Option<AutoCapture>,
    pub cps: CpsState,
    pub ab: AbState,
    pub sensor: SensorState,
    pub scroll: ScrollState,
    pub data: SessionDataState,
}

/// Export and reload of session data.
#[derive(Default)]
pub struct SessionDataState {
    /// Path typed by the user, or picked from the list of previous exports.
    pub load_path: String,
    /// A previously exported log, held alongside the live session for
    /// comparison rather than replacing it.
    pub loaded: Option<crate::core::session_log::SessionLog>,
    pub loaded_from: String,
    pub loaded_skipped: usize,
    /// Which level the side-by-side comparison is showing. Set on load to
    /// whichever level the loaded file actually has data at, because defaulting
    /// to the device level shows a column of zeros for a capture taken when
    /// only the system level was permitted, and zeros there mean "wrong level
    /// selected" rather than "nothing happened".
    pub compare_level: crate::core::session_log::Level,
    /// What the last export did, shown verbatim including any error.
    pub export_message: String,
    pub export_bad: bool,
    /// What the last load did. Separate from the export message because the two
    /// appear under different headings, and showing a load result under
    /// "export" reads as though the export produced it.
    pub load_message: String,
    pub load_bad: bool,
    pub last_raw: Option<String>,
    pub last_summary: Option<String>,
}

/// Scroll wheel capture. One capture feeds both encoders, because a person
/// scrolling and tilting in one session should not have to do it twice.
pub struct ScrollState {
    pub phase: SensorPhase,
    pub countdown_s: f64,
    pub capture_s: f64,
    started: Option<Instant>,
    baseline: usize,
    pub vertical: Option<sensor::scroll::ScrollResult>,
    pub horizontal: Option<sensor::scroll::ScrollResult>,
    /// Wheel reports the last capture collected, so a run that saw nothing can
    /// say so rather than showing an empty result.
    pub last_reports: usize,
    /// Set when the shared capture stopped while this run was recording.
    /// The run is void, and without this it would end in a refusal that
    /// reads as though the mouse stopped moving.
    pub capture_lost: bool,
}

impl Default for ScrollState {
    fn default() -> Self {
        ScrollState {
            phase: SensorPhase::Idle,
            countdown_s: 3.0,
            capture_s: 20.0,
            started: None,
            baseline: 0,
            capture_lost: false,
            vertical: None,
            horizontal: None,
            last_reports: 0,
        }
    }
}

impl ScrollState {
    pub fn elapsed_s(&self) -> f64 {
        self.started.map(|t| t.elapsed().as_secs_f64()).unwrap_or(0.0)
    }
}

/// Phase of one sensor test.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SensorPhase {
    Idle,
    /// Counting down so the mouse under test can be picked up. Every sensor
    /// test needs both hands on the mouse being measured, which cannot also be
    /// the thing that pressed the button.
    Countdown,
    Recording,
}

pub struct SensorState {
    pub test: sensor::protocol::Test,
    pub phase: SensorPhase,
    pub countdown_s: f64,
    started: Option<Instant>,
    /// Index into the device series when the current capture began, so a run
    /// analyses only its own motion.
    baseline: usize,
    /// The CPI the mouse is configured to, as typed.
    pub claimed_cpi: String,
    /// The distance actually swiped, as typed.
    pub distance: String,
    /// True when `distance` is in millimetres rather than inches.
    pub distance_mm: bool,
    pub cpi_trials: Vec<sensor::cpi::CpiResult>,
    pub cpi_summary: Option<sensor::cpi::CpiSummary>,
    pub drift: Option<sensor::drift::DriftResult>,
    pub snap: Option<sensor::snap::SnapResult>,
    pub smooth: Option<sensor::smooth::SmoothResult>,
    pub tracking: Option<sensor::tracking::TrackResult>,
    /// Reports the last capture actually collected, so a run that measured
    /// nothing can say so instead of showing an empty result.
    pub last_reports: usize,
    /// Set when the shared capture stopped while this run was recording.
    /// The run is void, and without this it would end in a refusal that
    /// reads as though the mouse stopped moving.
    pub capture_lost: bool,
}

impl Default for SensorState {
    fn default() -> Self {
        SensorState {
            test: sensor::protocol::Test::Cpi,
            phase: SensorPhase::Idle,
            countdown_s: 3.0,
            started: None,
            baseline: 0,
            claimed_cpi: String::new(),
            distance: String::new(),
            distance_mm: false,
            capture_lost: false,
            cpi_trials: Vec::new(),
            cpi_summary: None,
            drift: None,
            snap: None,
            smooth: None,
            tracking: None,
            last_reports: 0,
        }
    }
}

impl SensorState {
    /// Seconds elapsed in the current phase.
    pub fn elapsed_s(&self) -> f64 {
        self.started.map(|t| t.elapsed().as_secs_f64()).unwrap_or(0.0)
    }

    pub fn claimed_cpi_value(&self) -> Option<f64> {
        self.claimed_cpi.trim().parse::<f64>().ok().filter(|v| *v > 0.0)
    }

    /// The swiped distance in inches, whichever unit it was typed in.
    pub fn distance_in(&self) -> Option<f64> {
        let v = self.distance.trim().parse::<f64>().ok().filter(|v| *v > 0.0)?;
        Some(if self.distance_mm { v / 25.4 } else { v })
    }
}

/// Phase of an A/B run. Nothing about a trial's result is available to the
/// interface until the whole run reaches `Done`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AbPhase {
    Setup,
    /// Waiting for the setting to be changed on the mouse.
    Prompt,
    Countdown,
    Trial,
    Done,
}

pub struct AbState {
    pub plan: ab::Plan,
    pub phase: AbPhase,
    pub current: usize,
    /// Trials completed so far. Deliberately not shown while the run is live.
    pub trials: Vec<ab::Trial>,
    pub run: Option<ab::Run>,
    pub countdown_s: f64,
    pub last_export: Option<String>,
    pub export_error: Option<String>,
    baseline: usize,
    phase_started: Option<Instant>,
}

impl Default for AbState {
    fn default() -> Self {
        AbState {
            plan: ab::Plan::default(),
            phase: AbPhase::Setup,
            current: 0,
            trials: Vec::new(),
            run: None,
            countdown_s: 3.0,
            last_export: None,
            export_error: None,
            baseline: 0,
            phase_started: None,
        }
    }
}

impl AbState {
    pub fn phase_elapsed(&self) -> f64 {
        self.phase_started
            .map(|t| t.elapsed().as_secs_f64())
            .unwrap_or(0.0)
    }

    pub fn condition_now(&self) -> ab::Condition {
        self.plan.condition_at(self.current)
    }
}

/// State of the clicks-per-second test.
pub struct CpsState {
    pub mode: usize,
    pub duration_s: f64,
    /// None counts every button.
    pub button: Option<u8>,
    pub running: bool,
    started: Option<Instant>,
    countdown: Option<(Instant, f64)>,
    /// Index into the session's button series when the run began, so a run
    /// counts only its own clicks.
    baseline: usize,
    pub history: Vec<cps::Run>,
}

impl Default for CpsState {
    fn default() -> Self {
        CpsState {
            mode: 0,
            duration_s: 10.0,
            button: None,
            running: false,
            started: None,
            countdown: None,
            baseline: 0,
            history: Vec::new(),
        }
    }
}

impl CpsState {
    pub fn elapsed_s(&self) -> f64 {
        self.started.map(|s| s.elapsed().as_secs_f64()).unwrap_or(0.0)
    }

    pub fn countdown_remaining(&self) -> Option<f64> {
        let (start, total) = self.countdown?;
        let left = total - start.elapsed().as_secs_f64();
        if left > 0.0 {
            Some(left)
        } else {
            None
        }
    }
}

pub struct AutoCapture {
    pub seconds: f64,
    pub out: String,
    pub started: Option<Instant>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::apply(&cc.egui_ctx);
        let survey = Survey::run(None);
        // Default to the first device that can actually be streamed from.
        let selected = survey
            .devices
            .iter()
            .find(|d| d.streamable)
            .or_else(|| survey.devices.first())
            .map(|d| d.key.clone());
        let survey = Survey::run(selected.as_deref());
        App {
            section: Section::Device,
            screenshot: None,
            survey,
            selected,
            session: Session::default(),
            poll_result: PollResult::default(),
            claimed_hz: String::new(),
            poll_auto_stop: true,
            poll_armed: false,
            poll_auto_stopped: false,
            countdown: None,
            last_analysis: None,
            auto_capture: None,
            cps: CpsState::default(),
            ab: AbState::default(),
            sensor: SensorState::default(),
            scroll: ScrollState::default(),
            data: SessionDataState::default(),
        }
    }

    pub fn ab_start(&mut self) {
        if !self.plan_ready() {
            return;
        }
        self.claim_capture();
        if !self.session.running() {
            self.start_capture(0.0);
        }
        self.ab.trials.clear();
        self.ab.run = None;
        self.ab.current = 0;
        self.ab.last_export = None;
        self.ab.export_error = None;
        self.ab.phase = AbPhase::Prompt;
        self.ab.phase_started = Some(Instant::now());
    }

    fn plan_ready(&self) -> bool {
        self.ab.plan.ready()
    }

    /// The setting has been changed; begin the countdown for this trial.
    pub fn ab_ready(&mut self) {
        if self.ab.phase != AbPhase::Prompt {
            return;
        }
        self.ab.phase = AbPhase::Countdown;
        self.ab.phase_started = Some(Instant::now());
    }

    pub fn ab_abandon(&mut self) {
        self.ab.phase = AbPhase::Setup;
        self.ab.trials.clear();
        self.ab.current = 0;
        self.ab.phase_started = None;
    }

    fn ab_presses(&self) -> usize {
        let want = self.ab.plan.button;
        self.session
            .buttons
            .iter()
            .skip(self.ab.baseline)
            .filter(|e| e.down && want.map(|b| b == e.button).unwrap_or(true))
            .count()
    }

    fn tick_ab(&mut self) {
        match self.ab.phase {
            AbPhase::Countdown => {
                if self.ab.phase_elapsed() >= self.ab.countdown_s {
                    self.ab.baseline = self.session.buttons.len();
                    self.ab.phase = AbPhase::Trial;
                    self.ab.phase_started = Some(Instant::now());
                }
            }
            AbPhase::Trial => {
                if self.ab.phase_elapsed() < self.ab.plan.trial_seconds {
                    return;
                }
                let presses = self.ab_presses();
                let value = match self.ab.plan.variant {
                    ab::Variant::Rate => presses as f64 / self.ab.plan.trial_seconds,
                    // The weak-input variant asks how many attempts registered
                    // at all, so the count is the measurement, not a rate.
                    ab::Variant::WeakInput => presses as f64,
                };
                let index = self.ab.current;
                self.ab.trials.push(ab::Trial {
                    index,
                    condition: self.ab.plan.condition_at(index),
                    pair: index / 2,
                    value,
                    presses,
                    duration_s: self.ab.plan.trial_seconds,
                });
                self.ab.current += 1;
                if self.ab.current >= self.ab.plan.total_trials() {
                    self.ab.run = Some(ab::Run {
                        plan: self.ab.plan.clone(),
                        trials: self.ab.trials.clone(),
                    });
                    self.ab.phase = AbPhase::Done;
                } else {
                    self.ab.phase = AbPhase::Prompt;
                }
                self.ab.phase_started = Some(Instant::now());
            }
            _ => {}
        }
    }

    /// Fills in a finished run with synthetic trials, so the results view can
    /// be inspected without sitting through a real one. Command line only.
    pub fn ab_demo(&mut self) {
        self.ab.plan.label_a = "1000 Hz, debounce 4 ms".into();
        self.ab.plan.label_b = "1000 Hz, debounce 0 ms".into();
        self.ab.plan.pairs = 8;
        self.ab.plan.trial_seconds = 10.0;
        // A modest, believable advantage for B against realistic trial noise.
        let a_vals = [11.8, 12.4, 11.5, 12.9, 12.1, 11.9, 12.6, 12.0];
        let b_vals = [12.6, 12.9, 12.2, 13.4, 12.7, 12.8, 13.1, 12.5];
        self.ab.trials.clear();
        for pair in 0..8usize {
            for slot in 0..2usize {
                let index = pair * 2 + slot;
                let condition = self.ab.plan.condition_at(index);
                let v = match condition {
                    ab::Condition::A => a_vals[pair],
                    ab::Condition::B => b_vals[pair],
                };
                self.ab.trials.push(ab::Trial {
                    index,
                    condition,
                    pair,
                    value: v,
                    presses: (v * 10.0) as usize,
                    duration_s: 10.0,
                });
            }
        }
        self.ab.run = Some(ab::Run {
            plan: self.ab.plan.clone(),
            trials: self.ab.trials.clone(),
        });
        self.ab.phase = AbPhase::Done;
        self.section = Section::Ab;
    }

    pub fn ab_export(&mut self) {
        let run = match &self.ab.run {
            Some(r) => r.clone(),
            None => return,
        };
        let path = crate::core::export::path_for("ab-comparison", &crate::core::export::stamp(), "csv");
        match crate::core::export::write(&path, &run.to_csv()) {
            Ok(()) => {
                self.ab.last_export = Some(path.display().to_string());
                self.ab.export_error = None;
            }
            Err(e) => {
                self.ab.export_error = Some(format!("{}: {e}", path.display()));
                self.ab.last_export = None;
            }
        }
    }

    /// Press timestamps belonging to the run in progress.
    pub fn cps_presses(&self) -> Vec<u64> {
        let want = self.cps.button;
        self.session
            .buttons
            .iter()
            .skip(self.cps.baseline)
            .filter(|e| e.down && want.map(|b| b == e.button).unwrap_or(true))
            .map(|e| e.t_ns)
            .collect()
    }

    pub fn cps_start(&mut self, delay_s: f64) {
        // A clicks-per-second run needs button events, so the capture has to be
        // live for it to count anything.
        self.claim_capture();
        if !self.session.running() {
            self.start_capture(0.0);
        }
        if delay_s > 0.0 {
            self.cps.countdown = Some((Instant::now(), delay_s));
        } else {
            self.begin_cps_run();
        }
    }

    fn begin_cps_run(&mut self) {
        self.cps.countdown = None;
        self.cps.baseline = self.session.buttons.len();
        self.cps.started = Some(Instant::now());
        self.cps.running = true;
    }

    pub fn cps_abort(&mut self) {
        self.cps.running = false;
        self.cps.started = None;
        self.cps.countdown = None;
    }

    fn tick_cps(&mut self) {
        if let Some((start, total)) = self.cps.countdown {
            if start.elapsed().as_secs_f64() >= total {
                self.begin_cps_run();
            }
        }
        if !self.cps.running {
            return;
        }
        if self.cps.elapsed_s() < self.cps.duration_s {
            return;
        }
        let presses = self.cps_presses();
        let duration_ns = (self.cps.duration_s * 1e9) as u64;
        let (sustained, peak) = cps::rates(&presses, duration_ns, 1_000_000_000);
        self.cps.history.push(cps::Run {
            mode: sections::cps::MODES[self.cps.mode].to_string(),
            button: self.cps.button.unwrap_or(0),
            duration_s: self.cps.duration_s,
            presses: presses.len(),
            sustained_cps: sustained,
            peak_cps: peak,
        });
        self.cps.running = false;
        self.cps.started = None;
    }

    // ------------------------------------------------------------ sensor

    /// Begins one sensor test after the countdown.
    pub fn sensor_start(&mut self) {
        self.claim_capture();
        if !self.session.running() {
            self.session.os_build = self.os_build_number();
            self.session.start(self.selected.as_deref());
        }
        self.sensor.capture_lost = false;
        self.sensor.baseline = self.session.device.times_ns.len();
        self.sensor.started = Some(Instant::now());
        self.sensor.phase = SensorPhase::Countdown;
    }

    pub fn sensor_abandon(&mut self) {
        self.sensor.phase = SensorPhase::Idle;
        self.sensor.started = None;
    }

    /// Clears the results for the current test only. The other four are
    /// separate measurements and clearing one should not discard them.
    pub fn sensor_clear(&mut self) {
        use sensor::protocol::Test;
        match self.sensor.test {
            Test::Cpi => {
                self.sensor.cpi_trials.clear();
                self.sensor.cpi_summary = None;
            }
            Test::Drift => self.sensor.drift = None,
            Test::Snap => self.sensor.snap = None,
            Test::Smooth => self.sensor.smooth = None,
            Test::Tracking => self.sensor.tracking = None,
        }
        self.sensor.last_reports = 0;
    }

    /// Motion captured since the current run began.
    fn sensor_reports(&self) -> Vec<sensor::Report> {
        let from = self.sensor.baseline.min(self.session.device.times_ns.len());
        self.session.device.motion_from(from)
    }

    fn tick_sensor(&mut self) {
        use sensor::protocol::Test;
        // A run measures the shared capture, so if that stops the run is void.
        // Ending it here rather than letting it play out means the answer is
        // "the capture stopped" instead of a refusal about missing motion.
        if self.sensor.phase != SensorPhase::Idle && !self.session.running() {
            self.sensor.phase = SensorPhase::Idle;
            self.sensor.started = None;
            self.sensor.capture_lost = true;
            return;
        }
        match self.sensor.phase {
            SensorPhase::Idle => return,
            SensorPhase::Countdown => {
                if self.sensor.elapsed_s() >= self.sensor.countdown_s {
                    // The baseline is taken again here, not at the button
                    // press, so nothing that happened during the countdown
                    // (setting the mouse down, lining it up on a mark) is
                    // counted as part of the measurement.
                    self.sensor.baseline = self.session.device.times_ns.len();
                    self.sensor.started = Some(Instant::now());
                    self.sensor.phase = SensorPhase::Recording;
                }
                return;
            }
            SensorPhase::Recording => {
                if self.sensor.elapsed_s() < self.sensor.test.capture_s() {
                    return;
                }
            }
        }

        let reports = self.sensor_reports();
        let capture_s = self.sensor.test.capture_s();
        self.sensor.last_reports = reports.len();
        let cpi = self.sensor.claimed_cpi_value().unwrap_or(1600.0);
        match self.sensor.test {
            Test::Cpi => {
                if let (Some(claimed), Some(dist)) =
                    (self.sensor.claimed_cpi_value(), self.sensor.distance_in())
                {
                    let mut cfg = sensor::cpi::CpiConfig::new(claimed, dist);
                    if self.sensor.distance_mm {
                        // A millimetre ruler read to the nearest millimetre
                        // carries a different uncertainty than an inch scale,
                        // and that uncertainty sets the width of the pass band.
                        cfg.distance_sigma_in = 1.0 / 25.4;
                    }
                    let r = sensor::cpi::analyze_cpi(&reports, &cfg);
                    self.sensor.cpi_trials.push(r);
                    self.sensor.cpi_summary =
                        Some(sensor::cpi::summarize_cpi(&self.sensor.cpi_trials, claimed));
                }
            }
            Test::Drift => {
                let cfg = sensor::drift::DriftConfig { cpi, ..Default::default() };
                self.sensor.drift = Some(sensor::drift::analyze_drift(&reports, capture_s, &cfg));
            }
            Test::Snap => {
                let cfg = sensor::snap::SnapConfig { cpi, ..Default::default() };
                self.sensor.snap = Some(sensor::snap::analyze_snap(&reports, &cfg));
            }
            Test::Smooth => {
                let cfg = sensor::smooth::SmoothConfig::default();
                self.sensor.smooth = Some(sensor::smooth::analyze_smoothing(&reports, &cfg));
            }
            Test::Tracking => {
                let cfg = sensor::tracking::TrackConfig { cpi, ..Default::default() };
                self.sensor.tracking =
                    Some(sensor::tracking::analyze_tracking(&reports, &cfg));
            }
        }
        self.sensor.phase = SensorPhase::Idle;
        self.sensor.started = None;
    }

    // ------------------------------------------------------------ session data

    /// Everything about this run that is not an event.
    pub fn log_meta(&self) -> crate::core::session_log::Meta {
        use crate::core::session_log::Meta;
        let dev = self.selected_device();
        Meta {
            device_name: dev.map(|d| d.name.clone()).unwrap_or_else(|| "none".into()),
            device_ids: dev.map(|d| d.ids()).unwrap_or_default(),
            transport: dev
                .and_then(|d| d.transport.clone())
                .unwrap_or_else(|| "unknown".into()),
            os: format!("{} ({})", self.survey.env.os, self.survey.env.os_build),
            arch: self.survey.env.arch.clone(),
            cpu: self.survey.env.cpu.clone(),
            clock: self.survey.env.timer.name.clone(),
            clock_resolution_ns: self.survey.env.timer.resolution_ns,
            clock_cost_ns: self.survey.env.timer.cost_ns,
            claimed_hz: self.claimed_hz.clone(),
            claimed_cpi: self.sensor.claimed_cpi.clone(),
            warnings: self
                .survey
                .env
                .warnings
                .iter()
                .map(|w| format!("{}: {}", w.title, w.detail))
                .collect(),
            duration_s: self.session.elapsed_s(),
        }
    }

    /// Every captured event, in one list, ordered by time.
    pub fn session_log(&self) -> crate::core::session_log::SessionLog {
        use crate::core::session_log::{Event, Level, SessionLog};
        let mut events: Vec<Event> = Vec::with_capacity(
            self.session.device.times_ns.len()
                + self.session.system.times_ns.len()
                + self.session.app.times_ns.len()
                + self.session.buttons.len(),
        );
        for (level, series) in [
            (Level::Device, &self.session.device),
            (Level::System, &self.session.system),
            (Level::App, &self.session.app),
        ] {
            for i in 0..series.times_ns.len() {
                events.push(Event {
                    t_ns: series.times_ns[i],
                    level,
                    // The app level has no per-axis motion to record; it counts
                    // frames a normal program was handed, not device counts.
                    dx: series.dx.get(i).copied().unwrap_or(0),
                    dy: series.dy.get(i).copied().unwrap_or(0),
                    wheel: series.wheel.get(i).copied().unwrap_or(0),
                    hwheel: series.hwheel.get(i).copied().unwrap_or(0),
                    button: 0,
                    down: false,
                    is_button: false,
                });
            }
        }
        let button_level = match self.session.button_source {
            Some(platform::Tier::System) => Level::System,
            _ => Level::Device,
        };
        for b in &self.session.buttons {
            events.push(Event {
                t_ns: b.t_ns,
                level: button_level,
                dx: 0,
                dy: 0,
                wheel: 0,
                hwheel: 0,
                button: b.button,
                down: b.down,
                is_button: true,
            });
        }
        // Sorted by time, because the file is meant to read as one session
        // rather than three concatenated ones. The sort is stable, so events
        // sharing a timestamp keep the order they were captured in.
        events.sort_by_key(|e| e.t_ns);
        SessionLog {
            meta: self.log_meta(),
            events,
        }
    }

    /// Writes the raw event log and the readable summary side by side.
    pub fn export_session(&mut self) {
        let log = self.session_log();
        self.export_log(&log);
    }

    /// Writes a log that has already been built. Separate from
    /// `export_session` so a caller that needs to keep the exact bytes it
    /// exported, such as the round-trip check, can do so: rebuilding the log
    /// afterwards would pick up a later duration and compare unequal for a
    /// reason that has nothing to do with the file format.
    pub fn export_log(&mut self, log: &crate::core::session_log::SessionLog) {
        use crate::core::export;
        let stamp = export::stamp();
        let n = log.events.len();
        let raw = export::path_for("session", &stamp, "csv");
        let sum = export::path_for("summary", &stamp, "txt");
        let text = crate::core::summary::render(self, &log.meta);
        match export::write(&raw, &log.to_csv()).and_then(|_| export::write(&sum, &text)) {
            Ok(()) => {
                self.data.last_raw = Some(raw.display().to_string());
                self.data.last_summary = Some(sum.display().to_string());
                self.data.export_message = format!("Wrote {n} event(s) and a summary.");
                self.data.export_bad = false;
            }
            Err(e) => {
                self.data.export_message = format!("Could not write the export: {e}");
                self.data.export_bad = true;
            }
        }
    }

    /// Reads a previous export back, keeping the live session intact.
    pub fn load_session(&mut self, path: &str) {
        use crate::core::session_log::SessionLog;
        let path = path.trim();
        if path.is_empty() {
            self.data.load_message = "Give the path of a previously exported .csv file.".into();
            self.data.load_bad = true;
            return;
        }
        match std::fs::read_to_string(path) {
            Err(e) => {
                self.data.load_message = format!("Could not read {path}: {e}");
                self.data.load_bad = true;
            }
            Ok(text) => match SessionLog::from_csv(&text) {
                Err(e) => {
                    self.data.load_message = format!("Could not load {path}: {e}");
                    self.data.load_bad = true;
                    self.data.loaded = None;
                }
                Ok((log, skipped)) => {
                    use crate::core::session_log::Level;
                    self.data.compare_level = [Level::Device, Level::System, Level::App]
                        .into_iter()
                        .max_by_key(|l| log.count(*l))
                        .unwrap_or(Level::Device);
                    self.data.load_message = format!(
                        "Loaded {} event(s){}.",
                        log.events.len(),
                        if skipped > 0 {
                            format!(", skipping {skipped} unreadable row(s)")
                        } else {
                            String::new()
                        }
                    );
                    self.data.load_bad = skipped > 0;
                    self.data.loaded_skipped = skipped;
                    self.data.loaded_from = path.to_string();
                    self.data.loaded = Some(log);
                }
            },
        }
    }

    /// Export the live session, read it straight back, and report whether the
    /// two agree. Used by the unattended capture test, so the file format is
    /// checked against real captured events rather than only against the
    /// synthetic ones in the unit tests.
    pub fn verify_round_trip(&mut self) -> String {
        use std::fmt::Write as _;
        let mut s = String::new();
        let _ = writeln!(s, "\n== session export round trip ==");
        let before = self.session_log();
        self.export_log(&before);
        let Some(path) = self.data.last_raw.clone() else {
            let _ = writeln!(s, "  export failed: {}", self.data.export_message);
            return s;
        };
        self.load_session(&path);
        let Some(after) = self.data.loaded.clone() else {
            let _ = writeln!(s, "  reload failed: {}", self.data.load_message);
            return s;
        };
        let _ = writeln!(s, "  file           {path}");
        let _ = writeln!(
            s,
            "  events         {} written, {} read back, {} unreadable",
            before.events.len(),
            after.events.len(),
            self.data.loaded_skipped
        );
        let _ = writeln!(
            s,
            "  metadata       {}",
            if before.meta == after.meta {
                "identical"
            } else {
                "CHANGED across the round trip"
            }
        );
        let _ = writeln!(
            s,
            "  events match   {}",
            if before.events == after.events {
                "identical"
            } else {
                "CHANGED across the round trip"
            }
        );
        for level in [
            crate::core::session_log::Level::Device,
            crate::core::session_log::Level::System,
            crate::core::session_log::Level::App,
        ] {
            let _ = writeln!(
                s,
                "  {:<8}       {} written, {} read back",
                level.as_str(),
                before.count(level),
                after.count(level)
            );
        }
        let cfg = PollConfig::default();
        let a = crate::core::polling::analyze(
            &before.reports(crate::core::session_log::Level::System),
            &cfg,
        );
        let b = crate::core::polling::analyze(
            &after.reports(crate::core::session_log::Level::System),
            &cfg,
        );
        let _ = writeln!(
            s,
            "  re-analysis    system level sustained {:.4} Hz before, {:.4} Hz after reload",
            a.effective_hz, b.effective_hz
        );
        if let Some(p) = &self.data.last_summary {
            let _ = writeln!(s, "  summary        {p}");
        }
        s
    }

    /// Previously exported logs, newest first.
    pub fn previous_exports(&self) -> Vec<std::path::PathBuf> {
        let mut out: Vec<std::path::PathBuf> = std::fs::read_dir(crate::core::export::dir())
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.extension().and_then(|e| e.to_str()) == Some("csv")
                    && p.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.starts_with("session-"))
            })
            .collect();
        // The stamp is seconds since the epoch, so the name sorts by time.
        out.sort();
        out.reverse();
        out
    }

    // ------------------------------------------------------------ scroll

    pub fn scroll_start(&mut self) {
        self.claim_capture();
        if !self.session.running() {
            self.session.os_build = self.os_build_number();
            self.session.start(self.selected.as_deref());
        }
        self.scroll.capture_lost = false;
        self.scroll.baseline = self.session.device.times_ns.len();
        self.scroll.started = Some(Instant::now());
        self.scroll.phase = SensorPhase::Countdown;
    }

    pub fn scroll_abandon(&mut self) {
        self.scroll.phase = SensorPhase::Idle;
        self.scroll.started = None;
    }

    pub fn scroll_clear(&mut self) {
        self.scroll.vertical = None;
        self.scroll.horizontal = None;
        self.scroll.last_reports = 0;
    }

    fn tick_scroll(&mut self) {
        if self.scroll.phase != SensorPhase::Idle && !self.session.running() {
            self.scroll.phase = SensorPhase::Idle;
            self.scroll.started = None;
            self.scroll.capture_lost = true;
            return;
        }
        match self.scroll.phase {
            SensorPhase::Idle => return,
            SensorPhase::Countdown => {
                if self.scroll.elapsed_s() >= self.scroll.countdown_s {
                    self.scroll.baseline = self.session.device.times_ns.len();
                    self.scroll.started = Some(Instant::now());
                    self.scroll.phase = SensorPhase::Recording;
                }
                return;
            }
            SensorPhase::Recording => {
                if self.scroll.elapsed_s() < self.scroll.capture_s {
                    return;
                }
            }
        }

        let from = self.scroll.baseline.min(self.session.device.times_ns.len());
        let reports = self.session.device.scroll_from(from);
        self.scroll.last_reports = reports
            .iter()
            .filter(|r| r.wheel != 0 || r.hwheel != 0)
            .count();
        let cfg = sensor::scroll::ScrollConfig::default();
        self.scroll.vertical = Some(sensor::scroll::analyze_scroll(&reports, &cfg));
        // Only reported when the wheel actually tilted. A device with no
        // horizontal encoder should not be shown an empty result for one.
        let any_h = reports.iter().any(|r| r.hwheel != 0);
        self.scroll.horizontal = any_h.then(|| {
            sensor::scroll::analyze_axis(&reports, sensor::scroll::Axis::Horizontal, &cfg)
        });
        self.scroll.phase = SensorPhase::Idle;
        self.scroll.started = None;
    }

    /// Fills in one finished result per sensor test, so each result view can be
    /// inspected without a mouse in hand. Command line only; nothing in the
    /// normal path reaches it.
    pub fn sensor_demo(&mut self) {
        use sensor::{cpi, drift, smooth, snap, tracking};
        self.sensor.claimed_cpi = "1600".into();
        self.sensor.distance = "8".into();

        // A sensor counting 7.7% high: past part variation, into "the number on
        // the box is wrong".
        let trials: Vec<cpi::CpiResult> = [1719.0, 1728.0, 1722.0]
            .iter()
            .map(|&m| cpi::CpiResult {
                verdict: sensor::Verdict::Fail,
                measured_cpi: m,
                cpi_sigma: 6.1,
                deviation: (m - 1600.0) / 1600.0,
                deviation_z: 19.0,
                l_net: m * 8.0,
                l_path: m * 8.0 * 1.006,
                l_axis: m * 8.0 * 0.999,
                wobble: 1.006,
                max_off_axis_counts: 91.0,
                n_reports: 512,
                duration_s: 0.48,
                peak_ips: 22.0,
                note: "",
            })
            .collect();
        self.sensor.cpi_summary = Some(cpi::summarize_cpi(&trials, 1600.0));
        self.sensor.cpi_trials = trials;

        let axis = |net: f64, abs: f64, sq: f64, z: f64, zs: f64| drift::AxisStats {
            net,
            abs_sum: abs,
            sum_sq: sq,
            n_nonzero: 61,
            n_pos: 46,
            n_neg: 15,
            z_mean: z,
            p_mean: 0.0001,
            z_sign: zs,
            p_sign: 0.0001,
            directionality: net.abs() / abs,
            directionality_null: 0.10,
            ..Default::default()
        };
        self.sensor.drift = Some(drift::DriftResult {
            verdict: sensor::Verdict::Warn,
            duration_s: 15.0,
            n_reports: 74,
            n_moving_reports: 74,
            x: axis(31.0, 61.0, 61.0, 3.97, 3.97),
            y: axis(-1.0, 13.0, 13.0, -0.28, -0.28),
            jitter_cps: 4.93,
            drift_cps: 2.07,
            drift_ips: 0.0013,
            drift_detected: true,
            jitter_detected: false,
            note: "the pointer is walking, not shimmering: one axis has a consistent bias",
        });

        // The interesting snapping case: the primary test could not run.
        self.sensor.snap = Some(snap::SnapResult {
            verdict: sensor::Verdict::Inconclusive,
            n_reports: 498,
            n_bins: 156,
            bin_ns: 3_000_000,
            travel_in: 4.86,
            median_ips: 11.2,
            sigma_perp_counts: 21.4,
            straightness: 0.00275,
            perp_step_sd: 0.51,
            hf_perp_ratio: 1.94,
            hf_aniso: f64::NAN,
            hf_along_rms: 0.42,
            aniso_applicable: false,
            axis_lock_frac: 0.19,
            axis_lock_expected: 0.18,
            axis_lock_excess: 0.01,
            angle_r_bar: 0.97,
            angle_sd_deg: 13.6,
            angle_on_octant_frac: 0.21,
            note: "this sensor is too quiet for the primary test: with almost no \
                   high-frequency noise along the stroke there is nothing for the \
                   across-stroke comparison to measure. The secondary statistics below \
                   still apply.",
        });

        self.sensor.smooth = Some(smooth::SmoothResult {
            verdict: sensor::Verdict::Fail,
            report_rate_hz: 1000.0,
            n_reports: 601,
            tail_len: 11,
            tail_ms: 14.21,
            tail_reports: 15,
            tail_floor_counts: 2.0,
            decay_tau_ms: 2.95,
            decay_r2: 0.98,
            alpha_from_decay: 0.288,
            rho1_raw: 0.827,
            rho1_corrected: 0.905,
            alpha_from_rho1: 0.095,
            hp_window: 17,
            hf_attenuation_db: 11.4,
            alpha_from_psd: 1.0,
            median_counts_per_report: 29.0,
            n_uniform: 598,
            note: "high-frequency deltas are strongly serially correlated: firmware low-pass",
        });

        self.sensor.tracking = Some(tracking::TrackResult {
            verdict: sensor::Verdict::Warn,
            field: tracking::FieldWidth {
                observed_max: 127,
                matched_bound: Some(127),
                clip_atom: 0.41,
                saturating: true,
            },
            max_tracking_ips: 79.4,
            bounded_below: false,
            peak_observed_ips: 118.0,
            first_failure_ips: 81.2,
            first_failure_reason: "per-report counts pinned at the field maximum",
            n_windows: 214,
            n_failed_windows: 37,
            note: "motion is limited by the report format rather than by the sensor",
        });

        self.section = Section::Sensor;
    }

    /// A finished scroll result, so both axis views can be inspected without a
    /// wheel to turn. Command line only.
    pub fn scroll_demo(&mut self) {
        use sensor::scroll::{Axis, ScrollResult};
        self.scroll.vertical = Some(ScrollResult {
            verdict: sensor::Verdict::Fail,
            continuous: false,
            quantum: 120.0,
            quantum_coverage: 0.96,
            n_clusters: 148,
            detents_up: 74,
            detents_down: 71,
            reversals: 4,
            skips: 2,
            reversal_rate: 4.0 / 148.0,
            skip_rate: 2.0 / 148.0,
            median_gap_ms: 88.0,
            cluster_gap_ms: 12.0,
            axis: Axis::Vertical,
            note: "encoder errors present",
        });
        self.scroll.horizontal = Some(ScrollResult {
            verdict: sensor::Verdict::Pass,
            continuous: false,
            quantum: 120.0,
            quantum_coverage: 1.0,
            n_clusters: 22,
            detents_up: 11,
            detents_down: 11,
            reversals: 0,
            skips: 0,
            reversal_rate: 0.0,
            skip_rate: 0.0,
            median_gap_ms: 143.0,
            cluster_gap_ms: 12.0,
            axis: Axis::Horizontal,
            note: "detents clean",
        });
        self.section = Section::Scroll;
    }

    /// Selects one sensor test by name, for the command line.
    pub fn select_sensor_test(&mut self, name: &str) {
        let n = name.to_ascii_lowercase();
        for t in sensor::protocol::Test::ALL {
            if format!("{t:?}").to_ascii_lowercase() == n {
                self.sensor.test = t;
            }
        }
    }

    /// Selects a section by name, for the command line.
    pub fn select_section(&mut self, name: &str) {
        let n = name.to_ascii_lowercase();
        for s in Section::ALL {
            if s.title().to_ascii_lowercase() == n {
                self.section = s;
            }
        }
    }

    /// The Windows build number, parsed out of what the environment reported.
    /// Zero everywhere else, and unused there.
    fn os_build_number(&self) -> u32 {
        self.survey
            .env
            .os_build
            .split(|c: char| !c.is_ascii_digit())
            .filter(|s| !s.is_empty())
            .next_back()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0)
    }

    /// Begins a capture, optionally after a delay.
    /// Another section is taking over the shared capture.
    ///
    /// Every section but POLLING reuses a capture that is already running
    /// rather than restarting it, so `start_capture` does not run and nothing
    /// else would clear the arming. A polling run left armed underneath one of
    /// those will stop the capture the moment its verdict settles, and the
    /// section on screen carries on counting down against a dead capture:
    /// reports frozen, timer running, and a refusal at the end that reads like
    /// the mouse stopped moving. Any new section that shares the capture must
    /// call this.
    pub fn claim_capture(&mut self) {
        self.poll_armed = false;
    }

    /// Start a run from the POLLING section, which is the only one allowed to
    /// end early on its own.
    pub fn start_poll_run(&mut self, delay_s: f64) {
        self.start_capture(delay_s);
        self.poll_armed = true;
    }

    pub fn start_capture(&mut self, delay_s: f64) {
        self.poll_armed = false;
        self.poll_auto_stopped = false;
        if delay_s > 0.0 {
            self.countdown = Some((Instant::now(), delay_s));
        } else {
            self.countdown = None;
            let key = self.selected.clone();
            self.session.os_build = self.os_build_number();
            self.session.start(key.as_deref());
            self.poll_result = PollResult::default();
        }
    }

    pub fn countdown_remaining(&self) -> Option<f64> {
        let (start, total) = self.countdown?;
        let left = total - start.elapsed().as_secs_f64();
        if left > 0.0 {
            Some(left)
        } else {
            None
        }
    }

    fn drive_auto_capture(&mut self, ctx: &egui::Context) {
        let (seconds, out, started) = match &self.auto_capture {
            Some(a) => (a.seconds, a.out.clone(), a.started),
            None => return,
        };
        match started {
            None => {
                self.section = Section::Polling;
                self.start_capture(0.0);
                if let Some(a) = &mut self.auto_capture {
                    a.started = Some(Instant::now());
                }
                ctx.request_repaint();
            }
            Some(t) if t.elapsed().as_secs_f64() >= seconds => {
                self.poll_result = self.session.analyze_device(&PollConfig::default());
                // Exercise the export and reload path against a real capture,
                // and append what it found. A round trip that only ever runs on
                // synthetic events has not been shown to work on real ones.
                let round_trip = self.verify_round_trip();
                let mut report = crate::dump::capture_report(self);
                report.push_str(&round_trip);
                if let Err(e) = std::fs::write(&out, report) {
                    eprintln!("could not write {out}: {e}");
                } else {
                    eprintln!("wrote {out}");
                }
                self.session.stop();
                self.auto_capture = None;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            _ => {
                ctx.request_repaint_after(std::time::Duration::from_millis(30));
            }
        }
    }

    fn tick(&mut self, ctx: &egui::Context) {
        // Fire a pending delayed start.
        if let Some((start, total)) = self.countdown {
            if start.elapsed().as_secs_f64() >= total {
                self.countdown = None;
                let key = self.selected.clone();
                self.session.os_build = self.os_build_number();
                self.session.start(key.as_deref());
                self.poll_result = PollResult::default();
            }
        }

        self.session.pump();
        self.session.pump_app(ctx);
        self.tick_cps();
        self.tick_ab();
        self.tick_sensor();
        self.tick_scroll();

        // Re-analysing a long capture every frame would make the interface
        // the slowest thing in the program, so it runs a few times a second.
        let due = self
            .last_analysis
            .map(|t| t.elapsed().as_millis() >= 200)
            .unwrap_or(true);
        if due && self.session.device.total > 0 {
            let cfg = PollConfig::default();
            self.poll_result = self.session.analyze_device(&cfg);
            self.last_analysis = Some(Instant::now());

            // End the run as soon as more swiping cannot move the answer. The
            // test otherwise asks for effort it has no use for, and a person
            // spinning a mouse for another twenty seconds is not producing
            // information, only fatigue.
            if self.poll_armed
                && self.poll_auto_stop
                && self.section == Section::Polling
                && self.session.running()
                && verdict_settled(&self.poll_result, &cfg)
            {
                self.session.stop();
                self.poll_armed = false;
                self.poll_auto_stopped = true;
            }
        }
    }

    pub fn refresh(&mut self) {
        self.survey = Survey::run(self.selected.as_deref());
        if !self
            .survey
            .devices
            .iter()
            .any(|d| Some(&d.key) == self.selected.as_ref())
        {
            self.selected = self
                .survey
                .devices
                .iter()
                .find(|d| d.streamable)
                .or_else(|| self.survey.devices.first())
                .map(|d| d.key.clone());
        }
    }

    pub fn selected_device(&self) -> Option<&DeviceInfo> {
        let key = self.selected.as_ref()?;
        self.survey.devices.iter().find(|d| &d.key == key)
    }
}

impl eframe::App for App {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 1.0]
    }

    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if let Some(job) = &mut self.screenshot {
            if job.step(ctx) {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }

        // Keyboard trigger. Only reaches us while the window has focus, which
        // is stated in the section rather than left to be discovered.
        let toggle = ctx.input(|i| {
            i.key_pressed(egui::Key::F5) || i.key_pressed(egui::Key::Space)
        });
        if toggle {
            if self.session.running() {
                self.session.stop();
            } else {
                self.start_capture(0.0);
            }
        }

        if self.auto_capture.is_some() {
            self.drive_auto_capture(ctx);
        }

        self.tick(ctx);

        // While a capture is live, redraw as fast as the display allows. The
        // application level is bounded by how often this program draws, so
        // throttling here would make that level look worse than a normal
        // application's and misattribute the difference to the mouse.
        if self.session.running()
            || matches!(self.ab.phase, AbPhase::Countdown | AbPhase::Trial)
        {
            ctx.request_repaint();
        } else if self.countdown.is_some() || self.cps.countdown.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::Panel::left("sidebar")
            .resizable(false)
            .exact_size(148.0)
            .frame(theme::panel_frame(0))
            .show(ui, |ui| {
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.add_space(8.0);
                    ui.add(egui::Label::new(
                        egui::RichText::new("MOUSE TESTING").color(theme::WHITE),
                    ));
                });
                ui.add_space(8.0);
                for s in Section::ALL {
                    if widgets::nav_row(ui, self.section == s, s.title()).clicked() {
                        self.section = s;
                    }
                }
                ui.add_space(8.0);
                widgets::rule(ui);
                ui.horizontal(|ui| {
                    ui.add_space(8.0);
                    ui.vertical(|ui| {
                        ui.add(egui::Label::new(
                            egui::RichText::new(format!(
                                "{} device{}",
                                self.survey.devices.len(),
                                if self.survey.devices.len() == 1 { "" } else { "s" }
                            ))
                            .small()
                            .color(theme::GREY_TEXT),
                        ));
                        let worst = self.survey.env.worst();
                        let (lvl, txt) = match worst {
                            Some(platform::WarnLevel::Fail) => (theme::Level::Fail, "environment"),
                            Some(platform::WarnLevel::Warn) => (theme::Level::Warn, "environment"),
                            _ => (theme::Level::Pass, "environment"),
                        };
                        ui.add(egui::Label::new(
                            egui::RichText::new(txt).small().color(lvl.color()),
                        ));
                    });
                });
            });

        egui::CentralPanel::default()
            .frame(theme::panel_frame(10))
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| match self.section {
                        Section::Device => sections::device::show(self, ui),
                        Section::Polling => sections::polling::show(self, ui),
                        Section::Clicks => sections::clicks::show(self, ui),
                        Section::Cps => sections::cps::show(self, ui),
                        Section::Ab => sections::ab::show(self, ui),
                        Section::Sensor => sections::sensor::show(self, ui),
                        Section::Scroll => sections::scroll::show(self, ui),
                        Section::Session => sections::session::show(self, ui),
                    });
            });
    }
}
