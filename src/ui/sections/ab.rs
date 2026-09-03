//! Blinded, interleaved comparison of two mouse settings.

use crate::app::{AbPhase, App};
use crate::core::ab::{Condition, Variant};
use crate::core::abstats;
use crate::ui::theme::{self, Level};
use crate::ui::widgets as w;

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    w::heading(ui, "A/B COMPARISON");

    match app.ab.phase {
        AbPhase::Setup => setup(app, ui),
        AbPhase::Prompt => prompt(app, ui),
        AbPhase::Countdown => countdown(app, ui),
        AbPhase::Trial => trial(app, ui),
        AbPhase::Done => results(app, ui),
    }
}

fn setup(app: &mut App, ui: &mut egui::Ui) {
    w::note_indent(
        ui,
        0.0,
        "For comparing two settings you can only change by hand, when your own technique \
         varies more than the difference you are looking for. Trials alternate between the \
         two settings instead of running all of one and then all of the other, and nothing \
         about the result is shown until the whole run is over.",
    );
    ui.add_space(4.0);
    w::note_indent(
        ui,
        0.0,
        "A setting another section can measure directly does not belong here. Polling rate, \
         CPI, angle snapping, motion smoothing and scroll behaviour are all read straight off \
         the device in POLLING, SENSOR and SCROLL, where the change is hundreds of times \
         larger than the measurement error and no statistics are needed to see it. Run those \
         twice and compare the numbers. This section is for what only your hand can detect, \
         such as debounce.",
    );

    ui.add_space(8.0);
    w::subheading(ui, "the two conditions");
    w::boxed(ui, |ui| {
        ui.horizontal(|ui| {
            w::fixed_label(ui, "condition A", 14, Level::Off);
            ui.add(
                egui::TextEdit::singleline(&mut app.ab.plan.label_a)
                    .desired_width(280.0)
                    .hint_text("for example: 1000 Hz, debounce 4 ms"),
            );
        });
        ui.horizontal(|ui| {
            w::fixed_label(ui, "condition B", 14, Level::Off);
            ui.add(
                egui::TextEdit::singleline(&mut app.ab.plan.label_b)
                    .desired_width(280.0)
                    .hint_text("for example: 1000 Hz, debounce 0 ms"),
            );
        });
        w::note_indent(
            ui,
            0.0,
            "Name the whole configuration, not only the part you are changing, so the export \
             still means something in a month. In the example only the debounce differs: the \
             polling rate is written down because it stayed the same, not because it is being \
             compared.",
        );
    });

    ui.add_space(8.0);
    w::subheading(ui, "what to measure");
    w::boxed(ui, |ui| {
        for v in [Variant::Rate, Variant::WeakInput] {
            let sel = app.ab.plan.variant == v;
            ui.horizontal(|ui| {
                if ui.add(egui::Button::new(v.label()).selected(sel)).clicked() {
                    app.ab.plan.variant = v;
                }
            });
            if sel {
                w::note_indent(ui, 14.0, v.instruction());
            }
        }
        w::note_indent(
            ui,
            0.0,
            "The weak-input variant answers a different question from the rate variant. A \
             press that never registers is not a slow press, so a rate test cannot see it at \
             all; counting how many light presses each setting registers can.",
        );
    });

    ui.add_space(8.0);
    w::subheading(ui, "run shape");
    w::boxed(ui, |ui| {
        ui.horizontal(|ui| {
            w::fixed_label(ui, "trial length", 14, Level::Off);
            for s in [5.0f64, 10.0, 15.0, 30.0] {
                let sel = (app.ab.plan.trial_seconds - s).abs() < 0.01;
                if ui
                    .add(egui::Button::new(format!("{s:.0} s")).selected(sel))
                    .clicked()
                {
                    app.ab.plan.trial_seconds = s;
                }
            }
        });
        ui.horizontal(|ui| {
            w::fixed_label(ui, "pairs", 14, Level::Off);
            for p in [5usize, 8, 12, 20] {
                let sel = app.ab.plan.pairs == p;
                if ui
                    .add(egui::Button::new(format!("{p}")).selected(sel))
                    .clicked()
                {
                    app.ab.plan.pairs = p;
                }
            }
        });
        ui.horizontal(|ui| {
            w::fixed_label(ui, "button", 14, Level::Off);
            let sel = app.ab.plan.button.is_none();
            if ui.add(egui::Button::new("any").selected(sel)).clicked() {
                app.ab.plan.button = None;
            }
            for b in 1u8..=5 {
                let sel = app.ab.plan.button == Some(b);
                if ui.add(egui::Button::new(format!("{b}")).selected(sel)).clicked() {
                    app.ab.plan.button = Some(b);
                }
            }
        });

        let total = app.ab.plan.total_trials();
        let secs = total as f64 * (app.ab.plan.trial_seconds + 6.0);
        w::readout(ui, "trials", &format!("{total}"), 6, "", Level::Info);
        w::readout(
            ui,
            "roughly",
            &format!("{:.0}", secs / 60.0),
            6,
            "min",
            Level::Info,
        );
        w::note_indent(
            ui,
            0.0,
            "Fix the number of pairs now and do not extend the run afterwards. Adding trials \
             until the result looks convincing is how a coin flip becomes a discovery.",
        );
    });

    ui.add_space(8.0);
    if app.ab.plan.ready() {
        if ui.button("start run").clicked() {
            app.ab_start();
        }
    } else {
        w::status_line(
            ui,
            Level::Warn,
            "Give both conditions a distinct label and choose at least five pairs.",
        );
    }
}

