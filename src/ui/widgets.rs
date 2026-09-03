//! Plain widgets. The one rule that matters here: a numeric readout occupies a
//! rect whose size depends only on its declared width in characters, never on
//! the value currently in it, so a changing number cannot move anything.

use super::theme::{self, Level};
use egui::{Align2, RichText, Sense, TextStyle, Vec2};

/// Width of `n` monospace digits, in points.
pub fn mono_width(ui: &egui::Ui, n: usize) -> f32 {
    let font = TextStyle::Monospace.resolve(ui.style());
    // 0.35: layout needs `fonts_mut`; the immutable `fonts` view cannot lay out.
    let cw = ui.ctx().fonts_mut(|f| f.glyph_width(&font, '0'));
    cw * n as f32
}

pub fn row_height(ui: &egui::Ui) -> f32 {
    ui.text_style_height(&TextStyle::Monospace)
}

/// Right-aligned value in a rect of exactly `n_chars` width.
///
/// The text is painted straight into the reserved rect rather than through a
/// child `Ui`: a child would advance the parent cursor by its own content
/// width, which silently undoes the exact-size allocation and lets a long
/// value shove the next column sideways.
pub fn fixed_value(ui: &mut egui::Ui, value: &str, n_chars: usize, level: Level) -> egui::Response {
    cell(ui, value, n_chars, level, Align2::RIGHT_CENTER)
}

/// Left-aligned text in a fixed-width cell, so values always start at the same
/// x no matter how long the key is.
pub fn fixed_label(ui: &mut egui::Ui, text: &str, n_chars: usize, level: Level) -> egui::Response {
    cell(ui, text, n_chars, level, Align2::LEFT_CENTER)
}

fn cell(
    ui: &mut egui::Ui,
    text: &str,
    n_chars: usize,
    level: Level,
    align: Align2,
) -> egui::Response {
    let w = mono_width(ui, n_chars);
    let h = row_height(ui);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, h), Sense::hover());
    if ui.is_rect_visible(rect) {
        // painter_at clips, so a value wider than its field is truncated
        // instead of being drawn over its neighbour.
        let p = ui.painter_at(rect);
        let x = match align {
            Align2::RIGHT_CENTER => rect.right(),
            _ => rect.left(),
        };
        p.text(
            egui::pos2(x, rect.center().y),
            align,
            text,
            TextStyle::Monospace.resolve(ui.style()),
            level.color(),
        );
    }
    resp
}

/// `label   value unit` on one line, with both columns fixed so nothing moves.
/// `unit` is mandatory by design: every number in this program carries its unit.
pub fn readout(
    ui: &mut egui::Ui,
    label: &str,
    value: &str,
    value_chars: usize,
    unit: &str,
    level: Level,
) {
    ui.horizontal(|ui| {
        fixed_label(ui, label, LABEL_CHARS, Level::Info);
        fixed_value(ui, value, value_chars, level);
        if !unit.is_empty() {
            fixed_label(ui, unit, unit.chars().count().max(3), Level::Off);
        }
    });
}

pub const LABEL_CHARS: usize = 26;

/// A number for display, or a dash when there is not one.
///
/// Several statistics here are genuinely undefined on a short capture, and
/// their computed value is NaN. Printing "NaN" in a readout invites the reader
/// to treat it as a measurement that went wrong, when it means the measurement
/// was never taken.
pub fn num(v: f64, dp: usize) -> String {
    if v.is_finite() {
        format!("{v:.dp$}")
    } else {
        "-".into()
    }
}

/// The same, with a leading sign for figures where the direction is the point.
pub fn signed(v: f64, dp: usize) -> String {
    if v.is_finite() {
        format!("{v:+.dp$}")
    } else {
        "-".into()
    }
}

/// Key/value line for identifiers and facts. Values are free-form text, so this
/// one is allowed to be as wide as it needs; it is not a live number.
pub fn kv(ui: &mut egui::Ui, key: &str, value: &str) {
    kv_level(ui, key, value, Level::Info);
}

pub fn kv_level(ui: &mut egui::Ui, key: &str, value: &str, level: Level) {
    let avail = ui.available_width();
    let key_w = mono_width(ui, LABEL_CHARS);
    ui.horizontal(|ui| {
        fixed_label(ui, key, LABEL_CHARS, Level::Off);
        // Identifiers and device paths can be long; wrap them rather than
        // letting them run off the panel.
        wrapped(ui, value, (avail - key_w - 8.0).max(120.0), level.color());
    });
}

