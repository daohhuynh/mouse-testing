//! `--dump` prints the same survey the Device section shows, as text.
//!
//! Useful for checking a machine over SSH, for attaching to a bug report, and
//! for verifying the platform layer without a window.

use crate::app::Survey;
use crate::platform::Link;

/// Writes the report to `path`, or to stdout when `path` is None.
///
/// Writing to a file matters on macOS: the report has to be produced from
/// inside the app bundle for the permission state to be the bundle's own, and a
/// bundle launched with `open` has no terminal to print to.
pub fn run(path: Option<&str>) {
    let mut buf = String::new();
    render(&mut buf);
    match path {
        Some(p) => {
            if let Err(e) = std::fs::write(p, &buf) {
                eprintln!("could not write {p}: {e}");
            }
        }
        None => print!("{buf}"),
    }
}

fn render(out: &mut String) {
    use std::fmt::Write as _;
    macro_rules! line {
        ($($a:tt)*) => { let _ = writeln!(out, $($a)*); };
    }

    let first = Survey::run(None);
    // The same rule the interface uses, so a dump attached to a bug report
    // describes the device the app would actually have measured.
    let selected = crate::app::default_device(&first.devices);
    let s = Survey::run(selected.as_deref());

    line!("== host ==");
    line!("  os            {} ({})", s.env.os, s.env.os_build);
    line!("  architecture  {}", s.env.arch);
    line!("  processor     {}", s.env.cpu);
    line!("  cores         {}", s.env.cores);
    for (k, v) in &s.env.facts {
        line!("  {:<13} {}", k, v);
    }
    line!(
        "  clock         {}, {:.1} ns resolution, {:.1} ns per read",
        s.env.timer.name, s.env.timer.resolution_ns, s.env.timer.cost_ns
    );

    line!("\n== devices ({}) ==", s.devices.len());
    for d in &s.devices {
        let mark = if Some(&d.key) == selected.as_ref() {
            ">"
        } else {
            " "
        };
        line!("{mark} {}", d.name);
        line!("    ids         {}", d.ids());
        line!(
            "    transport   {}",
            d.transport.clone().unwrap_or_else(|| "-".into())
        );
        line!(
            "    usage       page {:?} usage {:?}",
            d.usage_page, d.usage
        );
        line!(
            "    interval    {}",
            match (d.advertised_interval_us, d.advertised_interval_trusted) {
                (Some(us), true) => format!("{us} us advertised"),
                (Some(us), false) => format!("{us} us advertised (placeholder, not trusted)"),
                (None, _) => "not reported".into(),
            }
        );
        line!(
            "    link        {}",
            match &d.topology.link {
                Link::Usb { hub_depth, speed } => format!(
                    "usb, hub depth {}, {}",
                    hub_depth
                        .map(|h| h.to_string())
                        .unwrap_or_else(|| "unknown".into()),
                    speed.clone().unwrap_or_else(|| "speed unknown".into())
                ),
                Link::Bluetooth => "bluetooth".into(),
                Link::Internal => "internal".into(),
                Link::Virtual => "software device".into(),
                Link::Unknown => "unknown".into(),
            }
        );
        if !d.topology.chain.is_empty() {
            line!("    chain       {}", d.topology.chain.join(" < "));
        }
        for (k, v) in &d.extra {
            line!("    {:<11} {}", k, v);
        }
    }

    line!("\n== access ==");
    for i in &s.access.items {
        line!(
            "  [{}] {:<8} {}",
            i.state.level().tag(),
            i.tier.map(|t| t.short()).unwrap_or(""),
            i.name
        );
        for line in i.detail.lines() {
            line!("        {line}");
        }
    }

    line!("\n== measurement validity ==");
    if s.env.warnings.is_empty() {
        line!("  [PASS] nothing detected that would invalidate timing measurements");
    }
    for wn in &s.env.warnings {
        line!("  [{}] {}", wn.level.level().tag(), wn.title);
        for line in textwrap(&wn.detail, 76) {
            line!("        {line}");
        }
    }
}

