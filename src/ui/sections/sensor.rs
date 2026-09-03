//! Sensor behaviour: what the optics and the firmware do to your motion.

use crate::app::{App, SensorPhase};
use crate::core::sensor::protocol::Test;
use crate::core::sensor::Verdict;
use crate::ui::theme::Level;
use crate::ui::widgets as w;

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    w::heading(ui, "SENSOR BEHAVIOUR");

    if let Some(why) = unusable(app) {
        blocked(app, ui, &why);
        return;
    }

    picker(app, ui);
    ui.add_space(8.0);

    match app.sensor.phase {
        SensorPhase::Idle => {
            setup(app, ui);
            ui.add_space(10.0);
            result(app, ui);
        }
        SensorPhase::Countdown => countdown(app, ui),
        SensorPhase::Recording => recording(app, ui),
    }
}

/// Why this whole section cannot run, if it cannot.
///
/// Every test here needs signed per-axis counts, and there are two separate
/// ways not to have them: no access to the device level at all, or access to it
/// but no field map to decode its reports with. Both would otherwise present as
/// a mouse that never moved, which is the one thing this program must not do.
fn unusable(app: &App) -> Option<String> {
    use crate::capture::LevelState;
    if app.session.device_state == LevelState::Blocked {
        return Some(
            "the device level is not delivering the raw per-axis counts the mouse puts on \
             the wire"
                .into(),
        );
    }
    let s = &app.session;
    if s.decoded == 0 && s.undecoded > 0 {
        return Some(format!(
            "the device level is delivering reports but none of them could be decoded: \
             {} report(s) arrived with no usable field map, so their motion is unknown \
             rather than zero",
            s.undecoded
        ));
    }
    None
}

fn blocked(app: &mut App, ui: &mut egui::Ui, why: &str) {
    w::status_line(
        ui,
        Level::Fail,
        &format!(
            "Every test in this section needs signed per-axis motion, and {why}. None of \
             these measurements can be made from what the operating system passes on \
             instead, so nothing here will show you a number in place of one."
        ),
    );
    if !app.session.device_note.is_empty() {
        ui.add_space(4.0);
        w::note_indent(ui, 14.0, &app.session.device_note);
    }
    ui.add_space(6.0);

    // A removal is the one cause of this panel that a button can fix, and this
    // panel has replaced every control that would otherwise offer one.
    if app.session.device_removed {
        ui.horizontal(|ui| {
            if ui.button("start a new capture").clicked() {
                app.restart_capture();
            }
            w::note_indent(
                ui,
                6.0,
                "Opens the device again. Nothing measured before it went away is kept.",
            );
        });
        ui.add_space(6.0);
    }

    w::note_indent(
        ui,
        0.0,
        "The DEVICE section lists what access was obtained and exactly how to grant what \
         is missing.",
    );
}

fn picker(app: &mut App, ui: &mut egui::Ui) {
    w::subheading(ui, "measurement");
    w::boxed(ui, |ui| {
        for t in Test::ALL {
            let sel = app.sensor.test == t;
            let mark = verdict_of(app, t)
                .map(|v| format!("[{}]", v.level().tag()))
                .unwrap_or_else(|| "[    ]".into());
            let label = format!("{mark}  {:<38} {}", t.title(), t.purpose());
            if w::nav_row(ui, sel, &label).clicked() {
                app.sensor.test = t;
            }
        }
    });
}

fn verdict_of(app: &App, t: Test) -> Option<Verdict> {
    match t {
        Test::Cpi => app.sensor.cpi_summary.as_ref().map(|s| s.verdict),
        Test::Drift => app.sensor.drift.as_ref().map(|r| r.verdict),
        Test::Snap => app.sensor.snap.as_ref().map(|r| r.verdict),
        Test::Smooth => app.sensor.smooth.as_ref().map(|r| r.verdict),
        Test::Tracking => app.sensor.tracking.as_ref().map(|r| r.verdict),
        // The ladder's verdict, not the last run's: one run answers only
        // "tracked at this height", which is not a judgement on the mouse.
        Test::Lod => app.sensor.lod_summary.as_ref().map(|s| s.verdict),
    }
}

