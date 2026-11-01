use eframe::egui;
use eframe::egui::{Color32, Stroke};
use std::path::PathBuf;
use std::sync::Arc;

// Look ported from the Claude Design canvas "monctl UI" (https://claude.ai/artifact/Rf1HUMVeKckdNW75yiJLVQ).
pub(super) const BACKGROUND: Color32 = Color32::from_rgb(0x0E, 0x10, 0x14);
pub(super) const HEADER: Color32 = Color32::from_rgb(0x12, 0x15, 0x1B);
pub(super) const CARD: Color32 = Color32::from_rgb(0x16, 0x1A, 0x21);
pub(super) const CARD_LINE: Color32 = Color32::from_rgb(0x25, 0x2B, 0x36);
pub(super) const CONTROL: Color32 = Color32::from_rgb(0x1D, 0x22, 0x2B);
pub(super) const CONTROL_LINE: Color32 = Color32::from_rgb(0x2C, 0x33, 0x40);
pub(super) const TEXT: Color32 = Color32::from_rgb(0xE7, 0xEA, 0xF0);
pub(super) const SOFT: Color32 = Color32::from_rgb(0xCF, 0xD5, 0xDF);
pub(super) const MUTED: Color32 = Color32::from_rgb(0x8C, 0x95, 0xA6);
pub(super) const LABEL: Color32 = Color32::from_rgb(0x7F, 0x88, 0x99);
pub(super) const ACCENT: Color32 = Color32::from_rgb(0x45, 0xD0, 0xC1);
pub(super) const INK: Color32 = Color32::from_rgb(0x06, 0x27, 0x23);
pub(super) const CHIP: Color32 = Color32::from_rgb(0x17, 0x3A, 0x37);
pub(super) const CHIP_TEXT: Color32 = Color32::from_rgb(0x7F, 0xE3, 0xD8);
pub(super) const AMBER: Color32 = Color32::from_rgb(0xF5, 0xB9, 0x4A);

pub(super) fn bold() -> egui::FontFamily {
    egui::FontFamily::Name("bold".into())
}

/// Fonts (Segoe UI on Windows, closest to the design's Inter; egui's own elsewhere) + dark theme.
pub(super) fn setup_look(context: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let fonts_folder = PathBuf::from(std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into())).join("Fonts");
    for (name, file) in [("ui", "segoeui.ttf"), ("ui-bold", "seguisb.ttf"), ("mono", "consola.ttf")] {
        if let Ok(bytes) = std::fs::read(fonts_folder.join(file)) {
            fonts.font_data.insert(name.into(), Arc::new(egui::FontData::from_owned(bytes)));
        }
    }
    let has_font = |name: &str| fonts.font_data.contains_key(name);
    let (has_ui_font, has_bold_font, has_monospace_font) = (has_font("ui"), has_font("ui-bold"), has_font("mono"));
    let family_fonts = |family: egui::FontFamily, fonts: &mut egui::FontDefinitions| fonts.families.entry(family).or_default().clone();
    let mut proportional = family_fonts(egui::FontFamily::Proportional, &mut fonts);
    if has_ui_font {
        proportional.insert(0, "ui".into());
    }
    let mut bold_fonts = proportional.clone();
    if has_bold_font {
        bold_fonts.insert(0, "ui-bold".into());
    }
    let mut monospace = family_fonts(egui::FontFamily::Monospace, &mut fonts);
    if has_monospace_font {
        monospace.insert(0, "mono".into());
    }
    fonts.families.insert(egui::FontFamily::Proportional, proportional);
    fonts.families.insert(egui::FontFamily::Monospace, monospace);
    fonts.families.insert(bold(), bold_fonts);
    context.set_fonts(fonts);

    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = BACKGROUND;
    visuals.window_fill = CONTROL;
    visuals.window_stroke = Stroke::new(1.0, CONTROL_LINE);
    visuals.extreme_bg_color = Color32::from_rgb(0x2A, 0x31, 0x3D);
    visuals.selection.bg_fill = ACCENT;
    visuals.selection.stroke = Stroke::new(1.0, INK);
    visuals.slider_trailing_fill = true;
    visuals.handle_shape = egui::style::HandleShape::Circle;
    let corner_radius = egui::CornerRadius::same(8);
    for (widget, fill, line) in [
        (&mut visuals.widgets.inactive, CONTROL, CONTROL_LINE),
        (&mut visuals.widgets.hovered, Color32::from_rgb(0x23, 0x29, 0x35), Color32::from_rgb(0x3B, 0x44, 0x54)),
        (&mut visuals.widgets.active, Color32::from_rgb(0x2A, 0x31, 0x40), ACCENT),
        (&mut visuals.widgets.open, CONTROL, ACCENT),
    ] {
        (widget.bg_fill, widget.weak_bg_fill, widget.bg_stroke, widget.fg_stroke, widget.corner_radius) = (fill, fill, Stroke::new(1.0, line), Stroke::new(1.5, TEXT), corner_radius);
    }
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, CARD_LINE);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, SOFT);
    for theme in [egui::Theme::Dark, egui::Theme::Light] {
        context.set_visuals_of(theme, visuals.clone());
    }
    context.all_styles_mut(|style| {
        style.spacing.item_spacing = egui::vec2(8.0, 5.0);
        style.spacing.button_padding = egui::vec2(10.0, 4.0);
        style.spacing.interact_size.y = 28.0;
        style.text_styles.insert(egui::TextStyle::Body, egui::FontId::proportional(14.0));
        style.text_styles.insert(egui::TextStyle::Button, egui::FontId::proportional(13.5));
        // egui 0.37: a tooltip's fade-in never finishes, so the "?" help stays almost invisible.
        // With no animation it is drawn fully opaque on the first frame.
        style.animation_time = 0.0;
    });
}
