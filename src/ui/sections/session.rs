//! Session data: what was captured, exporting it, and reading it back.

use crate::app::App;
use crate::core::session_log::{Level, SessionLog};
use crate::ui::theme::Level as L;
use crate::ui::widgets as w;

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    w::heading(ui, "SESSION DATA");

    current(app, ui);
    ui.add_space(10.0);
    battery(app, ui);
    ui.add_space(10.0);
    export(app, ui);
    ui.add_space(10.0);
    load(app, ui);

    if app.data.loaded.is_some() {
        ui.add_space(10.0);
        compare(app, ui);
    }
}

fn current(app: &mut App, ui: &mut egui::Ui) {
    w::subheading(ui, "this session");
    w::boxed(ui, |ui| {
        w::readout(
            ui,
            "capture running for",
            &format!("{:.1}", app.session.elapsed_s()),
            10,
            "s",
            if app.session.running() { L::Pass } else { L::Off },
        );
        for (name, series) in [
            ("device level", &app.session.device),
            ("system level", &app.session.system),
            ("app level", &app.session.app),
        ] {
            w::readout(
                ui,
                name,
                &format!("{}", series.total),
                10,
                "event(s)",
                L::Info,
            );
        }
        w::readout(
            ui,
            "button transitions",
            &format!("{}", app.session.buttons.len()),
            10,
            "",
            L::Info,
        );
        if app.session.background_events > 0 {
            w::readout(
                ui,
                "while not in front",
                &format!("{}", app.session.background_events),
                10,
                "event(s)",
                L::Pass,
            );
        }
        if app.session.injected_events > 0 {
            w::readout(
                ui,
                "synthesised by software",
                &format!("{}", app.session.injected_events),
                10,
                "event(s)",
                L::Fail,
            );
            w::note_indent(
                ui,
                14.0,
                "These did not come from the mouse. Any measurement covering them \
                 describes whatever program produced them.",
            );
        }
        let losses = app.session.device.ring_drops
            + app.session.system.ring_drops
            + app.session.app.ring_drops;
        w::readout(
            ui,
            "buffer losses",
            &format!("{losses}"),
            10,
            "",
            if losses > 0 { L::Warn } else { L::Pass },
        );
        ui.add_space(2.0);
        w::note_indent(
            ui,
            0.0,
            "Buffer losses are events the capture threads produced faster than the \
             interface drained them. They are counted rather than hidden, because a rate \
             computed from a series with a hole in it is wrong and there is no way to tell \
             from the numbers alone.",
        );
    });
}

/// Which measurements count as part of this configuration.
///
/// The list is the experiment. A summary that only ever said "not measured this
/// session" could not tell a test the user never reached from one they left out
/// on purpose, and in a comparison those mean opposite things: the first is a
/// hole in the evidence, the second is the shape of the experiment.
fn battery(app: &mut App, ui: &mut egui::Ui) {
    use crate::core::battery;
    w::subheading(ui, "the battery");
    w::boxed(ui, |ui| {
        w::note_indent(
            ui,
            0.0,
            "The measurements that belong to this configuration. Highlighted ones are in; \
             click one to leave it out. Only those that are in are written to the export and \
             compared against a previous one, so a test you deliberately skipped does not \
             come back later looking like a test that went missing.",
        );
        ui.add_space(6.0);
        for item in battery::ITEMS {
            let on = !app.battery_off.contains(item.key);
            let have = battery::measure(app, item.key);
            ui.horizontal(|ui| {
                // Padded to one width so the column beside it lines up. The
                // rest of the app holds its columns still whatever the value,
                // and a ragged list here would read as a different program.
                if w::chip(ui, on, &format!("{:<22}", item.label)).clicked() {
                    if on {
                        app.battery_off.insert(item.key.to_string());
                    } else {
                        app.battery_off.remove(item.key);
                    }
                }
                ui.add_space(6.0);
                match (&have, on) {
                    (_, false) => {
                        w::fixed_value(ui, "left out", 12, L::Off);
                    }
                    (Some(r), true) => {
                        let lvl = r.verdict.map(w::level_of).unwrap_or(L::Info);
                        w::fixed_value(ui, "measured", 12, lvl);
                        w::note_indent(ui, 2.0, &r.headline);
                    }
                    (None, true) => {
                        w::fixed_value(ui, "not yet", 12, L::Warn);
                    }
                }
            });
        }
    });
}