/// A wrapped block of text of an explicit width.
///
/// The width has to be passed in: `Ui::available_width` inside a horizontal
/// layout is effectively infinite, so a `Label` placed there never wraps. The
/// galley is laid out first and then allocated at its exact size, which keeps
/// a one-line value one line tall instead of inheriting a container's padding.
fn wrapped(ui: &mut egui::Ui, text: &str, width: f32, color: egui::Color32) {
    let font = TextStyle::Monospace.resolve(ui.style());
    let galley = ui
        .ctx()
        .fonts_mut(|f| f.layout(text.to_owned(), font, color, width));
    let (rect, _) = ui.allocate_exact_size(galley.size(), Sense::hover());
    if ui.is_rect_visible(rect) {
        ui.painter().galley(rect.min, galley, color);
    }
}

/// `[PASS] text`. The bracketed tag keeps the state readable without relying on
/// colour alone.
pub fn status_line(ui: &mut egui::Ui, level: Level, text: &str) {
    let avail = ui.available_width();
    let tag_w = mono_width(ui, 7);
    ui.horizontal(|ui| {
        ui.add(egui::Label::new(
            RichText::new(format!("[{}]", level.tag())).color(level.color()),
        ));
        wrapped(ui, text, (avail - tag_w - 8.0).max(120.0), theme::WHITE);
    });
}

pub fn heading(ui: &mut egui::Ui, text: &str) {
    ui.add_space(2.0);
    ui.add(egui::Label::new(
        RichText::new(text).heading().color(theme::WHITE),
    ));
    rule(ui);
}

pub fn subheading(ui: &mut egui::Ui, text: &str) {
    ui.add_space(6.0);
    ui.add(egui::Label::new(
        RichText::new(text).color(theme::GREY_TEXT),
    ));
}

/// A one-pixel horizontal line. `ui.separator()` draws a rounded, inset thing;
/// this is a rule.
pub fn rule(ui: &mut egui::Ui) {
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(w, 5.0), Sense::hover());
    let y = rect.center().y.round() + 0.5;
    ui.painter().line_segment(
        [egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)],
        egui::Stroke::new(1.0, theme::GREY_LINE),
    );
}

/// Wrapped explanatory text, indented under the line it explains. Grey, so it
/// never competes with a number.
pub fn note_indent(ui: &mut egui::Ui, indent: f32, text: &str) {
    let avail = ui.available_width();
    ui.horizontal(|ui| {
        ui.add_space(indent);
        wrapped(ui, text, (avail - indent).max(120.0), theme::GREY_TEXT);
    });
}

/// Selectable, full-width, left-aligned row for the sidebar.
pub fn nav_row(ui: &mut egui::Ui, selected: bool, text: &str) -> egui::Response {
    let w = ui.available_width();
    let h = row_height(ui) + 4.0;
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, h), Sense::click());
    if ui.is_rect_visible(rect) {
        let p = ui.painter_at(rect);
        let hovered = resp.hovered();
        let fill = if selected {
            theme::GREY_MID
        } else if hovered {
            theme::GREY_DIM
        } else {
            theme::BLACK
        };
        p.rect_filled(rect, theme::NO_ROUND, fill);
        if selected {
            // A left bar rather than a colour: selection is not a state.
            p.rect_filled(
                egui::Rect::from_min_size(rect.left_top(), Vec2::new(2.0, rect.height())),
                theme::NO_ROUND,
                theme::WHITE,
            );
        }
        let text_pos = egui::pos2(rect.left() + 8.0, rect.center().y);
        p.text(
            text_pos,
            egui::Align2::LEFT_CENTER,
            text,
            TextStyle::Button.resolve(ui.style()),
            if selected { theme::WHITE } else { theme::GREY_TEXT },
        );
    }
    resp
}

/// A selectable chip sized to its own text, for a row of choices laid out
/// side by side. `nav_row` takes the full available width, which is right for a
/// vertical list and wrong for a horizontal one: the first chip would eat the
/// row and the rest would be pushed off the panel.
/// The colour a verdict is drawn in.
///
/// Inconclusive is grey rather than amber: the theme's own note is that a
/// disabled capability is not a failing one, and a measurement that refused to
/// answer is saying nothing about the mouse.
pub fn level_of(v: crate::core::sensor::Verdict) -> Level {
    use crate::core::sensor::Verdict;
    match v {
        Verdict::Pass => Level::Pass,
        Verdict::Warn => Level::Warn,
        Verdict::Fail => Level::Fail,
        Verdict::Inconclusive => Level::Off,
    }
}

