use crate::features::{Feature, Operation, load, write};
use crate::log::{Logged, log};
use crate::platform;
use crate::unsaved::{gpu_path, pending_path, read_pending, write_pending};
use eframe::egui;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::{JoinHandle, spawn};

type MonitorState = Result<(Vec<Feature>, String), String>;

pub(super) struct MonitorWorker {
    pub(super) name: String,
    pub(super) state: Arc<Mutex<MonitorState>>,
    /// Some((op, value)) = write · None = Salvar (DDC "Save Current Settings" + GPU value to disk)
    pub(super) sender: mpsc::Sender<Option<(Operation, u16)>>,
    thread: JoinHandle<()>,
    /// the display has a DDC/CI handle (it may still not answer, like a TV)
    has_ddc_handle: bool,
    /// still reading the monitor: controls shown so far are read-only
    pub(super) loading: Arc<AtomicBool>,
}

impl MonitorWorker {
    /// Run `action` on the loaded features (no-op while loading or on error).
    fn with_features<Output: Default>(&self, action: impl FnOnce(&mut Vec<Feature>) -> Output) -> Output {
        match &mut *self.state.lock().unwrap() {
            Ok((features, _)) => action(features),
            Err(_) => Output::default(),
        }
    }

    /// Has monitor controls (DDC/CI), not just the GPU ones. While loading: whether it has a DDC handle.
    pub(super) fn has_ddc(&self) -> bool {
        match &*self.state.lock().unwrap() {
            Ok((features, _)) => features.iter().any(|feature| feature.operation != Operation::Vibrance) || (self.has_ddc_handle && self.loading.load(Ordering::Relaxed)),
            Err(error) => self.has_ddc_handle && error == "carregando…",
        }
    }

    /// Name of the active preset ("FPS"), shown in the card title. AOC with the preset off: its ECO mode.
    pub(super) fn active_preset(&self) -> Option<String> {
        self.with_features(|features| {
            let label = |name: &str| {
                let feature = features.iter().find(|feature| feature.name == name)?;
                feature.values.iter().find(|value| value.0 == feature.current).map(|value| value.1.clone())
            };
            label("Preset").filter(|preset| preset != "Desligado").or_else(|| label("Modo ECO"))
        })
    }

    pub(super) fn dirty(&self) -> usize {
        self.with_features(|features| features.iter().filter(|feature| feature.current != feature.saved).count())
    }

    /// Desfazer: write the saved values back.
    pub(super) fn discard(&self) {
        self.with_features(|features| {
            for feature in features.iter_mut().filter(|feature| feature.current != feature.saved) {
                log(format!("{}: desfazer {} {} → {}", self.name, feature.name, feature.current, feature.saved));
                feature.current = feature.saved;
                _ = self.sender.send(Some((feature.operation, feature.current)));
            }
        })
    }

    /// Salvar: current values become the baseline and the monitor commits them.
    pub(super) fn save(&self) {
        self.with_features(|features| features.iter_mut().for_each(|feature| feature.saved = feature.current));
        log(format!("{}: salvar", self.name));
        _ = self.sender.send(None);
    }

    /// Window closing: unsaved changes don't persist. Waits until the worker wrote them back.
    pub(super) fn close(self) {
        if self.dirty() > 0 {
            log(format!("{}: fechando com {} alterações não salvas, desfazendo", self.name, self.dirty()));
            self.discard();
        }
        drop(self.sender);
        _ = self.thread.join();
    }
}

