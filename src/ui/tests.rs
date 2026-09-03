//! Headless layout guards.
//!
//! `egui::Context::run_ui` gives a real `Ui` with no window, so these run in
//! CI and on a machine with no display. They exist because two of the interface
//! requirements are testable properties rather than opinions: a numeric readout
//! must not change size when its value changes, and nothing may have rounded
//! corners.

use super::theme::{self, Level};
use super::widgets as w;

fn ctx() -> egui::Context {
    let ctx = egui::Context::default();
    theme::apply(&ctx);
    // One warm-up pass so fonts and styles are resolved before anything is
    // measured.
    let _ = ctx.run_ui(egui::RawInput::default(), |_ui| {});
    ctx
}

#[test]
fn monospace_font_has_nonzero_width() {
    let ctx = ctx();
    let mut width = 0.0;
    ctx.run_ui(egui::RawInput::default(), |ui| {
        width = w::mono_width(ui, 10);
    });
    assert!(
        width > 10.0,
        "monospace glyph width collapsed to {width}; the `default_fonts` feature \
         is what supplies FontFamily::Monospace and without it every fixed-width \
         cell silently becomes zero wide"
    );
}

#[test]
fn monospace_is_actually_monospace() {
    let ctx = ctx();
    ctx.run_ui(egui::RawInput::default(), |ui| {
        let font = egui::TextStyle::Monospace.resolve(ui.style());
        let widths: Vec<f32> = "0123456789Wil.".
            chars()
            .map(|c| ui.ctx().fonts_mut(|f| f.glyph_width(&font, c)))
            .collect();
        let first = widths[0];
        for (c, wd) in "0123456789Wil.".chars().zip(&widths) {
            assert!(
                (wd - first).abs() < 0.01,
                "glyph {c:?} is {wd} wide but '0' is {first}; the body font is not monospace"
            );
        }
    });
}

#[test]
fn readout_width_does_not_depend_on_the_value() {
    let ctx = ctx();
    let mut rects = Vec::new();
    ctx.run_ui(egui::RawInput::default(), |ui| {
        for v in ["0", "7", "1234", "999999999", "42", "-1"] {
            ui.horizontal(|ui| {
                let r = w::fixed_value(ui, v, 10, Level::Info);
                rects.push((v, r.rect));
            });
        }
    });
    let (_, first) = rects[0];
    assert!(
        first.width() > 10.0,
        "the readout field collapsed to {} wide, so this test would pass vacuously",
        first.width()
    );
    for (v, r) in &rects {
        assert!(
            (r.width() - first.width()).abs() < 0.01,
            "value {v:?} produced width {} but the first produced {}",
            r.width(),
            first.width()
        );
        assert!(
            (r.left() - first.left()).abs() < 0.01,
            "value {v:?} moved the left edge to {} from {}",
            r.left(),
            first.left()
        );
    }
}

#[test]
fn label_column_is_wide_enough_to_align_values() {
    let ctx = ctx();
    let mut value_x = Vec::new();
    ctx.run_ui(egui::RawInput::default(), |ui| {
        for k in ["name", "manufacturer", "vendor / product id", "hid usage"] {
            ui.horizontal(|ui| {
                w::fixed_label(ui, k, w::LABEL_CHARS, Level::Off);
                let r = w::fixed_value(ui, "x", 4, Level::Info);
                value_x.push((k, r.rect.left()));
            });
        }
    });
    let (_, first) = value_x[0];
    assert!(
        first > 100.0,
        "the key column collapsed to {first} px, so this test would pass vacuously"
    );
    for (k, x) in &value_x {
        assert!(
            (x - first).abs() < 0.01,
            "value after key {k:?} starts at x={x} but the first starts at x={first}; \
             the key column is not fixed width"
        );
    }
}

#[test]
fn a_value_wider_than_its_field_is_clipped_not_overdrawn() {
    let ctx = ctx();
    let mut narrow = egui::Rect::NOTHING;
    ctx.run_ui(egui::RawInput::default(), |ui| {
        ui.horizontal(|ui| {
            narrow = w::fixed_value(ui, "123456789012345678", 4, Level::Info).rect;
        });
    });
    let mut expect = 0.0;
    ctx.run_ui(egui::RawInput::default(), |ui| {
        expect = w::mono_width(ui, 4);
    });
    assert!(expect > 10.0, "field width collapsed to {expect}");
    assert!(
        (narrow.width() - expect).abs() < 0.01,
        "an over-long value grew its field to {} instead of staying at {expect}",
        narrow.width()
    );
}

#[test]
fn nothing_has_rounded_corners() {
    let ctx = ctx();
    let style = ctx.style_of(egui::Theme::Dark);
    let v = &style.visuals;
    assert_eq!(v.window_corner_radius, egui::CornerRadius::ZERO);
    assert_eq!(v.menu_corner_radius, egui::CornerRadius::ZERO);
    for (name, wv) in [
        ("noninteractive", &v.widgets.noninteractive),
        ("inactive", &v.widgets.inactive),
        ("hovered", &v.widgets.hovered),
        ("active", &v.widgets.active),
        ("open", &v.widgets.open),
    ] {
        assert_eq!(
            wv.corner_radius,
            egui::CornerRadius::ZERO,
            "{name} widgets are rounded"
        );
        assert_eq!(wv.expansion, 0.0, "{name} widgets grow on hover");
    }
}

#[test]
fn every_text_style_is_monospace() {
    let ctx = ctx();
    let style = ctx.style_of(egui::Theme::Dark);
    for (ts, id) in &style.text_styles {
        assert_eq!(
            id.family,
            egui::FontFamily::Monospace,
            "text style {ts:?} is not monospace"
        );
    }
}

#[test]
fn state_colours_are_distinct_and_everything_else_is_grey() {
    // Colour is only allowed to mean pass, fail or warning.
    for c in [theme::BLACK, theme::GREY_DIM, theme::GREY_MID, theme::GREY_HI,
              theme::GREY_LINE, theme::GREY_TEXT, theme::WHITE] {
        let [r, g, b, _] = c.to_array();
        assert!(
            r == g && g == b,
            "{c:?} is not a neutral grey; colour is reserved for state"
        );
    }
    assert_ne!(theme::PASS, theme::FAIL);
    assert_ne!(theme::PASS, theme::WARN);
    assert_ne!(theme::WARN, theme::FAIL);
}