fn progress(app: &App, ui: &mut egui::Ui) {
    let total = app.ab.plan.total_trials();
    w::readout(
        ui,
        "trial",
        &format!("{} of {}", app.ab.current + 1, total),
        10,
        "",
        Level::Info,
    );
}

fn big_instruction(ui: &mut egui::Ui, text: &str, level: Level) {
    w::boxed(ui, |ui| {
        ui.add(egui::Label::new(
            egui::RichText::new(text).heading().color(level.color()),
        ));
    });
}

fn prompt(app: &mut App, ui: &mut egui::Ui) {
    let c = app.ab.condition_now();
    let label = app.ab.plan.label(c).to_string();
    let which = if c == Condition::A { "A" } else { "B" };

    big_instruction(
        ui,
        &format!("Set the mouse to condition {which}:  {label}"),
        Level::Warn,
    );
    ui.add_space(6.0);
    progress(app, ui);
    ui.add_space(6.0);
    w::note_indent(
        ui,
        0.0,
        "Change the setting on the mouse now. Press ready when it is done, and a short \
         countdown will start before the trial records.",
    );
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        if ui.button("ready").clicked() {
            app.ab_ready();
        }
        ui.add_space(12.0);
        if ui.button("abandon run").clicked() {
            app.ab_abandon();
        }
    });
    ui.add_space(8.0);
    w::status_line(
        ui,
        Level::Off,
        "No results are shown until every trial is finished, so what you have scored so far \
         cannot change how hard you try on the rest.",
    );
}

fn countdown(app: &mut App, ui: &mut egui::Ui) {
    let left = (app.ab.countdown_s - app.ab.phase_elapsed()).max(0.0);
    big_instruction(ui, &format!("starting in {left:.1} s"), Level::Warn);
    ui.add_space(6.0);
    progress(app, ui);
}

fn trial(app: &mut App, ui: &mut egui::Ui) {
    let left = (app.ab.plan.trial_seconds - app.ab.phase_elapsed()).max(0.0);
    big_instruction(ui, "RECORDING", Level::Fail);
    ui.add_space(6.0);
    w::readout(ui, "time remaining", &format!("{left:.1}"), 8, "s", Level::Warn);
    progress(app, ui);
    ui.add_space(6.0);
    w::note_indent(ui, 0.0, app.ab.plan.variant.instruction());
    ui.add_space(6.0);
    if ui.button("abandon run").clicked() {
        app.ab_abandon();
    }
}

