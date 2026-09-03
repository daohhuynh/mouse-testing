//! Click and debounce behaviour, per button.

use crate::app::App;
use crate::core::debounce::{ButtonResult, DebounceConfig};
use crate::core::polling::Verdict;
use crate::platform::Tier;
use crate::ui::theme::Level;
use crate::ui::widgets as w;

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    w::heading(ui, "CLICK AND DEBOUNCE");

    ui.horizontal(|ui| {
        if app.session.running() {
            if ui.button("stop").clicked() {
                app.session.stop();
            }
        } else if ui.button("start now").clicked() {
            app.start_capture(0.0);
        }
        ui.add_space(10.0);
        if ui.button("clear button history").clicked() {
            app.session.buttons.clear();
            app.session.button_source = None;
        }
    });

    ui.add_space(4.0);
    w::note_indent(
        ui,
        0.0,
        "Click deliberately, about once a second, rather than as fast as you can. Contact \
         bounce is a press registering twice within a few milliseconds, and spam clicking \
         compresses your own gaps into the same range, which makes the two impossible to \
         tell apart. Around a hundred presses per button gives a good chance of catching a \
         switch that misfires two percent of the time; two hundred makes it near certain.",
    );

    ui.add_space(8.0);
    let source = app.session.button_source;
    let events = app.session.buttons.clone();
    let cfg = DebounceConfig::default();

    w::boxed(ui, |ui| {
        w::kv(
            ui,
            "edges recorded",
            &format!("{}", events.len()),
        );
        match source {
            Some(t) => w::kv(
                ui,
                "source",
                &format!("{} level, {}", t.short(), t.source_name()),
            ),
            None => w::kv(ui, "source", "nothing recorded yet"),
        }
        if source == Some(Tier::System) {
            w::note_indent(
                ui,
                0.0,
                "Falling back to the system level because the device level is unavailable. \
                 Timing here is stamped by this program rather than by the driver, and the \
                 events are not attributed to a particular mouse.",
            );
        }
    });

    if events.is_empty() {
        ui.add_space(8.0);
        w::status_line(
            ui,
            Level::Off,
            "Start a run and press the mouse buttons. Every button is listed by its raw \
             identifier, so a button the system does not map still appears here.",
        );
        return;
    }

    let results = crate::core::debounce::analyze_all(&events, &cfg);
    for r in &results {
        ui.add_space(10.0);
        button_block(ui, r, &cfg);
    }
}

fn button_block(ui: &mut egui::Ui, r: &ButtonResult, cfg: &DebounceConfig) {
    w::subheading(ui, &format!("button {} (raw identifier)", r.button));
    w::boxed(ui, |ui| {
        w::status_line(
            ui,
            r.verdict.level(),
            match r.verdict {
                Verdict::Pass => "No contact bounce detected.",
                Verdict::Warn => "Something is off. See the counts below.",
                Verdict::Fail => "This button registers presses it was not given.",
                Verdict::Inconclusive => "Not enough evidence to judge.",
            },
        );
        if !r.note.is_empty() {
            w::note_indent(ui, 14.0, r.note);
        }
        ui.add_space(6.0);

        w::readout(ui, "presses", &format!("{}", r.n_down), 9, "", Level::Info);
        w::readout(ui, "releases", &format!("{}", r.n_up), 9, "", Level::Info);
        let mismatch = r.unmatched_down + r.unmatched_up;
        w::readout(
            ui,
            "unmatched edges",
            &format!("{mismatch}"),
            9,
            "",
            if mismatch == 0 { Level::Pass } else { Level::Warn },
        );
        if mismatch > 0 {
            w::note_indent(
                ui,
                0.0,
                &format!(
                    "{} press(es) with no release and {} release(s) with no press.",
                    r.unmatched_down, r.unmatched_up
                ),
            );
        }

        ui.add_space(6.0);
        let bounce_level = if r.n_doublets > 0 || r.bounce_rate >= cfg.rate_fail {
            Level::Fail
        } else if r.n_bounce_fail > 0 || r.n_bounce_warn > 0 {
            Level::Warn
        } else {
            Level::Pass
        };
        w::readout(
            ui,
            "bounce doublets",
            &format!("{}", r.n_doublets),
            9,
            "",
            if r.n_doublets > 0 { Level::Fail } else { Level::Pass },
        );
        w::note_indent(
            ui,
            0.0,
            &format!(
                "A press starting within {:.1} ms of the previous release and held for less \
                 than {:.1} ms. That is below anything a hand can do and above where a \
                 healthy switch has finished settling.",
                r.doublet_gap_ms, cfg.dwell_fail_ms
            ),
        );
        w::readout(
            ui,
            "gaps under 15 ms",
            &format!("{}", r.n_bounce_fail),
            9,
            "",
            bounce_level,
        );
        w::readout(
            ui,
            "gaps 15 to 35 ms",
            &format!("{}", r.n_bounce_warn),
            9,
            "",
            if r.n_bounce_warn > 0 { Level::Warn } else { Level::Pass },
        );
        w::readout(
            ui,
            "presses under 15 ms",
            &format!("{}", r.n_short_dwell_fail),
            9,
            "",
            if r.n_short_dwell_fail > 0 { Level::Warn } else { Level::Pass },
        );

        ui.add_space(6.0);
        w::readout(
            ui,
            "shortest gap",
            &fmt_ms(r.min_gap_ms),
            9,
            "ms",
            Level::Info,
        );
        w::readout(ui, "median gap", &fmt_ms(r.median_gap_ms), 9, "ms", Level::Info);
        w::readout(ui, "shortest press", &fmt_ms(r.min_dwell_ms), 9, "ms", Level::Info);
        w::readout(ui, "median press", &fmt_ms(r.median_dwell_ms), 9, "ms", Level::Info);
        if r.spam_clicking {
            w::note_indent(
                ui,
                0.0,
                "Your median gap says you were clicking fast rather than deliberately, so \
                 the suspect band is reported as inconclusive instead of as a warning.",
            );
        }

        if !r.dwell_ms.is_empty() {
            ui.add_space(6.0);
            w::kv(ui, "press duration", "");
            w::histogram(ui, &bins(&r.dwell_ms, 0.0, 300.0, 60), 64.0, &[], "0 to 300 ms");
        }
        if !r.gap_ms.is_empty() {
            ui.add_space(4.0);
            w::kv(ui, "gap between clicks", "");
            w::histogram(ui, &bins(&r.gap_ms, 0.0, 1000.0, 60), 64.0, &[(15.0 / 1000.0 * 1000.0, "15 ms")], "0 to 1000 ms");
        }
    });
}

fn fmt_ms(v: f64) -> String {
    if v.is_finite() {
        format!("{v:.1}")
    } else {
        "-".to_string()
    }
}

fn bins(values: &[f64], lo: f64, hi: f64, n: usize) -> Vec<(f64, u32)> {
    let w = (hi - lo) / n as f64;
    let mut b = vec![0u32; n];
    for &v in values {
        if !v.is_finite() {
            continue;
        }
        let i = (((v - lo) / w) as isize).clamp(0, n as isize - 1) as usize;
        b[i] += 1;
    }
    b.iter()
        .enumerate()
        .map(|(i, &c)| (lo + (i as f64 + 0.5) * w, c))
        .collect()
}
