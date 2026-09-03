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
mod ui;

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().collect();
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
            .with_inner_size([1020.0, 680.0])
            .with_min_inner_size([760.0, 480.0])
            .with_transparent(false)
            .with_resizable(true),
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