pub fn chip(ui: &mut egui::Ui, selected: bool, text: &str) -> egui::Response {
    let w = mono_width(ui, text.chars().count() + 2);
    let h = row_height(ui) + 4.0;
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, h), Sense::click());
    if ui.is_rect_visible(rect) {
        let p = ui.painter_at(rect);
        let fill = if selected {
            theme::GREY_MID
        } else if resp.hovered() {
            theme::GREY_DIM
        } else {
            theme::BLACK
        };
        p.rect_filled(rect, theme::NO_ROUND, fill);
        p.rect_stroke(
            rect,
            theme::NO_ROUND,
            egui::Stroke::new(1.0, theme::GREY_LINE),
            egui::StrokeKind::Inside,
        );
        p.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            text,
            TextStyle::Button.resolve(ui.style()),
            if selected { theme::WHITE } else { theme::GREY_TEXT },
        );
    }
    resp
}

/// Framed box used to group a block of readouts.
///
/// Always the full width of the panel. Left to size itself to its contents, the
/// frame would grow and shrink as the text inside it changed, so a box whose
/// status line went from "detents clean" to "encoder errors present" would
/// visibly resize on a state change. Fixing the width means the only thing that
/// moves when a measurement changes is the digits.
pub fn boxed<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::new()
        .fill(theme::BLACK)
        .stroke(egui::Stroke::new(1.0, theme::GREY_LINE))
        .corner_radius(theme::NO_ROUND)
        .inner_margin(egui::Margin::same(6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

/// Bar chart of a distribution.
///
/// Bins are aggregated down to at most one per pixel column before drawing.
/// Without that, a 120-bin distribution in a 90 pixel panel draws overlapping
/// one-pixel bars and silently misrepresents the shape.
pub fn histogram(
    ui: &mut egui::Ui,
    bins: &[(f64, u32)],
    height: f32,
    marks: &[(f64, &str)],
    x_label: &str,
) -> egui::Response {
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, height), Sense::hover());
    if !ui.is_rect_visible(rect) || bins.is_empty() {
        return resp;
    }
    let p = ui.painter_at(rect);
    p.rect_filled(rect, theme::NO_ROUND, theme::BLACK);
    p.rect_stroke(
        rect,
        theme::NO_ROUND,
        egui::Stroke::new(1.0, theme::GREY_LINE),
        egui::StrokeKind::Inside,
    );

    let plot = rect.shrink2(egui::vec2(1.0, 1.0));
    let cols = (plot.width().floor() as usize).max(1);
    let per_col = (bins.len() as f32 / cols as f32).ceil().max(1.0) as usize;
    let mut agg: Vec<u32> = Vec::with_capacity(cols);
    let mut i = 0;
    while i < bins.len() {
        let end = (i + per_col).min(bins.len());
        agg.push(bins[i..end].iter().map(|(_, c)| *c).max().unwrap_or(0));
        i = end;
    }
    let max = agg.iter().copied().max().unwrap_or(1).max(1) as f32;
    let bw = plot.width() / agg.len() as f32;
    for (i, &c) in agg.iter().enumerate() {
        if c == 0 {
            continue;
        }
        let h = (c as f32 / max) * plot.height();
        let x0 = plot.left() + i as f32 * bw;
        let bar = egui::Rect::from_min_max(
            egui::pos2(x0, plot.bottom() - h),
            egui::pos2(x0 + bw.max(1.0), plot.bottom()),
        );
        p.rect_filled(bar, theme::NO_ROUND, theme::GREY_HI);
    }

    // Vertical guides, e.g. at one and two nominal intervals.
    let x_max = bins.last().map(|(c, _)| *c).unwrap_or(1.0);
    let font = TextStyle::Small.resolve(ui.style());
    for (at, label) in marks {
        if *at <= 0.0 || *at > x_max {
            continue;
        }
        let x = plot.left() + (*at / x_max) as f32 * plot.width();
        p.line_segment(
            [egui::pos2(x, plot.top()), egui::pos2(x, plot.bottom())],
            egui::Stroke::new(1.0, theme::GREY_MID),
        );
        p.text(
            egui::pos2(x + 2.0, plot.top() + 1.0),
            egui::Align2::LEFT_TOP,
            label,
            font.clone(),
            theme::GREY_TEXT,
        );
    }

    if !x_label.is_empty() {
        p.text(
            egui::pos2(plot.right() - 2.0, plot.bottom() - 1.0),
            egui::Align2::RIGHT_BOTTOM,
            x_label,
            font,
            theme::GREY_TEXT,
        );
    }
    resp
}

/// A bracketed state tag, for use inline.
pub fn tag(ui: &mut egui::Ui, level: Level, text: &str) {
    ui.add(egui::Label::new(
        RichText::new(format!("[{}] {}", level.tag(), text)).color(level.color()),
    ));
}
