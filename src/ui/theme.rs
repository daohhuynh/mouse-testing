//! Monochrome theme. Black / grey / white only; colour is reserved for state.
//!
//! egui 0.35 renamed a lot of this surface (`Rounding` -> `CornerRadius`,
//! `ctx.style()` -> `ctx.all_styles_mut()`), so the field names here are the
//! 0.35 ones and will not compile against older egui.

use egui::{Color32, CornerRadius, FontFamily, FontId, Margin, Stroke, TextStyle, Vec2};

pub const BLACK: Color32 = Color32::from_rgb(0, 0, 0);
pub const GREY_DIM: Color32 = Color32::from_rgb(22, 22, 22);
pub const GREY_MID: Color32 = Color32::from_rgb(44, 44, 44);
pub const GREY_HI: Color32 = Color32::from_rgb(86, 86, 86);
pub const GREY_LINE: Color32 = Color32::from_rgb(62, 62, 62);
pub const GREY_TEXT: Color32 = Color32::from_rgb(150, 150, 150);
pub const WHITE: Color32 = Color32::from_rgb(236, 236, 236);

/// The only three colours in the program. State, never decoration.
pub const PASS: Color32 = Color32::from_rgb(64, 190, 96);
pub const WARN: Color32 = Color32::from_rgb(214, 168, 48);
pub const FAIL: Color32 = Color32::from_rgb(214, 72, 62);

pub const NO_ROUND: CornerRadius = CornerRadius::ZERO;

/// Severity of a readout or a finding. Drives the only colour in the UI.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Level {
    /// No judgement attached; renders white.
    Info,
    Pass,
    Warn,
    Fail,
    /// Known-unavailable. Renders dim grey, never red: a disabled capability is
    /// not a failing one.
    Off,
}

impl Level {
    pub fn color(self) -> Color32 {
        match self {
            Level::Info => WHITE,
            Level::Pass => PASS,
            Level::Warn => WARN,
            Level::Fail => FAIL,
            Level::Off => GREY_TEXT,
        }
    }

    pub fn tag(self) -> &'static str {
        match self {
            Level::Info => "----",
            Level::Pass => "PASS",
            Level::Warn => "WARN",
            Level::Fail => "FAIL",
            Level::Off => "N/A ",
        }
    }
}

pub fn apply(ctx: &egui::Context) {
    // Pin the theme so the OS switching to light mode cannot repaint us beige.
    ctx.set_theme(egui::ThemePreference::Dark);

    ctx.all_styles_mut(|style| {
        style.text_styles = [
            (TextStyle::Small, FontId::new(10.0, FontFamily::Monospace)),
            (TextStyle::Body, FontId::new(12.5, FontFamily::Monospace)),
            (TextStyle::Button, FontId::new(12.5, FontFamily::Monospace)),
            (TextStyle::Heading, FontId::new(15.0, FontFamily::Monospace)),
            (TextStyle::Monospace, FontId::new(12.5, FontFamily::Monospace)),
        ]
        .into();
        // Readouts must never wrap; a wrapped number changes the row height and
        // shifts everything below it.
        style.wrap_mode = Some(egui::TextWrapMode::Extend);

        style.spacing.item_spacing = Vec2::new(6.0, 3.0);
        style.spacing.button_padding = Vec2::new(6.0, 2.0);
        style.spacing.window_margin = Margin::same(6);
        style.spacing.menu_margin = Margin::same(4);
        style.spacing.interact_size = Vec2::new(24.0, 17.0);
        style.spacing.scroll.floating = false;
        style.spacing.scroll.bar_width = 8.0;
        style.spacing.indent = 14.0;

        let v = &mut style.visuals;
        v.override_text_color = Some(WHITE);
        v.panel_fill = BLACK;
        v.window_fill = BLACK;
        v.faint_bg_color = GREY_DIM;
        v.extreme_bg_color = Color32::from_rgb(12, 12, 12);
        v.code_bg_color = GREY_DIM;
        // egui uses these two for its own warn/error text. Keep them monochrome:
        // colour in this program means a measurement verdict, nothing else.
        v.warn_fg_color = WHITE;
        v.error_fg_color = WHITE;
        v.hyperlink_color = WHITE;
        v.window_stroke = Stroke::new(1.0, GREY_LINE);
        v.window_shadow = egui::epaint::Shadow::NONE;
        v.popup_shadow = egui::epaint::Shadow::NONE;
        v.button_frame = true;
        v.collapsing_header_frame = false;
        v.indent_has_left_vline = false;
        v.striped = false;
        v.slider_trailing_fill = false;
        v.image_loading_spinners = false;
        v.handle_shape = egui::style::HandleShape::Rect { aspect_ratio: 0.4 };
        v.selection = egui::style::Selection {
            bg_fill: GREY_MID,
            stroke: Stroke::new(1.0, WHITE),
        };

        v.window_corner_radius = NO_ROUND;
        v.menu_corner_radius = NO_ROUND;
        for w in [
            &mut v.widgets.noninteractive,
            &mut v.widgets.inactive,
            &mut v.widgets.hovered,
            &mut v.widgets.active,
            &mut v.widgets.open,
        ] {
            w.corner_radius = NO_ROUND;
            // Stops hovered widgets growing by a pixel, which would nudge
            // neighbouring readouts.
            w.expansion = 0.0;
        }

        v.widgets.noninteractive.bg_fill = BLACK;
        v.widgets.noninteractive.weak_bg_fill = BLACK;
        v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, GREY_LINE);
        v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, WHITE);
        v.widgets.inactive.bg_fill = GREY_DIM;
        v.widgets.inactive.weak_bg_fill = GREY_DIM;
        v.widgets.inactive.bg_stroke = Stroke::new(1.0, GREY_LINE);
        v.widgets.inactive.fg_stroke = Stroke::new(1.0, WHITE);
        v.widgets.hovered.bg_fill = GREY_MID;
        v.widgets.hovered.weak_bg_fill = GREY_MID;
        v.widgets.hovered.bg_stroke = Stroke::new(1.0, GREY_HI);
        v.widgets.hovered.fg_stroke = Stroke::new(1.0, WHITE);
        v.widgets.active.bg_fill = GREY_HI;
        v.widgets.active.weak_bg_fill = GREY_HI;
        v.widgets.active.bg_stroke = Stroke::new(1.0, WHITE);
        v.widgets.active.fg_stroke = Stroke::new(1.0, WHITE);
        v.widgets.open.bg_fill = GREY_MID;
        v.widgets.open.weak_bg_fill = GREY_MID;
        v.widgets.open.bg_stroke = Stroke::new(1.0, GREY_LINE);
        v.widgets.open.fg_stroke = Stroke::new(1.0, WHITE);
    });
}

pub fn panel_frame(margin: i8) -> egui::Frame {
    egui::Frame::new()
        .fill(BLACK)
        .stroke(Stroke::NONE)
        .corner_radius(NO_ROUND)
        .inner_margin(Margin::same(margin))
}
