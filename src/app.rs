use crate::capture::Session;
use crate::core::ab;
use crate::core::cps;
use crate::core::polling::{PollConfig, PollResult};
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
}

impl Section {
    pub const ALL: [Section; 5] = [
        Section::Device,
        Section::Polling,
        Section::Clicks,
        Section::Cps,
        Section::Ab,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Section::Device => "DEVICE",
            Section::Polling => "POLLING",
            Section::Clicks => "CLICKS",
            Section::Cps => "CPS",
            Section::Ab => "A/B",
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
            countdown: None,
            last_analysis: None,
            auto_capture: None,
            cps: CpsState::default(),
            ab: AbState::default(),
        }
    }

    pub fn ab_start(&mut self) {
        if !self.plan_ready() {
            return;
        }
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

    /// Selects a section by name, for the command line.
    pub fn select_section(&mut self, name: &str) {
        let n = name.to_ascii_lowercase();
        for s in Section::ALL {
            if s.title().to_ascii_lowercase() == n {
                self.section = s;
            }
        }
    }

    /// Begins a capture, optionally after a delay.
    pub fn start_capture(&mut self, delay_s: f64) {
        if delay_s > 0.0 {
            self.countdown = Some((Instant::now(), delay_s));
        } else {
            self.countdown = None;
            let key = self.selected.clone();
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
                let report = crate::dump::capture_report(self);
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
                self.session.start(key.as_deref());
                self.poll_result = PollResult::default();
            }
        }

        self.session.pump();
        self.session.pump_app(ctx);
        self.tick_cps();
        self.tick_ab();

        // Re-analysing a long capture every frame would make the interface
        // the slowest thing in the program, so it runs a few times a second.
        let due = self
            .last_analysis
            .map(|t| t.elapsed().as_millis() >= 200)
            .unwrap_or(true);
        if due && self.session.device.total > 0 {
            self.poll_result = self.session.analyze_device(&PollConfig::default());
            self.last_analysis = Some(Instant::now());
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
                    });
            });
    }
}
