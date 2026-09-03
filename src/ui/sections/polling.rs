//! Polling rate: what the device sends, and what survives to an application.
//!
//! The two figures are shown side by side and are never reconciled. A large
//! gap between them is the expected finding on both platforms, not a fault.

use crate::app::App;
use crate::capture::LevelState;
use crate::core::polling::{PollConfig, Verdict};
use crate::platform::Tier;
use crate::ui::theme::{self, Level};
use crate::ui::widgets as w;

const LIVE_WINDOW_NS: u64 = 250_000_000;

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    w::heading(ui, "POLLING RATE");

    controls(app, ui);
    ui.add_space(8.0);
    levels(app, ui);
    ui.add_space(10.0);
    claimed(app, ui);
    ui.add_space(10.0);
    distribution(app, ui);
}

fn controls(app: &mut App, ui: &mut egui::Ui) {
    let running = app.session.running();
    ui.horizontal(|ui| {
        if running {
            if ui.button("stop").clicked() {
                app.session.stop();
            }
        } else if ui.button("start now").clicked() {
            app.start_poll_run(0.0);
        }

        ui.add_space(10.0);
        ui.add(egui::Label::new(
            egui::RichText::new("delayed start").color(theme::GREY_TEXT),
        ));
        for secs in [3.0f64, 5.0, 10.0] {
            if ui
                .add_enabled(!running, egui::Button::new(format!("{secs:.0} s")))
                .clicked()
            {
                app.start_poll_run(secs);
            }
        }

        ui.add_space(10.0);
        if w::chip(ui, app.poll_auto_stop, "stop when settled").clicked() {
            app.poll_auto_stop = !app.poll_auto_stop;
        }
    });

    ui.add_space(2.0);
    w::note_indent(
        ui,
        0.0,
        "With \"stop when settled\" on, the run ends by itself once the confidence interval on \
         both rates sits inside one verdict band, so more swiping could not change the answer. \
         A rate sitting on a threshold keeps it going, and it never stops on a result this \
         section would refuse to give.",
    );

    if app.poll_auto_stopped && !running {
        ui.add_space(2.0);
        w::status_line(
            ui,
            Level::Pass,
            "Stopped on its own: the answer stopped moving, so more swiping could not change it.",
        );
    }

    if let Some(remaining) = app.countdown_remaining() {
        ui.add_space(2.0);
        w::status_line(
            ui,
            Level::Warn,
            &format!(
                "starting in {remaining:.1} s  \u{2014}  let go of this machine's pointer and \
                 pick up the mouse under test"
            ),
        );
    }

    ui.add_space(2.0);
    w::note_indent(
        ui,
        0.0,
        "F5 or Space starts and stops a run. That only works while this window has keyboard \
         focus: neither platform gives an ordinary application a system-wide hotkey without a \
         separate permission, so use the delayed start if you need to be in another window.",
    );

    if running {
        ui.add_space(4.0);
        w::readout(
            ui,
            "elapsed",
            &format!("{:.1}", app.session.elapsed_s()),
            8,
            "s",
            Level::Info,
        );
    }
}

fn state_level(s: LevelState) -> Level {
    match s {
        LevelState::Live => Level::Pass,
        LevelState::Waiting => Level::Warn,
        LevelState::Blocked => Level::Fail,
        LevelState::Idle => Level::Off,
    }
}

fn state_text(s: LevelState) -> &'static str {
    match s {
        LevelState::Live => "receiving",
        LevelState::Waiting => "waiting for input",
        LevelState::Blocked => "blocked",
        LevelState::Idle => "not started",
    }
}

