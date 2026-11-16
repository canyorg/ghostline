//! Green-on-black hacker theme.

use eframe::egui::{self, Color32, CornerRadius, FontFamily, FontId, Stroke, TextStyle, Visuals};

pub const BG: Color32 = Color32::from_rgb(2, 8, 3);
pub const PANEL: Color32 = Color32::from_rgba_premultiplied(3, 18, 6, 235);
pub const GREEN: Color32 = Color32::from_rgb(0, 255, 65);
pub const MID: Color32 = Color32::from_rgb(0, 190, 50);
pub const DIM: Color32 = Color32::from_rgb(0, 120, 35);
pub const FAINT: Color32 = Color32::from_rgb(0, 70, 20);
pub const CYAN: Color32 = Color32::from_rgb(0, 230, 255);
pub const RED: Color32 = Color32::from_rgb(255, 50, 60);
pub const AMBER: Color32 = Color32::from_rgb(255, 190, 40);
pub const BUBBLE_ME: Color32 = Color32::from_rgb(0, 60, 18);
pub const BUBBLE_THEM: Color32 = Color32::from_rgb(8, 26, 12);

pub fn apply(ctx: &egui::Context) {
    let mut style = (*ctx.global_style()).clone();

    style.text_styles = [
        (TextStyle::Small, FontId::new(11.0, FontFamily::Monospace)),
        (TextStyle::Body, FontId::new(14.0, FontFamily::Monospace)),
        (
            TextStyle::Monospace,
            FontId::new(14.0, FontFamily::Monospace),
        ),
        (TextStyle::Button, FontId::new(14.0, FontFamily::Monospace)),
        (TextStyle::Heading, FontId::new(22.0, FontFamily::Monospace)),
    ]
    .into();

    let mut v = Visuals::dark();
    v.override_text_color = Some(MID);
    v.panel_fill = BG;
    v.window_fill = PANEL;
    v.window_stroke = Stroke::new(1.0, DIM);
    v.extreme_bg_color = Color32::from_rgb(0, 14, 4);
    v.faint_bg_color = Color32::from_rgb(0, 20, 6);
    v.selection.bg_fill = Color32::from_rgb(0, 90, 25);
    v.selection.stroke = Stroke::new(1.0, GREEN);
    v.hyperlink_color = GREEN;
    v.text_cursor.stroke = Stroke::new(2.0, GREEN);

    let r = CornerRadius::same(3);
    for (w, fill, stroke, fg) in [
        (&mut v.widgets.noninteractive, BG, FAINT, MID),
        (
            &mut v.widgets.inactive,
            Color32::from_rgb(0, 22, 7),
            DIM,
            MID,
        ),
        (
            &mut v.widgets.hovered,
            Color32::from_rgb(0, 45, 14),
            MID,
            GREEN,
        ),
        (
            &mut v.widgets.active,
            Color32::from_rgb(0, 80, 22),
            GREEN,
            GREEN,
        ),
        (&mut v.widgets.open, Color32::from_rgb(0, 30, 9), MID, GREEN),
    ] {
        w.bg_fill = fill;
        w.weak_bg_fill = fill;
        w.bg_stroke = Stroke::new(1.0, stroke);
        w.fg_stroke = Stroke::new(1.0, fg);
        w.corner_radius = r;
    }
    style.visuals = v;
    style.spacing.item_spacing = egui::vec2(8.0, 8.0);
    style.spacing.button_padding = egui::vec2(12.0, 6.0);
    ctx.set_style_of(egui::Theme::Dark, style.clone());
    ctx.set_style_of(egui::Theme::Light, style);
    ctx.set_theme(egui::ThemePreference::Dark);
}