fn setup(app: &mut App, ui: &mut egui::Ui) {
    let test = app.sensor.test;

    w::subheading(ui, "what to do");
    w::boxed(ui, |ui| {
        for (i, step) in test.steps().iter().enumerate() {
            w::note_indent(ui, 0.0, &format!("{}. {}", i + 1, step));
            if i + 1 < test.steps().len() {
                ui.add_space(2.0);
            }
        }
    });

    ui.add_space(8.0);
    if app.sensor.capture_lost {
        w::status_line(
            ui,
            Level::Fail,
            "This run is void: the capture was not running for the whole of it, so nothing was \
             measured and no result here describes your mouse. Press start to run it again.",
        );
        if !app.session.device_note.is_empty() {
            w::note_indent(ui, 7.0, &app.session.device_note);
        }
        ui.add_space(6.0);
    }
    w::subheading(ui, "settings");
    w::boxed(ui, |ui| {
        ui.horizontal(|ui| {
            w::fixed_label(ui, "configured CPI", 18, Level::Off);
            ui.add(
                egui::TextEdit::singleline(&mut app.sensor.claimed_cpi)
                    .desired_width(90.0)
                    .hint_text("1600"),
            );
            ui.add_space(6.0);
            w::fixed_label(
                ui,
                "what the mouse is set to, not what the box says",
                50,
                Level::Off,
            );
        });
        if test == Test::Cpi {
            ui.horizontal(|ui| {
                w::fixed_label(ui, "distance swiped", 18, Level::Off);
                ui.add(
                    egui::TextEdit::singleline(&mut app.sensor.distance)
                        .desired_width(90.0)
                        .hint_text(if app.sensor.distance_mm { "200" } else { "8" }),
                );
                ui.add_space(6.0);
                let unit = if app.sensor.distance_mm { "mm" } else { "inches" };
                if ui.button(unit).clicked() {
                    app.sensor.distance_mm = !app.sensor.distance_mm;
                }
                ui.add_space(6.0);
                w::fixed_label(ui, "click to switch units", 24, Level::Off);
            });
        }
        if test == Test::Lod {
            ui.horizontal(|ui| {
                w::fixed_label(ui, "twenty cards", 18, Level::Off);
                ui.add(
                    egui::TextEdit::singleline(&mut app.sensor.shim_ref_mm)
                        .desired_width(90.0)
                        .hint_text("6.0"),
                );
                ui.add_space(6.0);
                w::fixed_label(ui, "mm, measured once with a ruler", 50, Level::Off);
            });
            ui.horizontal(|ui| {
                w::fixed_label(ui, "cards per pile", 18, Level::Off);
                ui.add(
                    egui::TextEdit::singleline(&mut app.sensor.shims_in_stack)
                        .desired_width(90.0)
                        .hint_text("0"),
                );
                ui.add_space(6.0);
                let h = app
                    .sensor
                    .lod_height_mm()
                    .map(|v| format!("= {v:.2} mm; 0 is the control run"))
                    .unwrap_or_else(|| "0 is the control run".into());
                w::fixed_label(ui, &h, 50, Level::Off);
            });
            ui.horizontal(|ui| {
                w::fixed_label(ui, "slot width", 18, Level::Off);
                ui.add(
                    egui::TextEdit::singleline(&mut app.sensor.slot_mm)
                        .desired_width(90.0)
                        .hint_text("6"),
                );
                ui.add_space(6.0);
                w::fixed_label(ui, "mm, the gap between the two piles", 50, Level::Off);
            });
            ui.horizontal(|ui| {
                w::fixed_label(ui, "configured LOD", 18, Level::Off);
                ui.add(
                    egui::TextEdit::singleline(&mut app.sensor.claimed_lod_mm)
                        .desired_width(90.0)
                        .hint_text("1.0"),
                );
                ui.add_space(6.0);
                w::fixed_label(ui, "mm, optional: what the mouse is set to", 50, Level::Off);
            });
        }
        ui.horizontal(|ui| {
            w::fixed_label(ui, "countdown", 18, Level::Off);
            for secs in [0.0f64, 3.0, 5.0, 10.0] {
                let sel = (app.sensor.countdown_s - secs).abs() < 0.01;
                if w::chip(ui, sel, &format!("{secs:.0} s")).clicked() {
                    app.sensor.countdown_s = secs;
                }
            }
        });
        w::note_indent(
            ui,
            0.0,
            "Time to put this machine's pointer down and pick up the mouse being measured. \
             Every test here needs your hand on the mouse under test, which cannot also be \
             the thing that pressed start.",
        );
    });

    ui.add_space(8.0);
    let ready = match test {
        Test::Cpi => app.sensor.claimed_cpi_value().is_some() && app.sensor.distance_in().is_some(),
        // The height and the slot are both structural: without the slot width
        // there is nothing to check a silence against, and without the pile
        // count the run cannot be placed on the ladder.
        Test::Lod => {
            app.sensor.claimed_cpi_value().is_some()
                && app.sensor.lod_height_mm().is_some()
                && app.sensor.slot_mm_value().is_some()
        }
        _ => true,
    };
    ui.horizontal(|ui| {
        if ui
            .add_enabled(ready, egui::Button::new("start"))
            .clicked()
        {
            app.sensor_start();
        }
        ui.add_space(8.0);
        if verdict_of(app, test).is_some() && ui.button("clear this measurement").clicked() {
            app.sensor_clear();
        }
        ui.add_space(8.0);
        w::fixed_label(
            ui,
            &format!("captures for {:.0} s", test.capture_s()),
            22,
            Level::Off,
        );
    });
    if !ready {
        ui.add_space(4.0);
        w::status_line(
            ui,
            Level::Warn,
            if test == Test::Lod {
                "Enter the configured CPI, the twenty-card thickness, how many cards are in \
                 each pile, and the slot width. Enter 0 cards for the control run."
            } else {
                "Enter the configured CPI and the distance you will swipe. Without both \
                 there is nothing to compare a count against."
            },
        );
    }
}