fn textwrap(s: &str, width: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut line = String::new();
    for word in s.split_whitespace() {
        if !line.is_empty() && line.len() + 1 + word.len() > width {
            out.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        out.push(line);
    }
    out
}

/// Text report of a finished capture, for unattended verification.
pub fn capture_report(app: &crate::app::App) -> String {
    use crate::capture::LevelState;
    use crate::platform::Tier;
    use std::fmt::Write as _;
    let mut o = String::new();
    let s = &app.session;
    let r = &app.poll_result;

    let _ = writeln!(o, "== capture ==");
    let _ = writeln!(o, "  elapsed        {:.2} s", s.elapsed_s());
    let _ = writeln!(
        o,
        "  descriptor     {} report(s) decoded, {} without a field map",
        s.decoded, s.undecoded
    );
    let _ = writeln!(
        o,
        "  control        {} system motion event(s) counted by the OS over this run",
        s.control_motion
    );
    if s.control_motion == 0 {
        let _ = writeln!(
            o,
            "                 nothing moved during this run, so every level reading zero \
             says nothing about whether it works"
        );
    }

    for tier in Tier::ALL {
        let series = s.tier_series(tier);
        let state = match tier {
            Tier::Device => s.device_state,
            Tier::System => s.system_state,
            Tier::App => {
                if series.total > 0 {
                    LevelState::Live
                } else {
                    LevelState::Idle
                }
            }
        };
        let sustained = if tier == Tier::App {
            s.app_hz()
        } else {
            series.sustained_hz()
        };
        let _ = writeln!(
            o,
            "\n  {:<7} {:?}  events {:<7} sustained {:>9.2} Hz  buffer losses {}",
            tier.short(),
            state,
            series.total,
            sustained,
            series.ring_drops
        );
    }
    let note = if !s.device_note.is_empty() {
        s.device_note.clone()
    } else {
        String::new()
    };
    if !note.is_empty() {
        let _ = writeln!(o, "  device note    {note}");
    }
    if !s.system_note.is_empty() {
        let _ = writeln!(o, "  system note    {}", s.system_note);
    }

    let _ = writeln!(o, "\n== device level interval analysis ==");
    let _ = writeln!(o, "  verdict        {:?}", r.verdict);
    let _ = writeln!(o, "  intervals      {} total, {} judged", r.n_intervals, r.n_analyzable);
    let _ = writeln!(o, "  nominal        {:.1} us  ({:.1} Hz)", r.nominal_ns / 1000.0, r.nominal_hz);
    let _ = writeln!(o, "  snapped        {:?}", r.snapped_hz);
    let _ = writeln!(o, "  jitter sigma   {:.2} us", r.jitter_sigma_ns / 1000.0);
    let _ = writeln!(o, "  tolerance      {:.3}", r.tol_rel);
    let _ = writeln!(o, "  reliable       nominal {}  multiples {}", r.nominal_reliable, r.multiple_classification_valid);
    let _ = writeln!(o, "  min/p50/p99    {:.1} / {:.1} / {:.1} us", r.min_ns / 1000.0, r.p50_ns / 1000.0, r.p99_ns / 1000.0);
    let _ = writeln!(o, "  p999/max       {:.1} / {:.1} us", r.p999_ns / 1000.0, r.max_ns / 1000.0);
    let _ = writeln!(o, "  normal/fast    {} / {}", r.n_normal, r.n_fast);
    let _ = writeln!(o, "  dropped        {} slot(s) in {} event(s), {:.4}%", r.n_dropped_slots, r.n_drop_events, r.drop_rate * 100.0);
    let _ = writeln!(o, "  late           {} ({:.4}%)", r.n_slow, r.slow_rate * 100.0);
    let _ = writeln!(o, "  idle           {}", r.n_idle);
    if !r.note.is_empty() {
        let _ = writeln!(o, "  note           {}", r.note);
    }

    let _ = writeln!(
        o,
        "  system split   {} of {} events were bound for another application",
        s.system_elsewhere, s.system.total
    );
    if s.background_events > 0 {
        let _ = writeln!(
            o,
            "  background     {} event(s) arrived while this app was not in the foreground",
            s.background_events
        );
    }
    if s.injected_events > 0 {
        let _ = writeln!(
            o,
            "  injected       {} event(s) were synthesised by software, not by hardware",
            s.injected_events
        );
    }

    let _ = writeln!(o, "\n== buttons ==");
    let _ = writeln!(
        o,
        "  edges          {} from {:?}",
        s.buttons.len(),
        s.button_source
    );
    let cfg = crate::core::debounce::DebounceConfig::default();
    for b in crate::core::debounce::analyze_all(&s.buttons, &cfg) {
        let _ = writeln!(
            o,
            "  button {:<3} {:?}  presses {} releases {} unmatched {}+{} doublets {} \
             gaps<15ms {} min gap {:.1} ms median gap {:.1} ms min press {:.1} ms \
             median press {:.1} ms spam {}",
            b.button,
            b.verdict,
            b.n_down,
            b.n_up,
            b.unmatched_down,
            b.unmatched_up,
            b.n_doublets,
            b.n_bounce_fail,
            b.min_gap_ms,
            b.median_gap_ms,
            b.min_dwell_ms,
            b.median_dwell_ms,
            b.spam_clicking
        );
        if !b.note.is_empty() {
            let _ = writeln!(o, "                 {}", b.note);
        }
    }
    o
}
