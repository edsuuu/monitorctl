use super::help::{SECTIONS, help, section};
use super::theme::{ACCENT, AMBER, CARD, CARD_LINE, CHIP, CHIP_TEXT, CONTROL, CONTROL_LINE, INK, LABEL, MUTED, SOFT, bold};
use super::worker::MonitorWorker;
use crate::features::Feature;
use eframe::egui;
use eframe::egui::{Color32, RichText, Stroke};
use std::sync::atomic::Ordering;

pub(super) fn help_button(ui: &mut egui::Ui, text: &str) {
    let button = egui::Button::new(RichText::new("?").size(11.0).color(MUTED))
        .corner_radius(11)
        .min_size(egui::vec2(22.0, 22.0))
        .fill(Color32::TRANSPARENT)
        .stroke(Stroke::new(1.0, Color32::from_rgb(0x39, 0x41, 0x4F)));
    let response = ui.add_enabled(!text.is_empty(), button);
    if !text.is_empty() {
        response.on_hover_text(text);
    }
}

pub(super) fn chip(ui: &mut egui::Ui, text: &str, on: bool) {
    let (fill, color) = if on { (CHIP, CHIP_TEXT) } else { (CONTROL, MUTED) };
    egui::Frame::new().fill(fill).corner_radius(10).inner_margin(egui::Margin::symmetric(8, 2)).show(ui, |ui| {
        ui.label(RichText::new(text).size(11.0).family(bold()).color(color));
    });
}

/// iOS-style switch (from the egui demo), accent when on.
pub(super) fn toggle(ui: &mut egui::Ui, on: &mut bool, enabled: bool, label: &str) -> egui::Response {
    let (rect, mut response) = ui.allocate_exact_size(egui::vec2(44.0, 24.0), if enabled { egui::Sense::click() } else { egui::Sense::hover() });
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, enabled, *on, label));
    let position = ui.ctx().animate_bool_responsive(response.id, *on);
    let opacity = if enabled { 1.0 } else { 0.45 };
    ui.painter().rect_filled(rect, 12.0, (if *on { ACCENT } else { CONTROL_LINE }).gamma_multiply(opacity));
    let knob_x = egui::lerp((rect.left() + 12.0)..=(rect.right() - 12.0), position);
    ui.painter().circle_filled(egui::pos2(knob_x, rect.center().y), 9.0, (if *on { INK } else { SOFT }).gamma_multiply(opacity));
    response
}

/// One control row: label, "?", slider + value or combo.
fn row(ui: &mut egui::Ui, feature: &mut Feature, worker: &MonitorWorker) {
    // fixed widths from the column width, so long values truncate instead of widening the card
    let column_width = ui.available_width();
    ui.horizontal(|ui| {
        ui.set_min_height(30.0);
        let label_width = (column_width * 0.40).clamp(90.0, 160.0);
        ui.allocate_ui_with_layout(egui::vec2(label_width, 20.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.set_width(label_width);
            ui.add(egui::Label::new(RichText::new(&feature.name).color(if feature.current != feature.saved { AMBER } else { SOFT })).truncate());
        });
        help_button(ui, help(&feature.name));
        let control_width = (column_width - label_width - 22.0 - 3.0 * ui.spacing().item_spacing.x).max(60.0);
        let changed = if feature.values.is_empty() {
            ui.spacing_mut().slider_width = control_width - 36.0;
            let changed = ui.add(egui::Slider::new(&mut feature.current, 0..=feature.maximum.max(1)).show_value(false)).changed();
            ui.add_sized([28.0, 20.0], egui::Label::new(RichText::new(feature.current.to_string()).monospace().size(12.5).color(SOFT)));
            changed
        } else {
            let selected = feature.values.iter().find(|value| value.0 == feature.current).map_or(feature.current.to_string(), |value| value.1.clone());
            let mut changed = false;
            egui::ComboBox::from_id_salt((&worker.name, feature.operation)).width(control_width).truncate().selected_text(selected).show_ui(ui, |ui| {
                for (value, name) in &feature.values {
                    changed |= ui.selectable_value(&mut feature.current, *value, name).changed();
                }
            });
            changed
        };
        if changed {
            _ = worker.sender.send(Some((feature.operation, feature.current)));
        }
    });
}

