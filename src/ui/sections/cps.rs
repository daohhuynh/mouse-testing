//! Clicks per second.

use crate::app::App;
use crate::core::cps;
use crate::ui::theme::{self, Level};
use crate::ui::widgets as w;

/// Technique labels. Recorded with the result and nothing else: none of them
/// changes how anything is measured, and the interface says so rather than
/// leaving anyone to wonder.
pub const MODES: [&str; 4] = ["normal", "drag click", "butterfly", "jitter"];
pub const DURATIONS: [f64; 4] = [5.0, 10.0, 30.0, 60.0];

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    w::heading(ui, "CLICKS PER SECOND");

    setup(app, ui);
    ui.add_space(8.0);
    current(app, ui);
    ui.add_space(10.0);
    history(app, ui);
}

fn setup(app: &mut App, ui: &mut egui::Ui) {
    let live = app.cps.running;

    ui.horizontal(|ui| {
        w::fixed_label(ui, "technique", 12, Level::Off);
        for (i, m) in MODES.iter().enumerate() {
            let sel = app.cps.mode == i;
            if ui
                .add_enabled(!live, egui::Button::new(*m).selected(sel))
                .clicked()
            {
                app.cps.mode = i;
            }
        }
    });
    w::note_indent(
        ui,
        0.0,
        "A label only. It is stored with the result so you can compare like with like; it \
         does not change what is counted or how.",
    );

    ui.add_space(4.0);
    ui.horizontal(|ui| {
        w::fixed_label(ui, "duration", 12, Level::Off);
        for d in DURATIONS {
            let sel = (app.cps.duration_s - d).abs() < 0.01;
            if ui
                .add_enabled(!live, egui::Button::new(format!("{d:.0} s")).selected(sel))
                .clicked()
            {
                app.cps.duration_s = d;
            }
        }
    });

    ui.add_space(4.0);
    ui.horizontal(|ui| {
        w::fixed_label(ui, "button", 12, Level::Off);
        let sel = app.cps.button.is_none();
        if ui
            .add_enabled(!live, egui::Button::new("any").selected(sel))
            .clicked()
        {
            app.cps.button = None;
        }
        for b in 1u8..=5 {
            let sel = app.cps.button == Some(b);
            if ui
                .add_enabled(!live, egui::Button::new(format!("{b}")).selected(sel))
                .clicked()
            {
                app.cps.button = Some(b);
            }
        }
    });

    ui.add_space(6.0);
    ui.horizontal(|ui| {
        if live {
            if ui.button("abandon run").clicked() {
                app.cps_abort();
            }
        } else {
            if ui.button("start").clicked() {
                app.cps_start(0.0);
            }
            ui.add_space(8.0);
            ui.add(egui::Label::new(
                egui::RichText::new("after").color(theme::GREY_TEXT),
            ));
            for d in [3.0f64, 5.0] {
                if ui.button(format!("{d:.0} s")).clicked() {
                    app.cps_start(d);
                }
            }
        }
    });
}

fn current(app: &mut App, ui: &mut egui::Ui) {
    if let Some(remaining) = app.cps.countdown_remaining() {
        w::status_line(
            ui,
            Level::Warn,
            &format!("starting in {remaining:.1} s"),
        );
        return;
    }

    if !app.cps.running {
        if let Some(last) = app.cps.history.last().cloned() {
            result_block(ui, &last, "last run");
        } else {
            w::status_line(
                ui,
                Level::Off,
                "No run yet. Pick a duration and press start; clicks are counted from \
                 whichever level is supplying button events.",
            );
        }
        return;
    }

    let elapsed = app.cps.elapsed_s();
    let remaining = (app.cps.duration_s - elapsed).max(0.0);
    let presses = app.cps_presses().len();
    w::boxed(ui, |ui| {
        w::readout(ui, "time remaining", &format!("{remaining:.1}"), 9, "s", Level::Warn);
        w::readout(ui, "clicks so far", &format!("{presses}"), 9, "", Level::Info);
        let so_far = if elapsed > 0.0 {
            presses as f64 / elapsed
        } else {
            0.0
        };
        w::readout(ui, "rate so far", &format!("{so_far:.2}"), 9, "CPS", Level::Info);
    });
}

fn result_block(ui: &mut egui::Ui, run: &cps::Run, title: &str) {
    w::subheading(ui, title);
    w::boxed(ui, |ui| {
        // Sustained first and alone at the top: it is the figure that means
        // something.
        w::readout(
            ui,
            "sustained",
            &format!("{:.2}", run.sustained_cps),
            9,
            "CPS",
            Level::Pass,
        );
        w::note_indent(
            ui,
            0.0,
            "Clicks divided by the full length of the run. This is the number to quote.",
        );
        ui.add_space(4.0);
        w::readout(ui, "peak", &format!("{:.2}", run.peak_cps), 9, "CPS", Level::Info);
        w::note_indent(
            ui,
            0.0,
            "The busiest single second anywhere in the run. On a short test this is mostly \
             luck about where that second fell, so it is reported second and should not be \
             compared between runs.",
        );
        ui.add_space(4.0);
        w::readout(ui, "clicks", &format!("{}", run.presses), 9, "", Level::Info);
        w::readout(ui, "duration", &format!("{:.0}", run.duration_s), 9, "s", Level::Info);
        w::kv(ui, "technique", &run.mode);
        w::kv(
            ui,
            "button",
            &if run.button == 0 {
                "any".to_string()
            } else {
                run.button.to_string()
            },
        );
    });
}

fn history(app: &mut App, ui: &mut egui::Ui) {
    w::subheading(ui, "runs this session");
    let runs = app.cps.history.clone();
    w::boxed(ui, |ui| {
        if runs.is_empty() {
            w::note_indent(ui, 0.0, "Nothing yet.");
            return;
        }
        ui.horizontal(|ui| {
            w::fixed_label(ui, "#", 4, Level::Off);
            w::fixed_label(ui, "technique", 12, Level::Off);
            w::fixed_label(ui, "btn", 5, Level::Off);
            w::fixed_label(ui, "secs", 6, Level::Off);
            w::fixed_label(ui, "clicks", 8, Level::Off);
            w::fixed_label(ui, "sustained", 11, Level::Off);
            w::fixed_label(ui, "peak", 9, Level::Off);
        });
        for (i, r) in runs.iter().enumerate().rev() {
            ui.horizontal(|ui| {
                w::fixed_value(ui, &format!("{}", i + 1), 3, Level::Off);
                ui.add_space(4.0);
                w::fixed_label(ui, &r.mode, 12, Level::Info);
                w::fixed_value(ui, &if r.button == 0 { "any".into() } else { r.button.to_string() }, 4, Level::Info);
                ui.add_space(4.0);
                w::fixed_value(ui, &format!("{:.0}", r.duration_s), 5, Level::Info);
                w::fixed_value(ui, &format!("{}", r.presses), 7, Level::Info);
                w::fixed_value(ui, &format!("{:.2}", r.sustained_cps), 10, Level::Pass);
                w::fixed_value(ui, &format!("{:.2}", r.peak_cps), 8, Level::Info);
            });
        }
        ui.add_space(4.0);
        if ui.button("clear history").clicked() {
            app.cps.history.clear();
        }
    });
}
