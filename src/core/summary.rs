//! The readable summary: everything measured this session, as plain text.
//!
//! Written for a person reading it in a month, or pasting it into a support
//! thread, so every figure carries its unit and every measurement that was not
//! taken says so rather than being omitted. A missing line and a measurement
//! that came back clean look identical otherwise.

use crate::app::App;
use crate::core::session_log::Meta;
use std::fmt::Write as _;

pub fn render(app: &App, meta: &Meta) -> String {
    let mut s = String::with_capacity(4096);
    let _ = writeln!(s, "MOUSE TESTING SUMMARY");
    let _ = writeln!(s);

    section(&mut s, "DEVICE");
    kv(&mut s, "name", &meta.device_name);
    kv(&mut s, "identifiers", &meta.device_ids);
    kv(&mut s, "transport", &meta.transport);
    kv(&mut s, "configured rate", &plain(&meta.claimed_hz, "Hz"));
    kv(&mut s, "configured CPI", &plain(&meta.claimed_cpi, "CPI"));
    let _ = writeln!(s);

    section(&mut s, "HOST");
    kv(&mut s, "operating system", &meta.os);
    kv(&mut s, "architecture", &meta.arch);
    kv(&mut s, "processor", &meta.cpu);
    kv(&mut s, "timestamp clock", &meta.clock);
    kv(
        &mut s,
        "clock resolution",
        &format!("{:.1} ns", meta.clock_resolution_ns),
    );
    kv(&mut s, "clock read cost", &format!("{:.1} ns", meta.clock_cost_ns));
    if meta.warnings.is_empty() {
        kv(&mut s, "measurement validity", "nothing detected that would invalidate timing");
    } else {
        for w in &meta.warnings {
            kv(&mut s, "WARNING", w);
        }
    }
    let _ = writeln!(s);

    section(&mut s, "CAPTURE");
    kv(&mut s, "duration", &format!("{:.1} s", meta.duration_s));
    use crate::capture::LevelState;
    for (name, series, state) in [
        ("device", &app.session.device, app.session.device_state),
        ("system", &app.session.system, app.session.system_state),
        // The app level has no permission to be blocked by and nothing to wait
        // on, so its state is just whether anything arrived. Reporting it as
        // Live with zero events, which an earlier version did, said the
        // opposite of what the zero meant.
        (
            "app",
            &app.session.app,
            if app.session.app.total > 0 {
                LevelState::Live
            } else {
                LevelState::Idle
            },
        ),
    ] {
        let st = format!("{state:?}");
        kv(
            &mut s,
            &format!("{name} level"),
            &format!(
                "{st}, {} event(s), {:.2} Hz sustained, {} buffer loss(es)",
                series.total,
                series.sustained_hz(),
                series.ring_drops
            ),
        );
    }
    if !app.session.device_note.is_empty() {
        kv(&mut s, "device note", &app.session.device_note);
    }
    if !app.session.system_note.is_empty() {
        kv(&mut s, "system note", &app.session.system_note);
    }
    if app.session.background_events > 0 {
        kv(
            &mut s,
            "background events",
            &format!(
                "{} arrived while this app was not in the foreground",
                app.session.background_events
            ),
        );
    }
    if app.session.injected_events > 0 {
        kv(
            &mut s,
            "WARNING",
            &format!(
                "{} event(s) were synthesised by software rather than produced by the \
                 mouse. Any measurement covering them describes that program.",
                app.session.injected_events
            ),
        );
    }
    kv(
        &mut s,
        "OS motion counter",
        &format!(
            "{} event(s) counted by the OS over this run",
            app.session.control_motion
        ),
    );
    let _ = writeln!(s);

    polling(&mut s, app);
    clicks(&mut s, app);
    cps(&mut s, app);
    ab(&mut s, app);
    sensor(&mut s, app);
    scroll(&mut s, app);
    s
}

fn section(s: &mut String, title: &str) {
    let _ = writeln!(s, "{title}");
    let _ = writeln!(s, "{}", "-".repeat(title.len()));
}

fn kv(s: &mut String, k: &str, v: &str) {
    let _ = writeln!(s, "  {k:<24}  {v}");
}

fn plain(v: &str, unit: &str) -> String {
    if v.trim().is_empty() {
        "not stated".into()
    } else {
        format!("{} {unit}", v.trim())
    }
}

/// A millisecond figure, or a reason there is not one. A statistic over too few
/// samples is not a number, and printing the NaN it computes to would be worse
/// than saying so.
fn ms(v: f64) -> String {
    if v.is_finite() {
        format!("{v:.1} ms")
    } else {
        "too few to say".into()
    }
}