fn levels(app: &mut App, ui: &mut egui::Ui) {
    w::subheading(ui, "rate by level, measured simultaneously");
    let sess = &app.session;

    w::boxed(ui, |ui| {
        for tier in Tier::ALL {
            let series = sess.tier_series(tier);
            let state = match tier {
                Tier::Device => sess.device_state,
                Tier::System => sess.system_state,
                Tier::App => {
                    if sess.running() {
                        if sess.app.total > 0 {
                            LevelState::Live
                        } else {
                            LevelState::Waiting
                        }
                    } else {
                        LevelState::Idle
                    }
                }
            };

            ui.horizontal(|ui| {
                w::fixed_label(ui, tier.short(), 8, Level::Info);
                w::tag(ui, state_level(state), state_text(state));
            });

            let (live, sustained) = match tier {
                Tier::App => (f64::NAN, sess.app_hz()),
                _ => (series.live_hz(LIVE_WINDOW_NS), series.sustained_hz()),
            };

            ui.horizontal(|ui| {
                ui.add_space(10.0);
                ui.vertical(|ui| {
                    if live.is_nan() {
                        w::readout(ui, "live rate", "n/a", 11, "Hz", Level::Off);
                    } else {
                        w::readout(
                            ui,
                            "live rate",
                            &format!("{live:.1}"),
                            11,
                            "Hz",
                            Level::Info,
                        );
                    }
                    w::readout(
                        ui,
                        "sustained rate",
                        &format!("{sustained:.1}"),
                        11,
                        "Hz",
                        Level::Info,
                    );
                    w::readout(
                        ui,
                        "events",
                        &format!("{}", series.total),
                        11,
                        "",
                        Level::Info,
                    );
                    if series.ring_drops > 0 {
                        w::readout(
                            ui,
                            "lost to buffer overrun",
                            &format!("{}", series.ring_drops),
                            11,
                            "",
                            Level::Fail,
                        );
                    }
                });
            });

            let note = match tier {
                Tier::Device => {
                    if !sess.device_note.is_empty() {
                        sess.device_note.clone()
                    } else {
                        format!(
                            "{}. Per-device, and the only level with device timestamps.",
                            tier.source_name()
                        )
                    }
                }
                Tier::System => {
                    if !sess.system_note.is_empty() {
                        sess.system_note.clone()
                    } else {
                        format!(
                            "{}. System-wide: this platform attaches no device identity to a \
                             system mouse event, so this is the sum of every pointing device \
                             in use.",
                            tier.source_name()
                        )
                    }
                }
                Tier::App => format!(
                    "{}. Only counts while the pointer is over this window. The framework \
                     timestamps a whole frame rather than each event, so this level reports a \
                     rate but cannot report an interval distribution.",
                    tier.source_name()
                ),
            };
            w::note_indent(ui, 10.0, &note);
            ui.add_space(6.0);
        }
    });

    // The control. Without it, a level reading zero could equally mean "you
    // did not move the mouse" or "this level is broken", and the program would
    // have no way to tell the user which.
    if sess.running() || sess.control_motion > 0 {
        ui.add_space(6.0);
        w::boxed(ui, |ui| {
            w::readout(
                ui,
                "control: OS motion count",
                &format!("{}", sess.control_motion),
                11,
                "",
                if sess.control_motion > 0 {
                    Level::Info
                } else {
                    Level::Warn
                },
            );
            let text = if sess.control_motion == 0 {
                "The operating system's own counter says nothing has moved yet, so a level \
                 reading zero above means no input, not a fault."
            } else if sess.device.total == 0 && sess.system.total == 0 {
                "The operating system counted motion, but no level received any. That is a \
                 fault in the capture, not an absence of input."
            } else {
                "The operating system's own motion counter, read without any permission. It \
                 exists so that a level reading zero can be told apart from a level that is \
                 not working."
            };
            w::note_indent(ui, 0.0, text);
        });
    }

    // The comparison the section exists for.
    let dev = sess.device.sustained_hz();
    let sys = sess.system.sustained_hz();
    let appr = sess.app_hz();
    if dev > 1.0 {
        ui.add_space(6.0);
        w::boxed(ui, |ui| {
            w::readout(
                ui,
                "system / device",
                &format!("{:.1}", 100.0 * sys / dev),
                8,
                "%",
                Level::Info,
            );
            w::readout(
                ui,
                "application / device",
                &format!("{:.1}", 100.0 * appr / dev),
                8,
                "%",
                Level::Info,
            );
            w::note_indent(
                ui,
                0.0,
                "A large gap here is the expected result, not a defect. The application level \
                 is bounded by how often this program redraws, because the OS keeps at most \
                 one pending pointer move per application and discards the rest. The device \
                 level is what the mouse actually delivered.",
            );
        });
    }
}

fn claimed(app: &mut App, ui: &mut egui::Ui) {
    w::subheading(ui, "measured against claimed");
    w::boxed(ui, |ui| {
        ui.horizontal(|ui| {
            w::fixed_label(ui, "configured rate", w::LABEL_CHARS, Level::Off);
            ui.add(
                egui::TextEdit::singleline(&mut app.claimed_hz)
                    .desired_width(90.0)
                    .hint_text("e.g. 1000"),
            );
            ui.add(egui::Label::new(
                egui::RichText::new("Hz").color(theme::GREY_TEXT),
            ));
        });
        w::note_indent(
            ui,
            0.0,
            "What the mouse is set to, as you configured it. Entered by hand because no OS \
             reports a USB mouse's true polling interval to an application.",
        );

        let claimed: Option<f64> = app.claimed_hz.trim().parse().ok().filter(|v: &f64| *v > 0.0);
        let measured = app.poll_result.nominal_hz;
        if let (Some(c), true) = (claimed, measured.is_finite() && measured > 0.0) {
            let err = (measured - c) / c * 100.0;
            let level = if err.abs() <= 2.0 {
                Level::Pass
            } else if err.abs() <= 10.0 {
                Level::Warn
            } else {
                Level::Fail
            };
            ui.add_space(4.0);
            w::readout(ui, "measured", &format!("{measured:.1}"), 11, "Hz", Level::Info);
            w::readout(ui, "claimed", &format!("{c:.1}"), 11, "Hz", Level::Info);
            w::readout(ui, "difference", &format!("{err:+.2}"), 11, "%", level);
        }
    });
}