fn results(app: &mut App, ui: &mut egui::Ui) {
    let run = match app.ab.run.clone() {
        Some(r) => r,
        None => return,
    };
    let report = run.analyse(0.05);
    let unit = run.plan.variant.unit();

    w::subheading(ui, "result");
    w::boxed(ui, |ui| {
        let (shift, exact, level) = match &report.paired_shift {
            Some(s) => (Some(s.clone()), s.exact, Level::Info),
            None => (None, false, Level::Info),
        };
        let _ = (exact, level);

        if let Some(s) = &shift {
            let (better, worse) = if s.estimate >= 0.0 {
                (run.plan.label_a.clone(), run.plan.label_b.clone())
            } else {
                (run.plan.label_b.clone(), run.plan.label_a.clone())
            };
            let mag = s.estimate.abs();
            let verdict_level = if report.p_primary < 0.05 {
                Level::Pass
            } else {
                Level::Off
            };
            w::status_line(
                ui,
                verdict_level,
                &if report.p_primary < 0.05 {
                    format!(
                        "{better} beat {worse} by {mag:.3} {unit} at the median."
                    )
                } else {
                    format!(
                        "No difference this run can distinguish. The best estimate is \
                         {mag:.3} {unit} in favour of {better}, but the interval below \
                         includes no difference at all."
                    )
                },
            );
            ui.add_space(6.0);
            w::readout(
                ui,
                "median difference",
                &format!("{:+.3}", s.estimate),
                11,
                unit,
                Level::Info,
            );
            w::readout(ui, "interval low", &format!("{:+.3}", s.lo), 11, unit, Level::Info);
            w::readout(ui, "interval high", &format!("{:+.3}", s.hi), 11, unit, Level::Info);
            // The achieved coverage is shown, not a nominal "95%": with small
            // discrete samples the interval a distribution-free method can
            // actually deliver is coarser than the level requested.
            w::readout(
                ui,
                "interval covers",
                &format!("{:.1}", s.achieved_level * 100.0),
                11,
                "%",
                Level::Info,
            );
            if !s.exact {
                w::note_indent(
                    ui,
                    0.0,
                    "Ties or run length prevented the exact interval, so this one is \
                     approximate and errs towards being too wide.",
                );
            }
            w::note_indent(
                ui,
                0.0,
                "Positive means condition A scored higher. The estimate is the median of \
                 every paired difference, which is not the same as the difference of the \
                 medians and is the right thing to quote for a paired comparison.",
            );
        }

        ui.add_space(6.0);
        let p_level = if report.p_primary < 0.05 {
            Level::Pass
        } else {
            Level::Off
        };
        w::readout(
            ui,
            "p value",
            &format!("{:.4}", report.p_primary),
            11,
            "",
            p_level,
        );
        w::kv(
            ui,
            "primary test",
            match report.primary {
                abstats::Primary::Paired => {
                    "Wilcoxon signed-rank on the paired differences (no normality assumed)"
                }
                abstats::Primary::Unpaired => {
                    "Mann-Whitney U (no normality assumed)"
                }
            },
        );
        if let Some(p) = &report.paired {
            if p.rank_biserial.is_finite() {
                w::readout(
                    ui,
                    "effect size",
                    &format!("{:+.3}", p.rank_biserial),
                    11,
                    "",
                    Level::Info,
                );
                w::note_indent(
                    ui,
                    0.0,
                    "Rank-biserial correlation: the balance of pairs favouring one setting \
                     over the other, from -1 to +1.",
                );
            } else {
                w::kv_level(
                    ui,
                    "effect size",
                    "undefined: every pair scored identically",
                    Level::Off,
                );
            }
        }
        for n in &report.notes {
            w::note_indent(ui, 0.0, n);
        }
    });

    ui.add_space(8.0);
    w::subheading(ui, "each condition");
    w::boxed(ui, |ui| {
        for (name, s) in [
            (run.plan.label_a.clone(), &report.summary_a),
            (run.plan.label_b.clone(), &report.summary_b),
        ] {
            w::kv(ui, "condition", &name);
            ui.horizontal(|ui| {
                ui.add_space(14.0);
                ui.vertical(|ui| {
                    w::readout(ui, "median", &format!("{:.3}", s.median), 10, unit, Level::Info);
                    w::readout(
                        ui,
                        "interquartile range",
                        &format!("{:.3}", s.iqr),
                        10,
                        unit,
                        Level::Info,
                    );
                    w::readout(ui, "lowest", &format!("{:.3}", s.min), 10, unit, Level::Info);
                    w::readout(ui, "highest", &format!("{:.3}", s.max), 10, unit, Level::Info);
                    w::readout(ui, "trials", &format!("{}", s.n), 10, "", Level::Info);
                });
            });
            ui.add_space(4.0);
        }
        w::note_indent(
            ui,
            0.0,
            "Medians and interquartile range rather than mean and standard deviation, \
             because click rates are skewed and a single bad trial should not move the \
             summary.",
        );
    });

    ui.add_space(8.0);
    w::subheading(ui, "secondary, descriptive only");
    w::boxed(ui, |ui| {
        w::readout(
            ui,
            "unpaired p value",
            &format!("{:.4}", report.unpaired.p_two_sided),
            11,
            "",
            Level::Off,
        );
        w::readout(
            ui,
            "probability A beats B",
            &format!("{:.3}", report.unpaired.prob_superiority),
            11,
            "",
            Level::Off,
        );
        w::note_indent(ui, 0.0, abstats::MULTIPLICITY_NOTICE);
    });

    ui.add_space(8.0);
    w::subheading(ui, "every trial, in run order");
    w::boxed(ui, |ui| {
        ui.horizontal(|ui| {
            w::fixed_label(ui, "#", 4, Level::Off);
            w::fixed_label(ui, "pair", 6, Level::Off);
            w::fixed_label(ui, "condition", 24, Level::Off);
            w::fixed_label(ui, "clicks", 8, Level::Off);
            w::fixed_label(ui, "value", 10, Level::Off);
        });
        for t in &run.trials {
            ui.horizontal(|ui| {
                w::fixed_value(ui, &format!("{}", t.index + 1), 3, Level::Off);
                ui.add_space(4.0);
                w::fixed_value(ui, &format!("{}", t.pair + 1), 5, Level::Off);
                ui.add_space(4.0);
                w::fixed_label(ui, run.plan.label(t.condition), 24, Level::Info);
                w::fixed_value(ui, &format!("{}", t.presses), 7, Level::Info);
                w::fixed_value(ui, &format!("{:.3}", t.value), 9, Level::Info);
            });
        }
    });

    ui.add_space(8.0);
    ui.horizontal(|ui| {
        if ui.button("export raw data").clicked() {
            app.ab_export();
        }
        ui.add_space(12.0);
        if ui.button("new run").clicked() {
            app.ab_abandon();
        }
    });
    if let Some(p) = &app.ab.last_export {
        w::status_line(ui, Level::Pass, &format!("wrote {p}"));
    }
    if let Some(e) = &app.ab.export_error {
        w::status_line(ui, Level::Fail, &format!("could not write: {e}"));
    }
    let _ = theme::WHITE;
}
