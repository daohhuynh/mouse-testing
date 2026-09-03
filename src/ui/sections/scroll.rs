//! Scroll wheel: detent counts, reversed steps and skipped steps.

use crate::app::{App, SensorPhase};
use crate::core::sensor::scroll::ScrollResult;
use crate::ui::theme::Level;
use crate::ui::widgets as w;

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    w::heading(ui, "SCROLL WHEEL");

    match app.scroll.phase {
        SensorPhase::Idle => {
            setup(app, ui);
            ui.add_space(10.0);
            results(app, ui);
        }
        SensorPhase::Countdown => countdown(app, ui),
        SensorPhase::Recording => recording(app, ui),
    }
}

fn setup(app: &mut App, ui: &mut egui::Ui) {
    w::subheading(ui, "what to do");
    w::boxed(ui, |ui| {
        for (i, step) in [
            "Press start, wait for the countdown, then scroll at your normal reading pace.",
            "Scroll roughly the same number of detents up as down. The two directions are \
             counted separately, and a wheel can be fine one way and not the other.",
            "Do not flick or spin. A flick merges detents legitimately, and while that is \
             allowed for, an ordinary cadence gives a much clearer answer.",
            "Thirty detents in each direction is enough to see a gross fault. Detecting a \
             3 percent reversal rate reliably takes about 150 in total.",
            "If the wheel tilts sideways, tilt it too: the horizontal encoder is a separate \
             mechanism and gets its own result.",
        ]
        .iter()
        .enumerate()
        {
            w::note_indent(ui, 0.0, &format!("{}. {}", i + 1, step));
            ui.add_space(2.0);
        }
    });

    ui.add_space(8.0);
    if app.scroll.capture_lost {
        w::status_line(
            ui,
            Level::Fail,
            "The capture stopped while this run was recording, so the run is void and nothing \
             was measured. Press start to run it again.",
        );
        ui.add_space(6.0);
    }
    w::subheading(ui, "settings");
    w::boxed(ui, |ui| {
        ui.horizontal(|ui| {
            w::fixed_label(ui, "capture for", 14, Level::Off);
            for secs in [10.0f64, 20.0, 40.0, 60.0] {
                let sel = (app.scroll.capture_s - secs).abs() < 0.01;
                if w::chip(ui, sel, &format!("{secs:.0} s")).clicked() {
                    app.scroll.capture_s = secs;
                }
            }
        });
        ui.horizontal(|ui| {
            w::fixed_label(ui, "countdown", 14, Level::Off);
            for secs in [0.0f64, 3.0, 5.0, 10.0] {
                let sel = (app.scroll.countdown_s - secs).abs() < 0.01;
                if w::chip(ui, sel, &format!("{secs:.0} s")).clicked() {
                    app.scroll.countdown_s = secs;
                }
            }
        });
        w::note_indent(
            ui,
            0.0,
            "Time to put this machine's pointer down and pick up the mouse being measured.",
        );
    });

    ui.add_space(8.0);
    ui.horizontal(|ui| {
        if ui.button("start").clicked() {
            app.scroll_start();
        }
        ui.add_space(8.0);
        if app.scroll.vertical.is_some() && ui.button("clear").clicked() {
            app.scroll_clear();
        }
    });
}

fn countdown(app: &mut App, ui: &mut egui::Ui) {
    let left = (app.scroll.countdown_s - app.scroll.elapsed_s()).max(0.0);
    w::subheading(ui, "starting");
    w::boxed(ui, |ui| {
        w::readout(ui, "begins in", &format!("{left:.1}"), 8, "s", Level::Warn);
    });
    ui.add_space(8.0);
    if ui.button("cancel").clicked() {
        app.scroll_abandon();
    }
}

fn recording(app: &mut App, ui: &mut egui::Ui) {
    let left = (app.scroll.capture_s - app.scroll.elapsed_s()).max(0.0);
    w::subheading(ui, "recording");
    w::boxed(ui, |ui| {
        w::readout(ui, "time left", &format!("{left:.1}"), 8, "s", Level::Warn);
        w::note_indent(ui, 0.0, "Scroll up, then scroll down.");
    });
    ui.add_space(8.0);
    if ui.button("abandon").clicked() {
        app.scroll_abandon();
    }
}