fn not_measured(s: &mut String, what: &str) {
    kv(s, what, "not measured this session");
}

fn polling(s: &mut String, app: &App) {
    section(s, "POLLING");
    let r = &app.poll_result;
    if r.n_intervals == 0 {
        not_measured(s, "interval analysis");
        let _ = writeln!(s);
        return;
    }
    kv(s, "verdict", &format!("{:?}", r.verdict));
    kv(s, "note", r.note);
    kv(
        s,
        "nominal interval",
        &format!("{:.1} us  ({:.1} Hz)", r.nominal_ns / 1000.0, r.nominal_hz),
    );
    kv(s, "sustained rate", &format!("{:.2} Hz", r.effective_hz));
    kv(
        s,
        "intervals judged",
        &format!("{} of {}", r.n_analyzable, r.n_intervals),
    );
    kv(
        s,
        "dropped reports",
        &format!("{} ({:.4} %)", r.n_drop_events, r.drop_rate * 100.0),
    );
    kv(s, "slow reports", &format!("{}", r.n_slow));
    let _ = writeln!(s);
    let _ = writeln!(
        s,
        "  Levels are measured separately and are expected to disagree. The device\n  \
         figure is what the mouse put on the wire; the app figure is what a normal\n  \
         program receives after the operating system has processed it."
    );
    let _ = writeln!(s);
}

fn clicks(s: &mut String, app: &App) {
    section(s, "CLICKS AND DEBOUNCE");
    let buttons = &app.session.buttons;
    if buttons.is_empty() {
        not_measured(s, "button activity");
        let _ = writeln!(s);
        return;
    }
    kv(s, "transitions recorded", &format!("{}", buttons.len()));
    kv(
        s,
        "source level",
        &app.session
            .button_source
            .map(|t| t.short().to_string())
            .unwrap_or_else(|| "none".into()),
    );
    let mut ids: Vec<u8> = buttons.iter().map(|b| b.button).collect();
    ids.sort_unstable();
    ids.dedup();
    for id in ids {
        let evs: Vec<_> = buttons.iter().filter(|b| b.button == id).copied().collect();
        let r = crate::core::debounce::analyze_button(
            &evs,
            id,
            &crate::core::debounce::DebounceConfig::default(),
        );
        kv(
            s,
            &format!("button {id}"),
            &format!(
                "{:?}: {} press(es), {} release(s), {} unmatched, {} doublet(s), \
                 median gap {}, median dwell {}",
                r.verdict,
                r.n_down,
                r.n_up,
                r.unmatched_down + r.unmatched_up,
                r.n_doublets,
                ms(r.median_gap_ms),
                ms(r.median_dwell_ms)
            ),
        );
    }
    let _ = writeln!(s);
}

fn cps(s: &mut String, app: &App) {
    section(s, "CLICKS PER SECOND");
    if app.cps.history.is_empty() {
        not_measured(s, "click rate");
        let _ = writeln!(s);
        return;
    }
    for (i, run) in app.cps.history.iter().enumerate() {
        kv(
            s,
            &format!("run {}", i + 1),
            &format!(
                "{}, button {}, {:.0} s: {:.2} CPS sustained, {:.2} CPS peak, {} press(es)",
                run.mode, run.button, run.duration_s, run.sustained_cps, run.peak_cps, run.presses
            ),
        );
    }
    let _ = writeln!(
        s,
        "\n  Sustained is the figure that means anything. Peak is a one-second window\n  \
         and is mostly noise."
    );
    let _ = writeln!(s);
}