fn export(app: &mut App, ui: &mut egui::Ui) {
    w::subheading(ui, "export");
    w::boxed(ui, |ui| {
        w::note_indent(
            ui,
            0.0,
            "Two files: every captured event as CSV, and a readable summary of everything \
             measured. The CSV carries the device, the host and the clock's own resolution \
             in comment lines at the top, so it can be re-analysed on another machine \
             without losing the context that says whether the numbers can be trusted.",
        );
        ui.add_space(6.0);
        if ui.button("export raw data and summary").clicked() {
            app.export_session();
        }
        if let Some(p) = &app.data.last_raw {
            ui.add_space(4.0);
            w::kv(ui, "raw events", p);
        }
        if let Some(p) = &app.data.last_summary {
            w::kv(ui, "summary", p);
        }
        if !app.data.export_message.is_empty() {
            ui.add_space(4.0);
            w::status_line(
                ui,
                if app.data.export_bad { L::Warn } else { L::Pass },
                &app.data.export_message,
            );
        }
    });
}

fn load(app: &mut App, ui: &mut egui::Ui) {
    w::subheading(ui, "load a previous export");
    w::boxed(ui, |ui| {
        ui.horizontal(|ui| {
            w::fixed_label(ui, "path", 8, L::Off);
            ui.add(
                egui::TextEdit::singleline(&mut app.data.load_path)
                    .desired_width(520.0)
                    .hint_text("a session-*.csv written by this program"),
            );
            if ui.button("load").clicked() {
                let p = app.data.load_path.clone();
                app.load_session(&p);
            }
        });
        if !app.data.load_message.is_empty() {
            ui.add_space(4.0);
            w::status_line(
                ui,
                if app.data.load_bad { L::Warn } else { L::Pass },
                &app.data.load_message,
            );
        }

        let previous = app.previous_exports();
        ui.add_space(6.0);
        if previous.is_empty() {
            w::note_indent(ui, 0.0, "No previous exports found in the export folder.");
        } else {
            w::note_indent(ui, 0.0, "Previous exports, newest first:");
            for p in previous.iter().take(12) {
                let name = p
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
                    .to_string();
                let sel = app.data.loaded_from == p.display().to_string();
                if w::nav_row(ui, sel, &name).clicked() {
                    app.data.load_path = p.display().to_string();
                    let path = app.data.load_path.clone();
                    app.load_session(&path);
                }
            }
        }
    });
}