fn countdown(app: &mut App, ui: &mut egui::Ui) {
    let left = (app.sensor.countdown_s - app.sensor.elapsed_s()).max(0.0);
    w::subheading(ui, "starting");
    w::boxed(ui, |ui| {
        w::readout(ui, "begins in", &format!("{left:.1}"), 8, "s", Level::Warn);
        w::note_indent(ui, 0.0, app.sensor.test.steps()[0]);
    });
    ui.add_space(8.0);
    if ui.button("cancel").clicked() {
        app.sensor_abandon();
    }
}

fn recording(app: &mut App, ui: &mut egui::Ui) {
    let total = app.sensor.test.capture_s();
    let left = (total - app.sensor.elapsed_s()).max(0.0);
    w::subheading(ui, "recording");
    w::boxed(ui, |ui| {
        w::readout(ui, "time left", &format!("{left:.1}"), 8, "s", Level::Warn);
        let seen = app.sensor_run_reports();
        w::readout(
            ui,
            "reports so far",
            &format!("{seen}"),
            8,
            "",
            if seen > 0 { Level::Info } else { Level::Warn },
        );
        w::readout(ui, "from", &app.recording_device_name(), 8, "", Level::Info);
        ui.add_space(2.0);
        // Nothing arriving well into a run is almost always the wrong device
        // rather than a broken one, and the section gave no way to tell.
        if seen == 0 && left < app.sensor.test.capture_s() * 0.5 {
            w::status_line(
                ui,
                Level::Warn,
                "Nothing has arrived from that device. If the mouse you are moving is a \
                 different one, pick it in DEVICE and start this run again.",
            );
            ui.add_space(2.0);
        }
        w::note_indent(ui, 0.0, "Nothing is analysed until the capture finishes.");
    });
    ui.add_space(8.0);
    if ui.button("abandon").clicked() {
        app.sensor_abandon();
    }
}

fn result(app: &mut App, ui: &mut egui::Ui) {
    match app.sensor.test {
        Test::Cpi => cpi_result(app, ui),
        Test::Drift => drift_result(app, ui),
        Test::Snap => snap_result(app, ui),
        Test::Smooth => smooth_result(app, ui),
        Test::Tracking => tracking_result(app, ui),
        Test::Lod => lod_result(app, ui),
    }
}

/// Shown when a capture finished but produced nothing worth analysing.
fn empty_note(app: &App, ui: &mut egui::Ui) {
    if app.sensor.last_reports == 0 {
        return;
    }
    w::note_indent(
        ui,
        0.0,
        &format!(
            "The last capture collected {} motion report(s).",
            app.sensor.last_reports
        ),
    );
}

