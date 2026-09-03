// A plain window is the whole point; there is no console output to hide.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

#[macro_use]
mod bitflags_lite;

mod app;
mod capture;
mod core;
mod dump;
mod platform;
mod screenshot;
mod selftest;
mod statscheck;
mod ui;

const HELP: &str = "\
mouse testing suite: measures what a pointing device actually does.

    mouse-testing                       run the interface
    mouse-testing --section NAME        open on a named section
                                        DEVICE POLLING CLICKS CPS A/B
                                        SENSOR SCROLL SESSION

  reporting
    --dump                              device and environment report to stdout
    --dump-to FILE                      the same, to a file
    --screenshot FILE                   render the window to a PNG and exit
    --window WIDTHxHEIGHT               window size, for a tall section

  session data
    --load-session FILE                 open a previous export
    --capture-test SECS --out FILE      capture unattended, export, verify the
                                        round trip, write a report and exit

  checks
    --selftest-hid SECS                 exercise the HID capture path (macOS)
    --stats-check IN OUT                run the statistics against a fixture
    --request-access                    ask macOS for Input Monitoring

  inspecting result views without hardware
    --ab-demo
    --sensor-demo [--sensor-test NAME]  cpi drift snap smooth tracking
    --scroll-demo

Exports go to ~/mouse-testing-exports. No administrator or root privilege is
needed for anything. On macOS the device level needs Input Monitoring; the app
says so rather than reporting zero.
";

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        print!("{HELP}");
        return Ok(());
    }
    if args.iter().any(|a| a == "--dump") {
        dump::run(None);
        return Ok(());
    }
    #[cfg(target_os = "macos")]
    if args.iter().any(|a| a == "--request-access") {
        // Registers the app with macOS and raises the Input Monitoring prompt.
        // Returns immediately with no dialog if a decision was already
        // recorded, which is why the app also shows the Settings path.
        let before = platform::macos::access::preflight_listen();
        let asked = platform::macos::access::request_listen();
        let after = platform::macos::access::preflight_listen();
        println!("input monitoring: before={before} requested={asked} after={after}");
        if !after {
            println!(
                "Not granted yet. Open System Settings > Privacy & Security > Input \
                 Monitoring, switch on \"mouse testing suite\", then launch it again."
            );
        }
        return Ok(());
    }
    if let Some(i) = args.iter().position(|a| a == "--stats-check") {
        let (a, b) = (args.get(i + 1).cloned(), args.get(i + 2).cloned());
        if let (Some(a), Some(b)) = (a, b) {
            statscheck::run(&a, &b);
        } else {
            eprintln!("usage: --stats-check <cases.json> <out.json>");
        }
        return Ok(());
    }
    if let Some(i) = args.iter().position(|a| a == "--selftest-hid") {
        let secs = args.get(i + 1).and_then(|s| s.parse().ok()).unwrap_or(6.0);
        selftest::hid(secs);
        return Ok(());
    }
    if let Some(i) = args.iter().position(|a| a == "--dump-to") {
        dump::run(args.get(i + 1).map(String::as_str));
        return Ok(());
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("mouse testing suite")
            .with_inner_size(window_size(&args))
            .with_min_inner_size([760.0, 480.0])
            .with_transparent(false)
            .with_resizable(true)
            .with_icon(app_icon()),
        renderer: eframe::Renderer::Glow,
        glow_options: eframe::egui_glow::GlowConfiguration {
            vsync: true,
            hardware_acceleration: eframe::egui_glow::HardwareAcceleration::Preferred,
            shader_version: None,
        },
        multisampling: 0,
        depth_buffer: 0,
        stencil_buffer: 0,
        // Flat monochrome: dithering would only add noise to solid fills.
        dithering: false,
        centered: true,
        ..Default::default()
    };
    let section = args
        .iter()
        .position(|a| a == "--section")
        .and_then(|i| args.get(i + 1).cloned());
    let auto = args
        .iter()
        .position(|a| a == "--capture-test")
        .and_then(|i| {
            let secs: f64 = args.get(i + 1)?.parse().ok()?;
            let out = args
                .iter()
                .position(|a| a == "--out")
                .and_then(|j| args.get(j + 1).cloned())?;
            Some((secs, out))
        });
    let ab_demo = args.iter().any(|a| a == "--ab-demo");
    let sensor_demo = args.iter().any(|a| a == "--sensor-demo");
    let scroll_demo = args.iter().any(|a| a == "--scroll-demo");
    let load_session = args
        .iter()
        .position(|a| a == "--load-session")
        .and_then(|i| args.get(i + 1).cloned());
    let sensor_test = args
        .iter()
        .position(|a| a == "--sensor-test")
        .and_then(|i| args.get(i + 1).cloned());
    let shot = std::env::args()
        .skip_while(|a| a != "--screenshot")
        .nth(1)
        .map(screenshot::Job::new);
    eframe::run_native(
        "mouse testing suite",
        options,
        Box::new(move |cc| {
            let mut app = app::App::new(cc);
            app.screenshot = shot;
            if let Some(name) = section {
                app.select_section(&name);
            }
            if ab_demo {
                app.ab_demo();
            }
            if sensor_demo {
                app.sensor_demo();
            }
            if scroll_demo {
                app.scroll_demo();
            }
            if let Some(p) = load_session.clone() {
                app.load_session(&p);
                app.select_section("SESSION");
            }
            if let Some(t) = sensor_test.clone() {
                app.select_sensor_test(&t);
            }
            if let Some((secs, out)) = auto.clone() {
                app.auto_capture = Some(app::AutoCapture {
                    seconds: secs,
                    out,
                    started: None,
                });
            }
            Ok(Box::new(app))
        }),
    )
}

/// The icon handed to the window system at startup.
///
/// This has to be set explicitly, because eframe substitutes ITS OWN icon when
/// the caller supplies none and pushes that onto the running application. On
/// macOS that call overrides the bundle icon, so the Finder showed the real
/// mark while the Dock and Command-Tab showed egui's hexagon.
///
/// macOS gets a deliberately empty icon. eframe discards an icon equal to the
/// default and then makes no call at all, which leaves the bundle's own
/// AppIcon.icns in charge; that is the better outcome, because the .icns holds
/// ten separately rendered sizes and a single bitmap handed over here would be
/// scaled to all of them.
///
/// Windows has no bundle to carry an .icns, so there the icon is compiled in.
fn app_icon() -> std::sync::Arc<egui::IconData> {
    #[cfg(target_os = "macos")]
    {
        std::sync::Arc::new(egui::IconData::default())
    }
    #[cfg(not(target_os = "macos"))]
    {
        // Generated by `sh scripts/make-icon.sh` from the same geometry as the
        // .icns, so the two can never drift apart.
        const RGBA: &[u8] = include_bytes!("../assets/icon-128.rgba");
        const SIDE: u32 = 128;
        debug_assert_eq!(RGBA.len(), (SIDE * SIDE * 4) as usize);
        std::sync::Arc::new(egui::IconData {
            rgba: RGBA.to_vec(),
            width: SIDE,
            height: SIDE,
        })
    }
}

/// Window size, overridable with `--window WIDTHxHEIGHT`. Only useful for
/// capturing a screenshot of a section taller than the default window.
fn window_size(args: &[String]) -> [f32; 2] {
    args.iter()
        .position(|a| a == "--window")
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.split_once('x'))
        .and_then(|(w, h)| Some([w.parse().ok()?, h.parse().ok()?]))
        .unwrap_or([1020.0, 680.0])
}
