//! Device identification: what is attached, what access we got, and what about
//! this machine would make a timing measurement untrustworthy.

use crate::app::App;
use crate::platform::{Availability, DeviceInfo, Link, Tier};
use crate::ui::theme::{self, Level};
use crate::ui::widgets as w;

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    w::heading(ui, "DEVICE IDENTIFICATION");

    ui.horizontal(|ui| {
        if ui.button("refresh").clicked() {
            app.refresh();
        }
        ui.add_space(8.0);
        ui.add(egui::Label::new(
            egui::RichText::new("Re-enumerates devices and re-probes access.")
                .color(theme::GREY_TEXT),
        ));
    });

    ui.add_space(8.0);
    device_list(app, ui);

    ui.add_space(10.0);
    if let Some(dev) = app.selected_device().cloned() {
        identifiers(ui, &dev);
        ui.add_space(10.0);
        topology(ui, &dev);
    } else {
        w::status_line(
            ui,
            Level::Warn,
            "No pointing device is attached. Connect one and press refresh.",
        );
    }

    ui.add_space(10.0);
    access(app, ui);

    ui.add_space(10.0);
    environment(app, ui);
}

fn device_list(app: &mut App, ui: &mut egui::Ui) {
    w::subheading(ui, "attached pointing devices");
    let devices: Vec<DeviceInfo> = app.survey.devices.clone();
    if devices.is_empty() {
        w::status_line(ui, Level::Warn, "none found");
        return;
    }
    w::boxed(ui, |ui| {
        for d in &devices {
            let selected = app.selected.as_deref() == Some(d.key.as_str());
            let label = format!(
                "{:<34} {:<11} {}",
                truncate(&d.name, 34),
                d.transport.clone().unwrap_or_else(|| "-".into()),
                d.ids()
            );
            let resp = w::nav_row(ui, selected, &label);
            if resp.clicked() {
                app.selected = Some(d.key.clone());
                app.refresh();
                // Rebind a live capture straight away. It keeps streaming
                // whatever it opened, so leaving it alone means every reading
                // on screen still describes the device you just stopped
                // choosing, and nothing says so.
                if app.session.running() {
                    app.restart_capture();
                }
            }
        }
    });
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        s.chars().take(n.saturating_sub(1)).collect::<String>() + "\u{2026}"
    }
}

fn identifiers(ui: &mut egui::Ui, d: &DeviceInfo) {
    w::subheading(ui, "identifiers");
    w::boxed(ui, |ui| {
        w::kv(ui, "name", &d.name);
        w::kv(
            ui,
            "manufacturer",
            d.manufacturer.as_deref().unwrap_or("not reported"),
        );
        w::kv(ui, "product", d.product.as_deref().unwrap_or("not reported"));
        w::kv(
            ui,
            "vendor / product id",
            &match (d.vendor_id, d.product_id) {
                (Some(v), Some(p)) => format!("0x{v:04X} / 0x{p:04X}"),
                _ => "not reported".into(),
            },
        );
        w::kv(
            ui,
            "version",
            &d.version
                .map(|v| format!("0x{v:04X}"))
                .unwrap_or_else(|| "not reported".into()),
        );
        w::kv(
            ui,
            "serial",
            d.serial.as_deref().unwrap_or("not reported"),
        );
        w::kv(
            ui,
            "hid usage",
            &match (d.usage_page, d.usage) {
                (Some(p), Some(u)) => format!(
                    "page 0x{p:02X} usage 0x{u:02X}{}",
                    if p == 1 && u == 2 {
                        "  (generic desktop mouse)"
                    } else if p == 1 && u == 1 {
                        "  (generic desktop pointer)"
                    } else {
                        ""
                    }
                ),
                _ => "not reported".into(),
            },
        );

        // Advertised, never measured. The polling section is what turns this
        // into a real number.
        match (d.advertised_interval_us, d.advertised_interval_trusted) {
            (Some(us), true) => w::kv(
                ui,
                "advertised interval",
                &format!("{} us  ({:.0} Hz nominal)", us, 1_000_000.0 / us as f64),
            ),
            (Some(us), false) => w::kv_level(
                ui,
                "advertised interval",
                &format!(
                    "{us} us reported, not trustworthy on this transport (placeholder value)"
                ),
                Level::Off,
            ),
            (None, _) => w::kv(ui, "advertised interval", "not reported"),
        }

        if let Some(n) = d.buttons_reported {
            w::kv(ui, "buttons reported", &n.to_string());
        }
        if let Some(h) = d.has_horizontal_wheel {
            w::kv(ui, "horizontal wheel", if h { "yes" } else { "no" });
        }
        for (k, v) in &d.extra {
            w::kv(ui, k, v);
        }
        if let Some(p) = &d.raw_path {
            w::kv(ui, "os identifier", p);
        }
    });

    if !d.streamable {
        if let Some(reason) = &d.not_streamable_reason {
            ui.add_space(4.0);
            w::status_line(ui, Level::Warn, reason);
        }
    }
}