fn cpi_result(app: &mut App, ui: &mut egui::Ui) {
    let Some(sum) = app.sensor.cpi_summary.clone() else {
        w::subheading(ui, "result");
        w::note_indent(ui, 0.0, "No swipe recorded yet.");
        return;
    };
    let claimed = app.sensor.claimed_cpi_value().unwrap_or(0.0);

    w::subheading(ui, "result");
    w::boxed(ui, |ui| {
        if sum.n_trials == 0 {
            w::status_line(
                ui,
                Level::Off,
                "No swipe in this run could be used. Each attempt below says why.",
            );
        } else {
            w::status_line(
                ui,
                sum.verdict.level(),
                &format!(
                    "Measured {:.0} CPI against {:.0} claimed, a difference of {:+.2}%.",
                    sum.median_cpi,
                    claimed,
                    sum.deviation * 100.0
                ),
            );
            ui.add_space(4.0);
            w::readout(
                ui,
                "measured",
                &w::num(sum.median_cpi, 1),
                10,
                "CPI",
                sum.verdict.level(),
            );
            w::readout(ui, "claimed", &format!("{claimed:.1}"), 10, "CPI", Level::Info);
            w::readout(
                ui,
                "difference",
                &w::signed(sum.deviation * 100.0, 2),
                10,
                "%",
                Level::Info,
            );
            if sum.se_cpi.is_finite() {
                w::readout(
                    ui,
                    "spread across swipes",
                    &w::num(sum.se_cpi, 1),
                    10,
                    "CPI",
                    Level::Info,
                );
            }
            w::readout(
                ui,
                "usable swipes",
                &format!("{}", sum.n_trials),
                10,
                "",
                Level::Info,
            );
            ui.add_space(2.0);
            w::note_indent(
                ui,
                0.0,
                "The median across swipes, not the mean, so one bad swipe cannot move it. \
                 Sensor CPI on commodity optical sensors is routinely 1 to 3 percent off \
                 nominal and is not tightly specified, so a small difference is part \
                 variation rather than a fault.",
            );
            if sum.n_trials < 3 {
                ui.add_space(2.0);
                w::status_line(
                    ui,
                    Level::Warn,
                    "Fewer than three swipes. The spread between repeats is the only honest \
                     measure of how repeatable this is, and it needs repeats.",
                );
            }
        }
    });

    ui.add_space(8.0);
    w::subheading(ui, "each swipe");
    w::boxed(ui, |ui| {
        ui.horizontal(|ui| {
            w::fixed_label(ui, "#", 4, Level::Off);
            w::fixed_label(ui, "measured", 12, Level::Off);
            w::fixed_label(ui, "difference", 12, Level::Off);
            w::fixed_label(ui, "counts", 10, Level::Off);
            w::fixed_label(ui, "wobble", 9, Level::Off);
            w::fixed_label(ui, "peak", 10, Level::Off);
        });
        for (i, t) in app.sensor.cpi_trials.iter().enumerate() {
            ui.horizontal(|ui| {
                w::fixed_label(ui, &format!("{}", i + 1), 4, Level::Off);
                w::fixed_value(
                    ui,
                    &w::num(t.measured_cpi, 0),
                    12,
                    t.verdict.level(),
                );
                w::fixed_value(
                    ui,
                    &if t.deviation.is_finite() {
                        format!("{:+.2}%", t.deviation * 100.0)
                    } else {
                        "-".to_string()
                    },
                    12,
                    Level::Info,
                );
                w::fixed_value(ui, &w::num(t.l_net, 0), 10, Level::Info);
                w::fixed_value(
                    ui,
                    &w::num(t.wobble, 3),
                    9,
                    Level::Info,
                );
                w::fixed_value(ui, &format!("{} IPS", w::num(t.peak_ips, 0)), 10, Level::Info);
            });
            if !t.note.is_empty() {
                w::note_indent(ui, 14.0, t.note);
            }
        }
        ui.add_space(2.0);
        w::note_indent(
            ui,
            0.0,
            "Counts is the straight-line distance the mouse reported, which is what your \
             ruler measured. Wobble is the path length divided by that, so 1.000 is a \
             perfectly straight swipe and anything above 1.15 is thrown out.",
        );
    });
    ui.add_space(6.0);
    empty_note(app, ui);
}