fn results(app: &mut App, ui: &mut egui::Ui) {
    let Some(v) = app.scroll.vertical.clone() else {
        w::subheading(ui, "result");
        w::note_indent(ui, 0.0, "Nothing captured yet.");
        return;
    };
    axis_result(ui, &v);
    if let Some(h) = app.scroll.horizontal.clone() {
        ui.add_space(10.0);
        axis_result(ui, &h);
    } else {
        ui.add_space(8.0);
        w::note_indent(
            ui,
            0.0,
            "No horizontal wheel motion was seen, so no result is shown for one. That is \
             not a finding either way: most mice have no tilt wheel, and one that does was \
             perhaps simply not tilted.",
        );
    }
    ui.add_space(8.0);
    explanation(ui);
}

fn axis_result(ui: &mut egui::Ui, r: &ScrollResult) {
    w::subheading(ui, r.axis.title());
    w::boxed(ui, |ui| {
        w::status_line(ui, r.verdict.level(), r.note);
        ui.add_space(4.0);

        if r.continuous {
            w::kv_level(
                ui,
                "counts per detent",
                "none: this wheel has no detents to count",
                Level::Off,
            );
        } else {
            w::readout(
                ui,
                "counts per detent",
                &w::num(r.quantum, 0),
                10,
                "counts",
                Level::Info,
            );
            w::readout(
                ui,
                "steps this explains",
                &w::num(r.quantum_coverage * 100.0, 1),
                10,
                "%",
                if r.quantum_coverage >= 0.9 {
                    Level::Pass
                } else {
                    Level::Warn
                },
            );
        }

        w::readout(
            ui,
            "detents up",
            &format!("{}", r.detents_up),
            10,
            "",
            Level::Info,
        );
        w::readout(
            ui,
            "detents down",
            &format!("{}", r.detents_down),
            10,
            "",
            Level::Info,
        );
        w::readout(
            ui,
            "reversed steps",
            &format!("{}", r.reversals),
            10,
            "",
            if r.reversals > 0 { Level::Fail } else { Level::Pass },
        );
        w::readout(
            ui,
            "skipped steps",
            &format!("{}", r.skips),
            10,
            "",
            if r.skips > 0 { Level::Warn } else { Level::Pass },
        );
        if r.reversal_rate.is_finite() {
            w::readout(
                ui,
                "reversal rate",
                &w::num(r.reversal_rate * 100.0, 2),
                10,
                "%",
                Level::Info,
            );
            w::readout(
                ui,
                "skip rate",
                &w::num(r.skip_rate * 100.0, 2),
                10,
                "%",
                Level::Info,
            );
        }
        w::readout(
            ui,
            "steps recorded",
            &format!("{}", r.n_clusters),
            10,
            "",
            Level::Info,
        );
        w::readout(
            ui,
            "typical gap between",
            &w::num(r.median_gap_ms, 0),
            10,
            "ms",
            Level::Info,
        );
        w::readout(
            ui,
            "grouping threshold",
            &w::num(r.cluster_gap_ms, 1),
            10,
            "ms",
            Level::Info,
        );
    });
}

fn explanation(ui: &mut egui::Ui) {
    w::subheading(ui, "how this works");
    w::boxed(ui, |ui| {
        w::note_indent(
            ui,
            0.0,
            "What one detent is worth on the wire is not fixed. A classic wheel sends one \
             count; Windows sends 120; a high-resolution wheel sends a fraction of 120, or \
             eight or fifteen sub-counts spread across several reports inside a single \
             click. Assuming any one of those would give the wrong detent count on most \
             hardware, so the value is worked out from the capture instead: reports are \
             grouped by time, and the step size that explains the most group totals wins.",
        );
        ui.add_space(2.0);
        w::note_indent(
            ui,
            0.0,
            "The grouping threshold is also taken from the data rather than fixed. A wheel \
             that spreads fifteen sub-reports a millisecond apart spans fifteen \
             milliseconds, which a fixed twelve-millisecond threshold would cut in half and \
             report as a skip that never happened.",
        );
        ui.add_space(2.0);
        w::note_indent(
            ui,
            0.0,
            "A reversed step is one step against the direction of both its neighbours: \
             encoder chatter, not a change of mind, because deliberately changing direction \
             leaves a run of steps the other way rather than a single one.",
        );
        ui.add_space(2.0);
        w::note_indent(
            ui,
            0.0,
            "A skipped step is a double step that arrived at an ordinary cadence. Scrolling \
             fast genuinely merges detents, so a double that arrives early is your hand and \
             is not counted against the wheel.",
        );
    });
}