fn topology(ui: &mut egui::Ui, d: &DeviceInfo) {
    w::subheading(ui, "connection");
    w::boxed(ui, |ui| {
        let (text, level) = match &d.topology.link {
            Link::Usb { hub_depth, speed } => {
                let hubs = match hub_depth {
                    Some(0) => "direct to a port on the host controller".to_string(),
                    Some(n) => format!("behind {n} external hub(s)"),
                    None => "hub depth could not be determined".to_string(),
                };
                let sp = speed.clone().unwrap_or_else(|| "speed not reported".into());
                (
                    format!("USB, {hubs}, {sp}"),
                    match hub_depth {
                        Some(0) => Level::Pass,
                        Some(_) => Level::Warn,
                        None => Level::Info,
                    },
                )
            }
            Link::Bluetooth => ("Bluetooth".to_string(), Level::Warn),
            Link::Internal => ("built in, internal transport".to_string(), Level::Info),
            Link::Virtual => (
                "software HID device, not physical hardware".to_string(),
                Level::Fail,
            ),
            Link::Unknown => ("could not be determined".to_string(), Level::Info),
        };
        w::kv_level(ui, "link", &text, level);
        if !d.topology.chain.is_empty() {
            w::kv(
                ui,
                "parent chain",
                &d.topology.chain.join("  <  "),
            );
        }
    });
}

fn access(app: &mut App, ui: &mut egui::Ui) {
    w::subheading(ui, "access obtained");
    let items = app.survey.access.items.clone();
    w::boxed(ui, |ui| {
        for item in &items {
            let tier = item
                .tier
                .map(|t| format!("{:<7}", t.short()))
                .unwrap_or_else(|| "       ".to_string());
            ui.horizontal(|ui| {
                ui.add(egui::Label::new(
                    egui::RichText::new(format!("[{}]", item.state.level().tag()))
                        .color(item.state.level().color()),
                ));
                ui.add(egui::Label::new(
                    egui::RichText::new(tier).color(theme::GREY_TEXT),
                ));
                ui.add(egui::Label::new(
                    egui::RichText::new(&item.name).color(theme::WHITE),
                ));
            });
            w::note_indent(ui, 14.0, &item.detail);
            if let Some(src) = item.tier.map(|t| t.source_name()) {
                w::note_indent(ui, 14.0, &format!("source: {src}"));
            }
            if let Some(remedy) = &item.remedy {
                ui.add_space(2.0);
                w::note_indent(ui, 14.0, remedy);
                ui.horizontal(|ui| {
                    ui.add_space(14.0);
                    #[cfg(target_os = "macos")]
                    {
                        if ui.button("open the settings pane").clicked() {
                            if let Some(link) = &item.remedy_link {
                                let _ = std::process::Command::new("open").arg(link).spawn();
                            }
                        }
                        // Only prompts if macOS has not already recorded a
                        // decision; otherwise it returns silently, which is why
                        // the Settings path is offered alongside it.
                        if ui.button("ask macOS now").clicked() {
                            crate::platform::macos::access::request_listen();
                            app.refresh();
                        }
                    }
                    #[cfg(not(target_os = "macos"))]
                    {
                        let _ = &item.remedy_link;
                    }
                });
            }
            ui.add_space(4.0);
        }
    });

    // The one thing a user most needs to know, said once, plainly.
    let dev_state = app.survey.access.tier_state(Tier::Device);
    if dev_state == Availability::Denied {
        ui.add_space(4.0);
        w::status_line(
            ui,
            Level::Fail,
            "Device tier is blocked. Identification below is complete, but no rate can be \
             measured until Input Monitoring is granted. Nothing here will report zero \
             instead of saying so.",
        );
    }
}

fn environment(app: &mut App, ui: &mut egui::Ui) {
    w::subheading(ui, "host environment");
    let env = app.survey.env.clone();
    w::boxed(ui, |ui| {
        w::kv(
            ui,
            "operating system",
            &format!("{} ({})", env.os, env.os_build),
        );
        w::kv(ui, "architecture", &env.arch);
        w::kv(ui, "processor", &env.cpu);
        w::kv(ui, "cores", &env.cores);
        for (k, v) in &env.facts {
            w::kv(ui, k, v);
        }
        w::kv(ui, "timestamp clock", &env.timer.name);
        w::readout(
            ui,
            "clock resolution",
            &format!("{:.1}", env.timer.resolution_ns),
            9,
            "ns",
            Level::Info,
        );
        w::readout(
            ui,
            "clock read cost",
            &format!("{:.1}", env.timer.cost_ns),
            9,
            "ns",
            Level::Info,
        );
        for n in &env.timer.notes {
            w::note_indent(ui, 14.0, n);
        }
    });

    ui.add_space(8.0);
    w::subheading(ui, "measurement validity");
    w::boxed(ui, |ui| {
        if env.warnings.is_empty() {
            w::status_line(
                ui,
                Level::Pass,
                "Nothing detected that would invalidate timing measurements.",
            );
            return;
        }
        for warning in &env.warnings {
            w::status_line(ui, warning.level.level(), &warning.title);
            w::note_indent(ui, 14.0, &warning.detail);
            ui.add_space(4.0);
        }
    });
}