fn ab(s: &mut String, app: &App) {
    section(s, "A/B COMPARISON");
    let Some(run) = app.ab.run.as_ref() else {
        not_measured(s, "comparison");
        let _ = writeln!(s);
        return;
    };
    let rep = run.analyse(0.05);
    let unit = run.plan.variant.unit();
    kv(s, "condition A", &run.plan.label_a);
    kv(s, "condition B", &run.plan.label_b);
    kv(s, "variant", run.plan.variant.label());
    kv(s, "trials", &format!("{}", run.trials.len()));
    let shift = rep.paired_shift.as_ref().unwrap_or(&rep.unpaired_shift);
    kv(
        s,
        "median difference",
        &format!("{:+.3} {unit}", shift.estimate),
    );
    kv(
        s,
        "interval",
        &format!(
            "{:+.3} to {:+.3} {unit}, covering {:.1} %{}",
            shift.lo,
            shift.hi,
            shift.achieved_level * 100.0,
            if shift.exact { "" } else { " (conservative: the sample has ties)" }
        ),
    );
    kv(s, "p value", &format!("{:.4}", rep.p_primary));
    kv(
        s,
        "test",
        match rep.primary {
            crate::core::abstats::Primary::Paired => {
                "Wilcoxon signed-rank on the paired differences"
            }
            _ => "Mann-Whitney U (the run could not be paired)",
        },
    );
    if let Some(w) = &rep.paired {
        if w.rank_biserial.is_finite() {
            kv(s, "effect size", &format!("{:+.3} rank-biserial", w.rank_biserial));
        }
    }
    for note in &rep.notes {
        kv(s, "note", note);
    }
    kv(
        s,
        "condition A summary",
        &format!(
            "median {:.3} {unit}, IQR {:.3}, n {}",
            rep.summary_a.median, rep.summary_a.iqr, rep.summary_a.n
        ),
    );
    kv(
        s,
        "condition B summary",
        &format!(
            "median {:.3} {unit}, IQR {:.3}, n {}",
            rep.summary_b.median, rep.summary_b.iqr, rep.summary_b.n
        ),
    );
    let _ = writeln!(
        s,
        "\n  Trials were interleaved rather than batched, and no result was shown until\n  \
         the run finished. A positive difference means condition A scored higher."
    );
    let _ = writeln!(s);
}

fn sensor(s: &mut String, app: &App) {
    section(s, "SENSOR BEHAVIOUR");
    let st = &app.sensor;
    let mut any = false;

    if let Some(sum) = &st.cpi_summary {
        any = true;
        kv(
            s,
            "counts per inch",
            &format!(
                "{:?}: {:.0} CPI measured against {} claimed, {:+.2} %, over {} swipe(s)",
                sum.verdict,
                sum.median_cpi,
                if st.claimed_cpi.trim().is_empty() { "nothing" } else { st.claimed_cpi.trim() },
                sum.deviation * 100.0,
                sum.n_trials
            ),
        );
    }
    if let Some(r) = &st.drift {
        any = true;
        kv(
            s,
            "drift and jitter",
            &format!(
                "{:?}: {:.2} counts/s drift, {:.2} counts/s jitter over {:.0} s. {}",
                r.verdict, r.drift_cps, r.jitter_cps, r.duration_s, r.note
            ),
        );
    }
    if let Some(r) = &st.snap {
        any = true;
        let aniso = if r.aniso_applicable {
            format!("{:.3}", r.hf_aniso)
        } else {
            "not applicable, sensor too quiet".into()
        };
        kv(
            s,
            "angle snapping",
            &format!(
                "{:?}: across/along noise {aniso}, straightness {:.5}. {}",
                r.verdict, r.straightness, r.note
            ),
        );
    }
    if let Some(r) = &st.smooth {
        any = true;
        kv(
            s,
            "motion smoothing",
            &format!(
                "{:?}: {:.2} ms of motion after the stop, correlation {:+.3}. {}",
                r.verdict, r.tail_ms, r.rho1_corrected, r.note
            ),
        );
    }
    if let Some(r) = &st.tracking {
        any = true;
        let how = if r.bounded_below { "at least" } else { "failed above" };
        kv(
            s,
            "tracking speed",
            &format!(
                "{:?}: tracked {how} {:.0} inches/s, fastest reached {:.0} inches/s. {}",
                r.verdict, r.max_tracking_ips, r.peak_observed_ips, r.note
            ),
        );
    }
    if !any {
        not_measured(s, "sensor tests");
    }
    let _ = writeln!(s);
}

fn scroll(s: &mut String, app: &App) {
    section(s, "SCROLL WHEEL");
    let mut any = false;
    for r in [app.scroll.vertical.as_ref(), app.scroll.horizontal.as_ref()]
        .into_iter()
        .flatten()
    {
        any = true;
        let q = if r.continuous {
            "no detents".to_string()
        } else {
            format!("{:.0} counts/detent", r.quantum)
        };
        kv(
            s,
            r.axis.title(),
            &format!(
                "{:?}: {q}, {} up, {} down, {} reversed, {} skipped. {}",
                r.verdict, r.detents_up, r.detents_down, r.reversals, r.skips, r.note
            ),
        );
    }
    if !any {
        not_measured(s, "scroll wheel");
    }
    let _ = writeln!(s);
}