/// A monitor's controls, sections split in two balanced sub-columns so everything fits without scrolling.
pub(super) fn controls(ui: &mut egui::Ui, worker: &MonitorWorker) {
    let mut state = worker.state.lock().unwrap();
    let features = match &mut *state {
        Err(error) => {
            ui.label(RichText::new(error.as_str()).color(MUTED));
            return;
        }
        Ok((features, _)) => features,
    };
    let mut sections: Vec<(&str, Vec<&mut Feature>)> = SECTIONS.iter().map(|title| (*title, vec![])).collect();
    for feature in features.iter_mut() {
        let title = section(&feature.name);
        sections.iter_mut().find(|section| section.0 == title).unwrap().1.push(feature);
    }
    sections.retain(|section| !section.1.is_empty());
    for (_, section_features) in &mut sections {
        section_features.sort_by_key(|feature| ["Brilho", "Contraste"].iter().position(|name| *name == feature.name).unwrap_or(2));
    }
    let column_count = if ui.available_width() >= 520.0 { 2 } else { 1 };
    let (mut columns, mut column_rows): (Vec<Vec<(&str, Vec<&mut Feature>)>>, Vec<usize>) = ((0..column_count).map(|_| vec![]).collect(), vec![0; column_count]);
    for section in sections {
        let shortest = (0..column_count).min_by_key(|&index| column_rows[index]).unwrap();
        column_rows[shortest] += section.1.len() + 1;
        columns[shortest].push(section);
    }
    ui.columns(column_count, |column_uis| {
        for (ui, column) in column_uis.iter_mut().zip(columns.iter_mut()) {
            for (title, section_features) in column.iter_mut() {
                ui.add_space(6.0);
                ui.label(RichText::new(title.to_uppercase()).size(11.0).family(bold()).color(LABEL));
                for feature in section_features.iter_mut() {
                    row(ui, feature, worker);
                }
            }
        }
    });
}

/// Round "reload" button with a painted ↻ arrow (font glyphs render blurry/inconsistent).
pub(super) fn reload_button(ui: &mut egui::Ui) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(38.0, 38.0), egui::Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "Recarregar"));
    let hovered = response.hovered();
    let painter = ui.painter();
    painter.rect(rect, 10.0, if hovered { CONTROL } else { CARD }, Stroke::new(1.0, if hovered { CONTROL_LINE } else { CARD_LINE }), egui::StrokeKind::Inside);
    let (center, radius, arrow_color) = (rect.center(), 7.5, if hovered { ACCENT } else { SOFT });
    let point_at = |angle: f32| center + radius * egui::vec2(angle.cos(), angle.sin());
    // clockwise arc with a gap on the right; the arrowhead points into the gap
    let (start, end) = (40f32.to_radians(), 320f32.to_radians());
    let arc: Vec<egui::Pos2> = (0..=32).map(|step| point_at(start + (end - start) * step as f32 / 32.0)).collect();
    painter.add(egui::Shape::line(arc, Stroke::new(2.0, arrow_color)));
    let (tip_base, along, outward) = (point_at(end), egui::vec2(-end.sin(), end.cos()), egui::vec2(end.cos(), end.sin()));
    let arrowhead = vec![tip_base + along * 4.5, tip_base + outward * 4.0 - along * 1.0, tip_base - outward * 4.0 - along * 1.0];
    painter.add(egui::Shape::convex_polygon(arrowhead, arrow_color, Stroke::NONE));
    response
}

/// Salvar / Desfazer: changes apply live, but only persist once saved.
pub(super) fn save_bar(ui: &mut egui::Ui, worker: &MonitorWorker) {
    if worker.loading.load(Ordering::Relaxed) || worker.state.lock().unwrap().is_err() {
        return; // still loading: nothing to save yet
    }
    let unsaved = worker.dirty();
    ui.add_space(6.0);
    ui.separator();
    ui.horizontal(|ui| {
        let text = match unsaved {
            0 => "Tudo salvo".to_string(),
            1 => "1 alteração não salva".to_string(),
            count => format!("{count} alterações não salvas"),
        };
        ui.label(RichText::new(text).size(12.5).color(if unsaved == 0 { MUTED } else { AMBER }));
        help_button(ui, "As mudanças valem na hora, mas só ficam gravadas quando você clica em Salvar.\nDesfazer volta para os valores salvos. Fechar o monitorctl sem salvar também desfaz.\nEx.: subiu o brilho para testar e não gostou → Desfazer.");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let save = egui::Button::new(RichText::new("Salvar").family(bold()).color(if unsaved > 0 { INK } else { MUTED })).fill(if unsaved > 0 { ACCENT } else { CONTROL }).min_size(egui::vec2(84.0, 30.0));
            if ui.add_enabled(unsaved > 0, save).on_hover_text("Grava as alterações no monitor").clicked() {
                worker.save();
            }
            if ui.add_enabled(unsaved > 0, egui::Button::new("Desfazer").min_size(egui::vec2(84.0, 30.0))).on_hover_text("Volta para os valores salvos").clicked() {
                worker.discard();
            }
        });
    });
}
