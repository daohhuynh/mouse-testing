use crate::platform::{self, AccessReport, DeviceInfo, HostEnv};
use crate::ui::{sections, theme, widgets};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Device,
}

impl Section {
    pub const ALL: [Section; 1] = [Section::Device];

    pub fn title(self) -> &'static str {
        match self {
            Section::Device => "DEVICE",
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
                    });
            });
    }
}
