use crate::capture::Session;
use crate::core::polling::{PollConfig, PollResult};
use crate::platform::{self, AccessReport, DeviceInfo, HostEnv};
use crate::ui::{sections, theme, widgets};
use std::time::Instant;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Device,
    Polling,
}

impl Section {
    pub const ALL: [Section; 2] = [Section::Device, Section::Polling];

    pub fn title(self) -> &'static str {
        match self {
            Section::Device => "DEVICE",
            Section::Polling => "POLLING",
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
        if self.session.running() {
            ctx.request_repaint();
        } else if self.countdown.is_some() {
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
                    });
            });
    }
}
