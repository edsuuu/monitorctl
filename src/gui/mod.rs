mod help;
mod theme;
mod widgets;
mod worker;

use crate::displays::apply;
use crate::features::model;
use crate::log::{Logged, log};
use crate::{logo, platform};
use eframe::egui;
use eframe::egui::{Color32, RichText, Stroke};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::spawn;
use std::time::{Duration, Instant};
use theme::{ACCENT, AMBER, BACKGROUND, CARD, CARD_LINE, HEADER, MUTED, SOFT, TEXT, bold, setup_look};
use widgets::{chip, controls, help_button, reload_button, save_bar, toggle};
use worker::{MonitorWorker, spawn_worker};

struct Gui {
    displays: Vec<platform::Display>,
    workers: Vec<MonitorWorker>,
    status: Arc<Mutex<String>>,
    busy: Arc<AtomicBool>,
    reload: bool,
    styled: bool,
    /// (deadline, previous outputs): set when a change turned off every output you were looking at
    revert: Arc<Mutex<Option<(Instant, Vec<String>)>>>,
    audio: Vec<(String, String)>,
    default_audio: String,
    volume: f32,
    autostart: bool,
    /// started with --min (autostart): minimize on the first frame
    start_minimized: bool,
    /// header logo texture, made once
    logo: Option<egui::TextureHandle>,
}

impl Gui {
    fn reload(&mut self, context: &egui::Context) {
        self.displays = platform::displays().logged("listar telas").unwrap_or_default();
        // one worker per active display, with its DDC/CI handle when there is one
        let mut ddc_monitors = platform::monitors();
        self.workers = self
            .displays
            .iter()
            .filter(|display| display.active)
            .map(|display| {
                let monitor = ddc_monitors.iter().position(|monitor| monitor.name == display.name).map(|index| ddc_monitors.swap_remove(index));
                spawn_worker(monitor, display.name.clone(), display.gdi.clone(), context.clone())
            })
            .collect();
        self.reload_audio();
        self.reload = false;
    }

    fn reload_audio(&mut self) {
        self.audio = platform::audio_outputs().logged("listar áudio").unwrap_or_default();
        self.default_audio = platform::default_audio().logged("áudio padrão").unwrap_or_default();
        self.volume = platform::volume().logged("ler volume").map_or(0.0, |volume| volume * 100.0);
    }

    fn set_audio(&mut self, id: &str, name: &str) {
        if platform::set_default_audio(id).logged(&format!("escolher áudio {name}")).is_ok() {
            log(format!("áudio → {name}"));
            self.reload_audio();
        }
    }