fn drift_result(app: &mut App, ui: &mut egui::Ui) {
    let Some(r) = app.sensor.drift.clone() else {
        w::subheading(ui, "result");
        w::note_indent(ui, 0.0, "Not measured yet.");
        return;
    };
    w::subheading(ui, "result");
    w::boxed(ui, |ui| {
        w::status_line(ui, r.verdict.level(), r.note);
        ui.add_space(4.0);
        w::readout(
            ui,
            "drift",
            &w::num(r.drift_cps, 2),
            10,
            "counts/s",
            if r.drift_detected { Level::Fail } else { Level::Pass },
        );
        w::readout(
            ui,
            "drift",
            &w::num(r.drift_ips, 4),
            10,
            "inches/s",
            Level::Info,
        );
        w::readout(
            ui,
            "jitter",
            &w::num(r.jitter_cps, 2),
            10,
            "counts/s",
            if r.jitter_detected {
                Level::Warn
            } else {
                Level::Pass
            },
        );
        w::readout(
            ui,
            "reports while still",
            &format!("{}", r.n_moving_reports),
            10,
            "",
            Level::Info,
        );
        w::readout(
            ui,
            "capture length",
            &w::num(r.duration_s, 1),
            10,
            "s",
            Level::Info,
        );
        ui.add_space(2.0);
        w::note_indent(
            ui,
            0.0,
            "Drift is motion that goes somewhere: the pointer walks off. Jitter is motion \
             that goes nowhere: the pointer shimmers in place. They are different faults \
             and a mouse can have either without the other, which is why they are two \
             numbers rather than one.",
        );
    });

    ui.add_space(8.0);
    w::subheading(ui, "per axis");
    w::boxed(ui, |ui| {
        ui.horizontal(|ui| {
            w::fixed_label(ui, "axis", 8, Level::Off);
            w::fixed_label(ui, "net", 10, Level::Off);
            w::fixed_label(ui, "total", 10, Level::Off);
            w::fixed_label(ui, "z", 10, Level::Off);
            w::fixed_label(ui, "sign z", 10, Level::Off);
        });
        for (name, a) in [("x", &r.x), ("y", &r.y)] {
            ui.horizontal(|ui| {
                w::fixed_label(ui, name, 8, Level::Off);
                w::fixed_value(ui, &w::signed(a.net, 0), 10, Level::Info);
                w::fixed_value(ui, &w::num(a.abs_sum, 0), 10, Level::Info);
                w::fixed_value(
                    ui,
                    &w::signed(a.z_mean, 2),
                    10,
                    if a.z_mean.abs() > 3.0 {
                        Level::Fail
                    } else {
                        Level::Info
                    },
                );
                w::fixed_value(ui, &w::signed(a.z_sign, 2), 10, Level::Info);
            });
        }
        ui.add_space(2.0);
        w::note_indent(
            ui,
            0.0,
            "Net is where the pointer ended up; total is how far it travelled to get there. \
             A mouse with 4000 counts of total motion and a net of 12 is shimmering, not \
             drifting. The z figure is the net measured against the size of the noise, so \
             it does not need a threshold in counts: past 3 the bias is real. The sign \
             column ignores magnitudes entirely and is there so one knock against the desk \
             cannot be read as drift.",
        );
    });
    ui.add_space(6.0);
    empty_note(app, ui);
}