fn compare(app: &mut App, ui: &mut egui::Ui) {
    let Some(loaded) = app.data.loaded.clone() else {
        return;
    };
    w::subheading(ui, "loaded session");
    w::boxed(ui, |ui| {
        let m = &loaded.meta;
        w::kv(ui, "file", &app.data.loaded_from);
        w::kv(ui, "device", &m.device_name);
        w::kv(ui, "identifiers", &m.device_ids);
        w::kv(ui, "host", &format!("{} on {}", m.os, m.arch));
        w::kv(
            ui,
            "clock",
            &format!(
                "{}, {:.1} ns resolution, {:.1} ns to read",
                m.clock, m.clock_resolution_ns, m.clock_cost_ns
            ),
        );
        w::readout(ui, "duration", &format!("{:.1}", m.duration_s), 10, "s", L::Info);
        if app.data.loaded_skipped > 0 {
            ui.add_space(4.0);
            w::status_line(
                ui,
                L::Warn,
                &format!(
                    "{} row(s) in that file could not be read and were skipped. The figures \
                     below are computed from what loaded.",
                    app.data.loaded_skipped
                ),
            );
        }
        if !m.warnings.is_empty() {
            ui.add_space(4.0);
            w::note_indent(ui, 0.0, "Recorded at capture time on that machine:");
            for warning in &m.warnings {
                w::status_line(ui, L::Warn, warning);
            }
        }
    });

    ui.add_space(10.0);
    results_side_by_side(app, ui);

    ui.add_space(10.0);
    w::subheading(ui, "side by side");
    w::boxed(ui, |ui| {
        ui.horizontal(|ui| {
            w::fixed_label(ui, "level", 8, L::Off);
            for level in Level::ALL {
                let sel = app.data.compare_level == level;
                let label = format!("{} ({})", level.as_str(), loaded.count(level));
                if w::chip(ui, sel, &label).clicked() {
                    app.data.compare_level = level;
                }
            }
        });
        w::note_indent(
            ui,
            0.0,
            "The count in each button is what the loaded file holds at that level, so a \
             level with nothing in it is visible before you select it.",
        );
    });

    ui.add_space(6.0);
    let level = app.data.compare_level;
    let cfg = crate::core::polling::PollConfig::default();
    let now = app.session.tier_series(match level {
        Level::Device => crate::platform::Tier::Device,
        Level::System => crate::platform::Tier::System,
        Level::App => crate::platform::Tier::App,
    })
    .reports();
    let then = loaded.reports(level);
    let a = crate::core::polling::analyze(&now, &cfg);
    let b = crate::core::polling::analyze(&then, &cfg);

    w::boxed(ui, |ui| {
        ui.horizontal(|ui| {
            w::fixed_label(ui, &format!("{} level", level.as_str()), w::LABEL_CHARS, L::Off);
            w::fixed_label(ui, "now", 14, L::Off);
            w::fixed_label(ui, "loaded", 14, L::Off);
            w::fixed_label(ui, "difference", 14, L::Off);
        });
        row(ui, "events", now.len() as f64, then.len() as f64, 0, "");
        row(ui, "nominal rate", a.nominal_hz, b.nominal_hz, 1, "Hz");
        row(ui, "sustained rate", a.effective_hz, b.effective_hz, 2, "Hz");
        row(
            ui,
            "dropped reports",
            a.drop_rate * 100.0,
            b.drop_rate * 100.0,
            4,
            "%",
        );
        row(
            ui,
            "motion recorded",
            now.iter().map(|r| r.counts.max(0) as f64).sum(),
            loaded.counts(level),
            0,
            "counts",
        );
        row(ui, "median interval", a.p50_ns / 1000.0, b.p50_ns / 1000.0, 1, "us");
        row(ui, "99th percentile", a.p99_ns / 1000.0, b.p99_ns / 1000.0, 1, "us");
        row(
            ui,
            "button transitions",
            app.session.buttons.len() as f64,
            loaded.buttons().len() as f64,
            0,
            "",
        );
        ui.add_space(4.0);
        w::note_indent(
            ui,
            0.0,
            "Both columns are computed by the same analysis from the same kind of raw \
             data, so the difference is a real comparison rather than two summaries \
             written at different times.",
        );
        if !comparable(app, &loaded) {
            ui.add_space(4.0);
            w::status_line(
                ui,
                L::Warn,
                "These two captures are not from the same device, or not from the same \
                 machine. The difference column is still arithmetic, but it is not \
                 measuring what you probably want it to.",
            );
        }
    });
}

fn comparable(app: &App, loaded: &SessionLog) -> bool {
    let now = app.log_meta();
    now.device_ids == loaded.meta.device_ids && now.os == loaded.meta.os
}