fn load_gpu_feature(name: &str, gdi: &str) -> Option<Feature> {
    let mut current = platform::vibrance(gdi).logged(&format!("{name} saturação GPU")).ok()?;
    let saved = std::fs::read_to_string(gpu_path(name)).ok().and_then(|text| text.trim().parse::<u16>().ok());
    // an unsaved change from before a reload/crash stays as it is (read_pending marks it unsaved)
    let unsaved = std::fs::read_to_string(pending_path(name)).is_ok_and(|text| text.lines().any(|line| line.starts_with("Vibrance\t")));
    if let (Some(value), false) = (saved.filter(|&value| value != current), unsaved) {
        if platform::set_vibrance(gdi, value).logged(&format!("{name} reaplicar saturação GPU {value}")).is_ok() {
            log(format!("{name}: saturação GPU {current} → {value} (valor salvo)"));
            current = value;
        }
    }
    Some(Feature { operation: Operation::Vibrance, name: "Saturação (GPU)".into(), current, saved: current, maximum: 100, values: vec![] })
}

/// One worker per active display: loads DDC/CI features (if the monitor answers) plus the GPU ones,
/// then applies writes, coalescing slider drags. It's also the only thing touching that monitor,
/// which Dell's select-then-read codes require.
pub(super) fn spawn_worker(monitor: Option<platform::Monitor>, name: String, gdi: String, context: egui::Context) -> MonitorWorker {
    let state = Arc::new(Mutex::new(Err("carregando…".to_string())));
    let (sender, receiver) = mpsc::channel::<Option<(Operation, u16)>>();
    let loading = Arc::new(AtomicBool::new(true));
    let (shared_state, worker_name, has_ddc_handle, still_loading) = (state.clone(), name.clone(), monitor.is_some(), loading.clone());
    let thread = spawn(move || {
        // GPU saturation is instant: show it first, then each monitor control as soon as it is read
        let gpu_feature = load_gpu_feature(&worker_name, &gdi);
        let publish = |features: Vec<Feature>| {
            *shared_state.lock().unwrap() = Ok((features, String::new()));
            context.request_repaint();
        };
        if let Some(feature) = &gpu_feature {
            publish(vec![feature.clone()]);
        }
        let mut result = match &monitor {
            Some(monitor) => load(monitor, &mut |read_so_far| publish(read_so_far.iter().cloned().chain(gpu_feature.clone()).collect())).logged(&format!("{worker_name} carregar")),
            None => Err("sem DDC/CI".into()),
        };
        match (&mut result, gpu_feature) {
            (Ok((features, _)), Some(feature)) => features.push(feature),
            (Err(_), Some(feature)) => result = Ok((vec![feature], String::new())),
            _ => {}
        }
        if let Ok((features, _)) = &mut result {
            read_pending(&worker_name, features);
        }
        *shared_state.lock().unwrap() = result;
        still_loading.store(false, Ordering::Relaxed);
        context.request_repaint();
        while let Ok(first) = receiver.recv() {
            let (mut latest, mut save) = (BTreeMap::new(), false);
            for message in std::iter::once(first).chain(receiver.try_iter()) {
                match message {
                    Some((operation, value)) => _ = latest.insert(operation, value),
                    None => save = true,
                }
            }
            for (operation, value) in latest {
                log(format!("{worker_name}: {operation:?} = {value}"));
                let written = match (operation, &monitor) {
                    (Operation::Vibrance, _) => platform::set_vibrance(&gdi, value),
                    (_, Some(monitor)) => write(monitor, operation, value),
                    (_, None) => Err("sem DDC/CI".into()),
                };
                _ = written.logged(&format!("{worker_name} gravar {operation:?}={value}"));
            }
            let Ok((features, _)) = &*shared_state.lock().unwrap() else { continue };
            if save {
                if let (Some(monitor), true) = (&monitor, features.iter().any(|feature| feature.operation != Operation::Vibrance)) {
                    _ = monitor.save().logged(&format!("{worker_name} salvar (DDC 0x0C)"));
                }
                if let Some(feature) = features.iter().find(|feature| feature.operation == Operation::Vibrance) {
                    _ = std::fs::write(gpu_path(&worker_name), feature.saved.to_string()).map_err(|error| error.to_string()).logged(&format!("{worker_name} salvar saturação GPU"));
                }
            }
            write_pending(&worker_name, features);
        }
    });
    MonitorWorker { name, state, sender, thread, has_ddc_handle, loading }
}