fn snap_result(app: &mut App, ui: &mut egui::Ui) {
    let Some(r) = app.sensor.snap.clone() else {
        w::subheading(ui, "result");
        w::note_indent(ui, 0.0, "Not measured yet.");
        return;
    };
    w::subheading(ui, "result");
    w::boxed(ui, |ui| {
        w::status_line(ui, r.verdict.level(), r.note);
        ui.add_space(4.0);
        if r.aniso_applicable {
            w::readout(
                ui,
                "across/along noise",
                &w::num(r.hf_aniso, 3),
                10,
                "",
                if r.hf_aniso < 0.55 {
                    Level::Fail
                } else if r.hf_aniso < 0.75 {
                    Level::Warn
                } else {
                    Level::Pass
                },
            );
        } else {
            w::kv_level(
                ui,
                "across/along noise",
                "not applicable: this sensor is too quiet for the test to mean anything",
                Level::Off,
            );
        }
        w::readout(
            ui,
            "sensor noise along stroke",
            &w::num(r.hf_along_rms, 2),
            10,
            "counts",
            Level::Info,
        );
        w::readout(
            ui,
            "straightness",
            &w::num(r.straightness, 5),
            10,
            "",
            Level::Info,
        );
        w::readout(
            ui,
            "axis lock beyond normal",
            &w::signed(r.axis_lock_excess, 3),
            10,
            "",
            Level::Info,
        );
        w::readout(
            ui,
            "angle on 45 degrees",
            &w::num(r.angle_on_octant_frac, 3),
            10,
            "",
            Level::Info,
        );
        w::readout(ui, "travel", &w::num(r.travel_in, 2), 10, "inches", Level::Info);
        w::readout(
            ui,
            "speed",
            &w::num(r.median_ips, 1),
            10,
            "inches/s",
            Level::Info,
        );
    });

    ui.add_space(8.0);
    w::subheading(ui, "how this works");
    w::boxed(ui, |ui| {
        w::note_indent(
            ui,
            0.0,
            "The sensor's own noise does not know which way your hand is going, so it is \
             the same size along the stroke as across it. Angle snapping projects every \
             increment onto one direction, which deletes the across-stroke half and leaves \
             the along-stroke half alone. The ratio of the two is the measurement, and \
             because the mouse supplies both halves it needs no reference value.",
        );
        ui.add_space(2.0);
        w::note_indent(
            ui,
            0.0,
            "That ratio is near 1.0 on an honest mouse rather than exactly 1.0: it moves \
             between about 0.97 and 1.54 with sensor noise and poll rate. All of that error \
             is in the safe direction. The case it cannot handle is a snapping mouse whose \
             sensor is too quiet to have any noise to delete, and that is what the \
             not-applicable result above exists to catch instead of passing you.",
        );
        ui.add_space(2.0);
        w::note_indent(
            ui,
            0.0,
            "Straightness is how far your line wandered relative to its length. It is the \
             only figure here judged against how straight a human hand can draw rather than \
             against the mouse's own data, so it can only ever raise a warning on its own.",
        );
    });
    ui.add_space(6.0);
    empty_note(app, ui);
}

fn smooth_result(app: &mut App, ui: &mut egui::Ui) {
    let Some(r) = app.sensor.smooth.clone() else {
        w::subheading(ui, "result");
        w::note_indent(ui, 0.0, "Not measured yet.");
        return;
    };
    w::subheading(ui, "result");
    w::boxed(ui, |ui| {
        w::status_line(ui, r.verdict.level(), r.note);
        ui.add_space(4.0);
        w::readout(
            ui,
            "motion after the stop",
            &w::num(r.tail_ms, 2),
            10,
            "ms",
            if r.tail_ms >= 8.0 {
                Level::Fail
            } else if r.tail_ms >= 4.0 {
                Level::Warn
            } else {
                Level::Pass
            },
        );
        w::readout(
            ui,
            "reports after the stop",
            &format!("{}", r.tail_reports),
            10,
            "",
            Level::Info,
        );
        w::readout(
            ui,
            "high-frequency correlation",
            &w::signed(r.rho1_corrected, 3),
            10,
            "",
            if r.rho1_corrected > 0.35 {
                Level::Fail
            } else if r.rho1_corrected > 0.15 {
                Level::Warn
            } else {
                Level::Pass
            },
        );
        if r.decay_tau_ms.is_finite() {
            w::readout(
                ui,
                "filter time constant",
                &w::num(r.decay_tau_ms, 2),
                10,
                "ms",
                Level::Warn,
            );
            w::readout(
                ui,
                "fit quality",
                &w::num(r.decay_r2, 3),
                10,
                "",
                Level::Info,
            );
        }
        w::readout(
            ui,
            "report rate in stroke",
            &w::num(r.report_rate_hz, 0),
            10,
            "Hz",
            Level::Info,
        );
        w::readout(
            ui,
            "speed",
            &w::num(r.median_counts_per_report, 1),
            10,
            "counts/report",
            Level::Info,
        );
        if r.n_uniform > 0 && r.n_uniform < r.n_reports {
            w::readout(
                ui,
                "evenly spaced reports",
                &format!("{} of {}", r.n_uniform, r.n_reports),
                14,
                "",
                Level::Info,
            );
        }
    });

    ui.add_space(8.0);
    w::subheading(ui, "how this works");
    w::boxed(ui, |ui| {
        w::note_indent(
            ui,
            0.0,
            "Two separate signatures, because either can appear without the other. After a \
             hard stop the true motion is zero, so anything still arriving is stored filter \
             energy; an honest mouse holds less than one count and can emit at most a \
             report or two. Separately, a filter leaves its fingerprint on the \
             high-frequency part of the stroke, where a hand cannot put structure: \
             voluntary movement stops around 5 Hz and tremor at 8 to 12, so correlated \
             content at hundreds of Hz is firmware.",
        );
        ui.add_space(2.0);
        w::note_indent(
            ui,
            0.0,
            "The correlation is measured only over reports that are evenly spaced. Both the \
             filter and the correlation index the report list, which quietly assumes every \
             neighbouring pair is one period apart, and a gap in the middle of a stroke \
             breaks that badly enough to fail a mouse with nothing wrong with it.",
        );
    });
    ui.add_space(6.0);
    empty_note(app, ui);
}