fn row(ui: &mut egui::Ui, label: &str, now: f64, then: f64, dp: usize, unit: &str) {
    let d = now - then;
    ui.horizontal(|ui| {
        w::fixed_label(ui, label, w::LABEL_CHARS, L::Info);
        w::fixed_value(ui, &w::num(now, dp), 14, L::Info);
        w::fixed_value(ui, &w::num(then, dp), 14, L::Info);
        // Signed, and never coloured: which direction is better depends on the
        // row, and colour here is reserved for pass and fail.
        w::fixed_value(ui, &w::signed(d, dp), 14, L::Info);
        if !unit.is_empty() {
            w::fixed_label(ui, unit, unit.chars().count().max(3), L::Off);
        }
    });
}

/// Every measurement's verdict, before and after, which is the question a
/// person changing a setting actually has.
///
/// The timing table below this one compares two recordings. This compares two
/// configurations, and it is the only place in the app where a change to the
/// mouse can be seen as a change to the mouse rather than as two sets of
/// numbers a person has to hold in their head.
fn results_side_by_side(app: &mut App, ui: &mut egui::Ui) {
    use crate::core::battery;
    let Some(loaded) = app.data.loaded.as_ref() else {
        return;
    };
    let before = loaded.meta.results.clone();
    let after = battery::snapshot(app);
    let rows = battery::compare(&before, &after);

    w::subheading(ui, "results, loaded against now");
    w::boxed(ui, |ui| {
        if rows.is_empty() {
            w::note_indent(
                ui,
                0.0,
                "Neither the loaded file nor this session carries any measurement results. \
                 An export written by a version of this app older than the battery has \
                 none, and there is nothing to compare.",
            );
            return;
        }
        ui.horizontal(|ui| {
            w::fixed_label(ui, "", 22, L::Off);
            w::fixed_label(ui, "loaded", 14, L::Off);
            w::fixed_label(ui, "now", 14, L::Off);
        });
        for row in &rows {
            let label = battery::label_for(&row.key).to_string();
            ui.horizontal(|ui| {
                w::fixed_label(ui, &label, 22, L::Info);
                for side in [&row.before, &row.after] {
                    match side {
                        Some(r) => {
                            let (txt, lvl) = match r.verdict {
                                Some(v) => (verdict_word(v), w::level_of(v)),
                                // A measurement with no verdict is not a blank:
                                // CPS and A/B report numbers, and showing them
                                // as absent would read as never run.
                                None => ("recorded", L::Info),
                            };
                            w::fixed_value(ui, txt, 14, lvl);
                        }
                        // Not "pass", not blank. A side that never measured this
                        // has said nothing, and silence in a comparison reads as
                        // agreement unless it is spelled out.
                        None => {
                            w::fixed_value(ui, "not measured", 14, L::Off);
                        }
                    }
                }
                if row.changed() {
                    w::fixed_value(ui, "CHANGED", 9, L::Warn);
                }
            });
            // The figures underneath, because a verdict that stayed the same can
            // still have moved a long way inside its band.
            let b = row.before.as_ref().map(|r| r.headline.as_str()).unwrap_or("-");
            let a = row.after.as_ref().map(|r| r.headline.as_str()).unwrap_or("-");
            if b != "-" || a != "-" {
                w::note_indent(ui, 4.0, &format!("loaded: {b}"));
                w::note_indent(ui, 4.0, &format!("now:    {a}"));
            }
            ui.add_space(2.0);
        }
        w::note_indent(
            ui,
            0.0,
            "A row marked CHANGED is one whose verdict moved. A verdict that did not move \
             can still have travelled inside its band, which is what the two figures under \
             each row are for. Rows where one side says \"not measured\" are not agreement: \
             one of the two configurations was never asked.",
        );
    });
}

fn verdict_word(v: crate::core::sensor::Verdict) -> &'static str {
    use crate::core::sensor::Verdict;
    match v {
        Verdict::Pass => "pass",
        Verdict::Warn => "warn",
        Verdict::Fail => "fail",
        Verdict::Inconclusive => "inconclusive",
    }
}