    /// Monitors of a display that answer DDC/CI (or are still loading). A TV that only shows up
    /// in dxva2 but never answers is treated as "no image controls", not as an error card.
    fn ddc_workers<'gui>(&'gui self, name: &'gui str) -> impl Iterator<Item = &'gui MonitorWorker> + 'gui {
        self.workers.iter().filter(move |worker| worker.name == name && !matches!(&*worker.state.lock().unwrap(), Err(error) if error != "carregando…"))
    }

    /// The display has monitor controls (DDC/CI), not only the GPU saturation.
    fn has_ddc(&self, name: &str) -> bool {
        self.workers.iter().any(|worker| worker.name == name && worker.has_ddc())
    }

    /// Audio endpoint that belongs to a display (TV over HDMI), matched by name.
    fn audio_of(&self, display: &str) -> Option<(String, String)> {
        let key = display.to_lowercase();
        self.audio.iter().find(|audio_output| !key.is_empty() && audio_output.1.to_lowercase().contains(&key)).cloned()
    }

    fn header(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let logo = self.logo.get_or_insert_with(|| {
                let image = egui::ColorImage::from_rgba_unmultiplied([64, 64], &logo::pixels(64));
                ui.ctx().load_texture("logo", image, egui::TextureOptions::LINEAR)
            });
            ui.add(egui::Image::new(egui::load::SizedTexture::new(logo.id(), [34.0, 34.0])));
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.label(RichText::new("monitorctl").size(17.0).family(bold()).color(TEXT));
                let with_ddc = self.displays.iter().filter(|display| self.has_ddc(&display.name)).count();
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("{} telas · {with_ddc} com DDC/CI", self.displays.len())).size(12.0).color(MUTED));
                    let status = self.status.lock().unwrap().clone();
                    if !status.is_empty() {
                        let status_color = if self.busy.load(Ordering::Relaxed) {
                            AMBER
                        } else if status.starts_with("erro") {
                            egui::Color32::from_rgb(0xF0, 0x6A, 0x6A)
                        } else {
                            ACCENT
                        };
                        ui.label(RichText::new(format!("· {status}")).size(12.0).color(status_color));
                    }
                });
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if reload_button(ui).on_hover_text("Recarregar telas, monitores e áudio").clicked() {
                    self.reload = true;
                }
                egui::Frame::new().fill(CARD).stroke(Stroke::new(1.0, CARD_LINE)).corner_radius(10).inner_margin(egui::Margin::symmetric(12, 6)).show(ui, |ui| {
                    let mut on = self.autostart;
                    let response = toggle(ui, &mut on, true, "Iniciar com o sistema").on_hover_text("Abre o monitorctl minimizado quando você entra no Windows/Linux");
                    if response.changed() && platform::set_autostart(on).logged("iniciar com o sistema").is_ok() {
                        log(format!("iniciar com o sistema: {on}"));
                        self.autostart = on;
                    }
                    ui.label(RichText::new("Iniciar com o sistema").size(12.5).color(MUTED));
                });
                egui::Frame::new().fill(CARD).stroke(Stroke::new(1.0, CARD_LINE)).corner_radius(10).inner_margin(egui::Margin::symmetric(12, 6)).show(ui, |ui| {
                    ui.horizontal(|ui| self.audio_bar(ui));
                });
            });
        });
    }

    /// Default output picker + system volume. Laid out right-to-left (header's right side), so in reverse.
    fn audio_bar(&mut self, ui: &mut egui::Ui) {
        help_button(ui, "Saída de áudio padrão do sistema e o volume dela.\nLigar/desligar telas nunca troca o áudio: ele só muda quando você escolhe aqui ou em \"Usar o áudio desta tela\".");
        ui.label(RichText::new(format!("{:>3.0}%", self.volume)).monospace().size(12.5).color(SOFT));
        ui.spacing_mut().slider_width = 120.0;
        let response = ui.add(egui::Slider::new(&mut self.volume, 0.0..=100.0).show_value(false));
        if response.changed() {
            _ = platform::set_volume(self.volume / 100.0).logged("volume");
        }
        if response.drag_stopped() {
            log(format!("volume → {:.0}%", self.volume));
        }
        ui.label(RichText::new("Volume").size(12.5).color(MUTED));
        let current = self.audio.iter().find(|audio_output| audio_output.0 == self.default_audio).map_or("—".to_string(), |audio_output| audio_output.1.clone());
        let mut picked = None;
        egui::ComboBox::from_id_salt("audio").selected_text(current).width(290.0).truncate().show_ui(ui, |ui| {
            for (id, name) in &self.audio {
                if ui.selectable_label(*id == self.default_audio, name).clicked() {
                    picked = Some((id.clone(), name.clone()));
                }
            }
        });
        if let Some((id, name)) = picked {
            self.set_audio(&id, &name);
        }
        ui.label(RichText::new("Saída").size(12.5).color(MUTED));
    }

    fn ui(&mut self, ui: &mut egui::Ui) {
        if !self.styled {
            setup_look(ui.ctx());
            if self.start_minimized {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Minimized(true));
            }
            self.styled = true;
        }
        // closing: unsaved monitor changes are written back before the window goes away
        if ui.ctx().input(|input| input.viewport().close_requested()) {
            std::mem::take(&mut self.workers).into_iter().for_each(MonitorWorker::close);
        }
        if self.reload && !self.busy.load(Ordering::Relaxed) {
            self.reload(ui.ctx());
        }
        egui::Panel::top("header")
            .frame(egui::Frame::new().fill(HEADER).inner_margin(egui::Margin::symmetric(20, 10)).stroke(Stroke::new(1.0, CARD_LINE)))
            .show(ui, |ui| self.header(ui));
        let (mut wanted, mut set_audio) = (None, None::<(String, String)>);
        let active: Vec<String> = self.displays.iter().filter(|display| display.active).map(|display| display.id.clone()).collect();
        egui::CentralPanel::default().frame(egui::Frame::new().fill(BACKGROUND).inner_margin(egui::Margin::same(16))).show(ui, |ui| {
            if let Some((deadline, previous)) = self.revert.lock().unwrap().clone() {
                let left = deadline.saturating_duration_since(Instant::now()).as_secs();
                egui::Frame::new().fill(Color32::from_rgb(0x2E, 0x26, 0x12)).stroke(Stroke::new(1.0, AMBER)).corner_radius(10).inner_margin(egui::Margin::symmetric(14, 8)).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(format!("Manter esta configuração de telas? Revertendo em {left}s")).color(AMBER));
                        if ui.button("Manter").clicked() {
                            *self.revert.lock().unwrap() = None;
                        }
                        if ui.button("Reverter").clicked() || left == 0 {
                            *self.revert.lock().unwrap() = None;
                            wanted = Some(previous);
                        }
                    });
                });
                ui.add_space(10.0);
                ui.ctx().request_repaint_after(Duration::from_millis(250));
            }
            // ponytail: ScrollArea only as a fallback for tiny windows; the layout is sized to fit 1080p
            egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                let busy = self.busy.load(Ordering::Relaxed);
                let has_ddc = |display: &platform::Display| self.has_ddc(&display.name);
                // cards side by side: monitors with controls get twice the width of plain outputs (TV)
                let units: f32 = self.displays.iter().map(|display| if has_ddc(display) { 2.0 } else { 1.0 }).sum();
                let gap = 14.0;
                let unit = (ui.available_width() - gap * (self.displays.len().max(1) - 1) as f32) / units.max(1.0);
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = gap;
                    for display in &self.displays {
                        let width = unit * if has_ddc(display) { 2.0 } else { 1.0 };
                        ui.allocate_ui_with_layout(egui::vec2(width, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                            ui.set_width(width);
                            let audio = self.audio_of(&display.name);
                            egui::Frame::new().fill(CARD).stroke(Stroke::new(1.0, CARD_LINE)).corner_radius(14).inner_margin(egui::Margin::same(14)).show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                let title = |ui: &mut egui::Ui| {
                                    ui.vertical(|ui| {
                                        ui.spacing_mut().item_spacing.y = 2.0;
                                        ui.horizontal_wrapped(|ui| {
                                            ui.label(RichText::new(&display.name).size(19.0).family(bold()).color(TEXT));
                                            match (&audio, display.active) {
                                                (Some(_), true) => chip(ui, "Áudio HDMI", true),
                                                (_, true) => chip(ui, "Ativa", true),
                                                _ => chip(ui, "Desligada", false),
                                            }
                                        });
                                        let vendor = model(&display.name).0;
                                        let subtitle = [display.gdi.trim_start_matches(r"\\.\"), display.kind.as_str(), vendor].iter().filter(|part| !part.is_empty()).copied().collect::<Vec<_>>().join(" · ");
                                        ui.label(RichText::new(if display.active { subtitle } else { format!("desligada · {}", display.kind) }).size(12.0).color(MUTED));
                                    });
                                };
                                let mut buttons = |ui: &mut egui::Ui| {
                                    let mut on = display.active;
                                    let last = display.active && active.len() == 1;
                                    let response = toggle(ui, &mut on, !busy && !last, &display.name);
                                    let response = if last { response.on_hover_text("Precisa ficar pelo menos uma tela ativa") } else { response.on_hover_text("Liga/desliga esta tela") };
                                    if response.changed() {
                                        let mut new_active = active.clone();
                                        if on { new_active.push(display.id.clone()) } else { new_active.retain(|id| *id != display.id) }
                                        wanted = Some(new_active);
                                    }
                                    let only_this = egui::Button::new(RichText::new("Só esta").size(12.5)).min_size(egui::vec2(0.0, 30.0));
                                    if ui.add_enabled(!busy, only_this).on_hover_text("Deixa só esta tela ligada (como o Win+P)").clicked() {
                                        wanted = Some(vec![display.id.clone()]);
                                    }
                                };
                                // narrow card (small window, TV column): buttons get their own row instead of overlapping the name
                                if ui.available_width() < 380.0 {
                                    title(ui);
                                    ui.add_space(4.0);
                                    ui.horizontal(|ui| buttons(ui));
                                } else {
                                    ui.horizontal(|ui| {
                                        title(ui);
                                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| buttons(ui));
                                    });
                                }
                                ui.add_space(4.0);
                                ui.separator();
                                // ponytail: matched by name, two identical monitors would share a card
                                let workers: Vec<&MonitorWorker> = self.ddc_workers(&display.name).collect();
                                if !display.active {
                                    ui.add_space(8.0);
                                    ui.label(RichText::new("Tela desligada. Ligue no botão acima.").color(MUTED));
                                } else if !self.has_ddc(&display.name) {
                                    ui.add_space(8.0);
                                    ui.label(RichText::new("Sem controle de imagem no monitor").size(15.0).family(bold()).color(SOFT));
                                    let gpu = if workers.is_empty() { "" } else { " A saturação abaixo é aplicada pela placa de vídeo no sinal enviado." };
                                    ui.label(RichText::new(format!("TVs normalmente não aceitam DDC/CI pelo cabo.{gpu}")).color(MUTED));
                                }
                                for worker in workers {
                                    let loading = worker.loading.load(Ordering::Relaxed);
                                    ui.add_enabled_ui(!loading, |ui| controls(ui, worker));
                                    if loading {
                                        ui.add_space(6.0);
                                        ui.horizontal(|ui| {
                                            ui.add(egui::Spinner::new().size(14.0).color(MUTED));
                                            ui.label(RichText::new("lendo o monitor…").size(12.5).color(MUTED));
                                        });
                                    }
                                    save_bar(ui, worker);
                                }
                                if let (Some((id, name)), true) = (&audio, display.active) {
                                    ui.add_space(6.0);
                                    if *id == self.default_audio {
                                        chip(ui, "Áudio ativo nesta tela", true);
                                    } else if ui.button("Usar o áudio desta tela").clicked() {
                                        set_audio = Some((id.clone(), name.clone()));
                                    }
                                }
                            });
                        });
                    }
                });
            });
        });
        if let Some((id, name)) = set_audio {
            self.set_audio(&id, &name);
        }
        if let Some(wanted) = wanted {
            let (status, busy, revert, context) = (self.status.clone(), self.busy.clone(), self.revert.clone(), ui.ctx().clone());
            busy.store(true, Ordering::Relaxed);
            *status.lock().unwrap() = "aplicando…".into();
            self.reload = true;
            let risky = !wanted.iter().any(|id| active.contains(id));
            spawn(move || {
                let result = apply(&wanted);
                if result.is_ok() && risky {
                    *revert.lock().unwrap() = Some((Instant::now() + Duration::from_secs(15), active));
                }
                *status.lock().unwrap() = result.unwrap_or_else(|error| format!("erro: {error}"));
                busy.store(false, Ordering::Relaxed);
                context.request_repaint();
            });
        }
    }
}

/// Opens the window; `start_minimized` = started by "iniciar com o sistema" (--min).
pub(crate) fn run(start_minimized: bool) {
    let mut gui = Gui {
        displays: vec![],
        workers: vec![],
        status: Default::default(),
        busy: Default::default(),
        reload: true,
        styled: false,
        revert: Default::default(),
        audio: vec![],
        default_audio: String::new(),
        volume: 0.0,
        autostart: platform::autostart(),
        start_minimized,
        logo: None,
    };
    let options = eframe::NativeOptions { viewport: egui::ViewportBuilder::default().with_inner_size([1720.0, 900.0]).with_min_inner_size([900.0, 560.0]).with_title("monitorctl").with_icon(egui::IconData { rgba: logo::pixels(128), width: 128, height: 128 }), ..Default::default() };
    let result = eframe::run_ui_native("monitorctl", options, move |ui, _| gui.ui(ui));
    if let Err(error) = result {
        log(format!("ERRO interface: {error}"));
    }
}