fn tracking_result(app: &mut App, ui: &mut egui::Ui) {
    let Some(r) = app.sensor.tracking.clone() else {
        w::subheading(ui, "result");
        w::note_indent(ui, 0.0, "Not measured yet.");
        return;
    };
    w::subheading(ui, "result");
    w::boxed(ui, |ui| {
        w::status_line(ui, r.verdict.level(), r.note);
        ui.add_space(4.0);
        if r.bounded_below {
            w::readout(
                ui,
                "tracked at least",
                &w::num(r.max_tracking_ips, 0),
                10,
                "inches/s",
                Level::Pass,
            );
            w::note_indent(
                ui,
                14.0,
                "A lower bound, not a limit. Nothing broke, so the fastest you managed is \
                 all that has been shown.",
            );
        } else {
            w::readout(
                ui,
                "tracking failed above",
                &w::num(r.max_tracking_ips, 0),
                10,
                "inches/s",
                Level::Fail,
            );
        }
        w::readout(
            ui,
            "fastest reached",
            &w::num(r.peak_observed_ips, 0),
            10,
            "inches/s",
            Level::Info,
        );
        if r.first_failure_ips.is_finite() {
            w::readout(
                ui,
                "first failure at",
                &w::num(r.first_failure_ips, 0),
                10,
                "inches/s",
                Level::Warn,
            );
            w::kv(ui, "how it failed", r.first_failure_reason);
        }
        w::readout(
            ui,
            "windows judged",
            &format!("{} of {}", r.n_failed_windows, r.n_windows),
            14,
            "failed",
            Level::Info,
        );
    });

    ui.add_space(8.0);
    w::subheading(ui, "report field");
    w::boxed(ui, |ui| {
        match r.field.matched_bound {
            Some(b) => {
                w::readout(ui, "counts per report limit", &format!("{b}"), 10, "counts", Level::Info);
                w::kv_level(
                    ui,
                    "reaching that limit",
                    if r.field.saturating {
                        "yes, so the ceiling above is the report format, not the sensor"
                    } else {
                        "no, so the ceiling above is not this"
                    },
                    if r.field.saturating {
                        Level::Warn
                    } else {
                        Level::Pass
                    },
                );
            }
            None => w::kv(
                ui,
                "counts per report limit",
                "no fixed limit was reached",
            ),
        }
        w::readout(
            ui,
            "largest single report",
            &format!("{}", r.field.observed_max),
            10,
            "counts",
            Level::Info,
        );
        ui.add_space(2.0);
        w::note_indent(
            ui,
            0.0,
            "A mouse that packs motion into an 8-bit field cannot report more than 127 \
             counts at once, which at 1600 CPI and 1000 Hz is a hard ceiling of 79 inches \
             per second that has nothing to do with the optics. Blaming the sensor for that \
             would point you at the wrong component, so it is measured separately.",
        );
    });
    ui.add_space(6.0);
    empty_note(app, ui);
}

