//! `--selftest-hid [seconds]` exercises the capture pipeline end to end.
//!
//! macOS gates only Mouse and Keyboard HID collections behind Input
//! Monitoring, so every other HID device on the machine can be opened and
//! streamed with no permission at all. That is enough to prove the run loop,
//! the callbacks, the driver timestamps, the ring and the teardown, which is
//! everything except the mouse-specific decode.

#[cfg(target_os = "macos")]
pub fn hid(seconds: f64) {
    use crate::core::clock;
    use crate::core::sample::{Kind, Sample};
    use crate::platform::macos::capture::{HidCapture, Target};
    use std::collections::BTreeMap;
    use std::time::{Duration, Instant};

    println!("== hid capture self test ==");
    let t_start = Instant::now();
    let mut cap = HidCapture::start(Target::AnyOpenable { limit: 12 }, 1 << 16);

    // Give the run loop thread a moment to open devices and publish status.
    std::thread::sleep(Duration::from_millis(400));
    let st = cap.status();
    println!("  running        {}", st.running);
    println!("  opened         {} device(s)", st.opened);
    for n in &st.names {
        println!("    + {n}");
    }
    for (n, why) in &st.refused {
        println!("    - {n}: {why}");
    }
    if let Some(e) = &st.error {
        println!("  error          {e}");
    }

    let mut consumer = match cap.take_consumer() {
        Some(c) => c,
        None => {
            println!("  FAIL: could not take the ring consumer");
            return;
        }
    };

    let mut all: Vec<Sample> = Vec::new();
    let mut dropped_total = 0u64;
    let deadline = Instant::now() + Duration::from_secs_f64(seconds);
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
        dropped_total += cap.ring.drain(&mut consumer, &mut all);
    }
    dropped_total += cap.ring.drain(&mut consumer, &mut all);

    println!("\n  reports        {}", cap.reports_seen());
    println!("  values         {} ({} skipped as buffer elements)", cap.values_seen(), cap.values_skipped());
    println!(
        "  decoded        {} report(s) via descriptor, {} with no field map",
        cap.decoded(),
        cap.undecoded()
    );
    println!("  drained        {}", all.len());
    println!("  ring drops     {dropped_total}");

    // Per-device report timing, straight from the driver timestamps.
    let mut by_dev: BTreeMap<u64, Vec<u64>> = BTreeMap::new();
    for s in all.iter().filter(|s| s.kind == Kind::Report) {
        by_dev.entry(s.device).or_default().push(s.t);
    }
    println!("\n  per-device report timing (driver timestamps)");
    for (dev, times) in &by_dev {
        if times.len() < 3 {
            println!("    dev 0x{dev:x}: {} report(s), too few to time", times.len());
            continue;
        }
        let mut ivs: Vec<f64> = times
            .windows(2)
            .filter(|w| w[1] > w[0])
            .map(|w| clock::ticks_to_ns(w[1] - w[0]) as f64)
            .collect();
        ivs.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let med = ivs[ivs.len() / 2];
        println!(
            "    dev 0x{dev:x}: n={:<5} median interval {:>10.1} us  ({:>8.1} Hz)  min {:.1} us  max {:.1} us",
            times.len(),
            med / 1000.0,
            1e9 / med,
            ivs[0] / 1000.0,
            ivs[ivs.len() - 1] / 1000.0
        );
    }

    // Values prove IOKit's own descriptor parser is decoding fields for us.
    let mut usages: BTreeMap<(u16, u16), usize> = BTreeMap::new();
    for s in all.iter().filter(|s| s.kind == Kind::Value) {
        *usages.entry((s.page, s.usage)).or_default() += 1;
    }
    if !usages.is_empty() {
        println!("\n  decoded element usages");
        for ((page, usage), n) in usages.iter().take(12) {
            println!("    page 0x{page:02X} usage 0x{usage:02X}  x{n}");
        }
    }

    let t_stop = Instant::now();
    cap.stop();
    println!(
        "\n  clean stop     yes, joined in {:.1} ms",
        t_stop.elapsed().as_secs_f64() * 1000.0
    );
    println!("  total elapsed  {:.2} s", t_start.elapsed().as_secs_f64());
    println!(
        "  clock          {} ({} ns per tick)",
        clock::name(),
        clock::ticks_to_ns(1_000_000) as f64 / 1_000_000.0
    );
}

#[cfg(not(target_os = "macos"))]
pub fn hid(_seconds: f64) {
    println!("the hid self test is macOS specific");
}