fn distribution(app: &mut App, ui: &mut egui::Ui) {
    w::subheading(ui, "interval distribution, device level");
    let r = app.poll_result.clone();
    let cfg = PollConfig::default();

    w::boxed(ui, |ui| {
        if r.n_intervals == 0 {
            w::note_indent(ui, 0.0, "No intervals yet. Start a run and move the mouse.");
            return;
        }

        w::status_line(
            ui,
            r.verdict.level(),
            match r.verdict {
                Verdict::Pass => "No dropped or late reports beyond the noise floor.",
                Verdict::Warn => "Some reports are arriving late or missing.",
                Verdict::Fail => "Reports are being dropped or delayed at a rate you would feel.",
                Verdict::Inconclusive => "Not enough information for a verdict.",
            },
        );
        if !r.note.is_empty() {
            w::note_indent(ui, 14.0, r.note);
        }
        ui.add_space(6.0);

        let marks = [
            (r.nominal_ns, "1x"),
            (r.nominal_ns * 2.0, "2x"),
            (r.nominal_ns * 3.0, "3x"),
        ];
        w::histogram(ui, &r.histogram, 96.0, &marks, "interval");
        ui.add_space(6.0);

        w::readout(
            ui,
            "nominal interval",
            &format!("{:.1}", r.nominal_ns / 1000.0),
            11,
            "us",
            Level::Info,
        );
        w::readout(
            ui,
            "nominal rate",
            &format!("{:.1}", r.nominal_hz),
            11,
            "Hz",
            Level::Info,
        );
        if let Some(s) = r.snapped_hz {
            w::kv(ui, "matches standard rate", &format!("{s:.0} Hz"));
        }
        w::readout(
            ui,
            "interval jitter",
            &format!("{:.2}", r.jitter_sigma_ns / 1000.0),
            11,
            "us",
            Level::Info,
        );
        w::readout(ui, "shortest", &format!("{:.1}", r.min_ns / 1000.0), 11, "us", Level::Info);
        w::readout(ui, "median", &format!("{:.1}", r.p50_ns / 1000.0), 11, "us", Level::Info);
        w::readout(ui, "99th percentile", &format!("{:.1}", r.p99_ns / 1000.0), 11, "us", Level::Info);
        w::readout(ui, "99.9th percentile", &format!("{:.1}", r.p999_ns / 1000.0), 11, "us", Level::Info);
        w::readout(ui, "longest", &format!("{:.1}", r.max_ns / 1000.0), 11, "us", Level::Info);

        ui.add_space(6.0);
        // Dropped and slow are counted separately on purpose: a report that
        // never arrived and one that arrived late are different faults.
        let drop_level = if r.drop_rate >= cfg.drop_fail {
            Level::Fail
        } else if r.drop_rate >= cfg.drop_warn {
            Level::Warn
        } else {
            Level::Pass
        };
        w::readout(
            ui,
            "dropped reports",
            &format!("{:.3}", r.drop_rate * 100.0),
            11,
            "%",
            drop_level,
        );
        w::readout(
            ui,
            "  as whole reports",
            &format!("{}", r.n_dropped_slots),
            11,
            "",
            drop_level,
        );
        let slow_level = if r.slow_rate >= cfg.slow_fail {
            Level::Fail
        } else if r.slow_rate >= cfg.slow_warn {
            Level::Warn
        } else {
            Level::Pass
        };
        w::readout(
            ui,
            "late reports",
            &format!("{:.3}", r.slow_rate * 100.0),
            11,
            "%",
            slow_level,
        );
        w::readout(ui, "  count", &format!("{}", r.n_slow), 11, "", slow_level);
        w::note_indent(
            ui,
            0.0,
            "A dropped report is one that never arrived: the gap is a whole multiple of the \
             polling interval. A late report arrived, but off schedule. Idle time, when the \
             mouse had nothing to send, is excluded from both.",
        );

        ui.add_space(4.0);
        w::readout(
            ui,
            "intervals judged",
            &format!("{}", r.n_analyzable),
            11,
            "",
            if r.n_analyzable >= cfg.min_analyzable {
                Level::Pass
            } else {
                Level::Warn
            },
        );
        w::readout(ui, "intervals total", &format!("{}", r.n_intervals), 11, "", Level::Info);
        w::note_indent(
            ui,
            0.0,
            "Only intervals where the mouse was moving fast enough that every polling slot had \
             to carry motion can be judged. Slow movement is excluded rather than counted as \
             dropped reports.",
        );
    });
}