fn lod_result(app: &mut App, ui: &mut egui::Ui) {
    let Some(r) = app.sensor.lod_last.clone() else {
        w::subheading(ui, "result");
        w::note_indent(ui, 0.0, "No run recorded yet.");
        return;
    };
    w::subheading(ui, "last run");
    w::boxed(ui, |ui| {
        let (level, text) = match r.state {
            crate::core::sensor::lod::LodState::Tracked => (Level::Pass, "TRACKED at this height"),
            crate::core::sensor::lod::LodState::Lost => (Level::Fail, "LOST the pad at this height"),
            crate::core::sensor::lod::LodState::Marginal => (Level::Warn, "MARGINAL at this height"),
            crate::core::sensor::lod::LodState::Unknown => (Level::Off, "no answer from this run"),
        };
        w::status_line(ui, level, text);
        w::note_indent(ui, 7.0, r.note);
        ui.add_space(4.0);
        w::readout(ui, "height", &format!("{:.2}", r.height_mm), 11, "mm", Level::Info);
        w::readout(ui, "reports", &format!("{}", r.n_reports), 11, "", Level::Info);
        w::readout(ui, "sweeps", &format!("{}", r.n_half_strokes), 11, "", Level::Info);
        w::readout(ui, "turns", &format!("{}", r.n_turnarounds), 11, "", Level::Info);
        w::readout(ui, "silences", &format!("{}", r.n_silences), 11, "", Level::Info);
        w::readout(ui, "crossings", &format!("{}", r.n_crossings), 11, "", Level::Info);
        w::readout(ui, "of these, stops", &format!("{}", r.n_pauses), 11, "", Level::Info);
        w::readout(
            ui,
            "passes lost",
            &format!("{:.0}", r.loss_fraction * 100.0),
            11,
            "%",
            Level::Info,
        );
        if r.n_crossings > 0 {
            w::readout(ui, "slot found at", &format!("{:.1}", r.slot_at_mm), 11, "mm", Level::Info);
            w::readout(ui, "wander", &format!("{:.1}", r.slot_spread_mm), 11, "mm", Level::Info);
            w::readout(
                ui,
                "silence width",
                &format!("{:.1}", r.silence_width_mm),
                11,
                "mm",
                Level::Info,
            );
        }
        w::note_indent(
            ui,
            0.0,
            "A crossing is a silence entered and left at full speed, in the same direction, \
             in the same place every sweep. A stop is a silence with a hand slowing down on \
             one side of it. The turns at the ends of your sweep are what the difference is \
             measured against, so they are not wasted time.",
        );
    });

    ui.add_space(8.0);
    w::subheading(ui, "the ladder");
    w::boxed(ui, |ui| {
        if app.sensor.lod_rungs.is_empty() {
            w::note_indent(ui, 0.0, "Nothing yet.");
            return;
        }
        for rung in &app.sensor.lod_rungs {
            let (lvl, word) = match rung.state {
                crate::core::sensor::lod::LodState::Tracked => (Level::Pass, "tracked"),
                crate::core::sensor::lod::LodState::Lost => (Level::Fail, "lost"),
                crate::core::sensor::lod::LodState::Marginal => (Level::Warn, "marginal"),
                crate::core::sensor::lod::LodState::Unknown => (Level::Off, "-"),
            };
            ui.horizontal(|ui| {
                w::fixed_label(
                    ui,
                    &format!("{:.2} mm", rung.height_mm),
                    12,
                    Level::Info,
                );
                w::fixed_value(ui, word, 10, lvl);
            });
        }
        if let Some(s) = &app.sensor.lod_summary {
            ui.add_space(4.0);
            if s.verdict == Verdict::Inconclusive {
                w::status_line(ui, Level::Off, s.note);
            } else {
                w::status_line(
                    ui,
                    if s.verdict == Verdict::Fail { Level::Fail } else { Level::Pass },
                    &format!(
                        "Lift-off distance is between {:.2} and {:.2} mm. {}",
                        s.tracked_to_mm, s.lost_at_mm, s.note
                    ),
                );
                w::readout(ui, "bracket", &format!("{:.2}", s.bracket_mm), 11, "mm", Level::Info);
                w::note_indent(
                    ui,
                    0.0,
                    "The bracket can never be narrower than one card. There is no single \
                     number to give, and a midpoint would invent a precision the cards \
                     cannot carry.",
                );
            }
        }
    });
}
