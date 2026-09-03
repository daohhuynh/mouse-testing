// A plain window is the whole point; there is no console output to hide.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod dump;
mod platform;
mod screenshot;
mod ui;

fn main() -> eframe::Result {
    if std::env::args().any(|a| a == "--dump") {
        dump::run();
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
            Ok(Box::new(app))
        }),
    )
}
