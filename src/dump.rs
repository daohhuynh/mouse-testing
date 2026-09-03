//! `--dump` prints the same survey the Device section shows, as text.
//!
//! Useful for checking a machine over SSH, for attaching to a bug report, and
//! for verifying the platform layer without a window.

use crate::app::Survey;
use crate::platform::Link;

pub fn run() {
    let first = Survey::run(None);
    let selected = first
        .devices
        .iter()
        .find(|d| d.streamable)
        .or_else(|| first.devices.first())
        .map(|d| d.key.clone());
    let s = Survey::run(selected.as_deref());

    println!("== host ==");
    println!("  os            {} ({})", s.env.os, s.env.os_build);
    println!("  architecture  {}", s.env.arch);
    println!("  processor     {}", s.env.cpu);
    println!("  cores         {}", s.env.cores);
    for (k, v) in &s.env.facts {
        println!("  {:<13} {}", k, v);
    }
    println!(
        "  clock         {}, {:.1} ns resolution, {:.1} ns per read",
        s.env.timer.name, s.env.timer.resolution_ns, s.env.timer.cost_ns
    );

    println!("\n== devices ({}) ==", s.devices.len());
    for d in &s.devices {
        let mark = if Some(&d.key) == selected.as_ref() {
            ">"
        } else {
            " "
        };
        println!("{mark} {}", d.name);
        println!("    ids         {}", d.ids());
        println!(
            "    transport   {}",
            d.transport.clone().unwrap_or_else(|| "-".into())
        );
        println!(
            "    usage       page {:?} usage {:?}",
            d.usage_page, d.usage
        );
        println!(
            "    interval    {}",
            match (d.advertised_interval_us, d.advertised_interval_trusted) {
                (Some(us), true) => format!("{us} us advertised"),
                (Some(us), false) => format!("{us} us advertised (placeholder, not trusted)"),
                (None, _) => "not reported".into(),
            }
        );
        println!(
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
            println!("    chain       {}", d.topology.chain.join(" < "));
        }
        for (k, v) in &d.extra {
            println!("    {:<11} {}", k, v);
        }
    }

    println!("\n== access ==");
    for i in &s.access.items {
        println!(
            "  [{}] {:<8} {}",
            i.state.level().tag(),
            i.tier.map(|t| t.short()).unwrap_or(""),
            i.name
        );
        for line in i.detail.lines() {
            println!("        {line}");
        }
    }

    println!("\n== measurement validity ==");
    if s.env.warnings.is_empty() {
        println!("  [PASS] nothing detected that would invalidate timing measurements");
    }
    for wn in &s.env.warnings {
        println!("  [{}] {}", wn.level.level().tag(), wn.title);
        for line in textwrap(&wn.detail, 76) {
            println!("        {line}");
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
