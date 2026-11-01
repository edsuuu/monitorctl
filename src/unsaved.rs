use crate::features::Feature;
use crate::log::{Logged, app_folder, log};
use std::path::PathBuf;

/// Unsaved changes survive a crash here: "Op\tsaved" per line, rewritten after every write.
pub(crate) fn pending_path(name: &str) -> PathBuf {
    app_folder().join(format!("pendente-{name}.txt"))
}

pub(crate) fn write_pending(name: &str, features: &[Feature]) {
    let unsaved: String = features.iter().filter(|feature| feature.current != feature.saved).map(|feature| format!("{:?}\t{}\n", feature.operation, feature.saved)).collect();
    let path = pending_path(name);
    let result = if unsaved.is_empty() { std::fs::remove_file(&path).or_else(|error| if error.kind() == std::io::ErrorKind::NotFound { Ok(()) } else { Err(error) }) } else { std::fs::write(&path, unsaved) };
    _ = result.map_err(|error| error.to_string()).logged(&format!("{} pendências", path.display()));
}

/// Baselines left by a previous run that closed without saving: those changes show up as unsaved again.
pub(crate) fn read_pending(name: &str, features: &mut [Feature]) {
    let Ok(text) = std::fs::read_to_string(pending_path(name)) else { return };
    for (operation, saved) in text.lines().filter_map(|line| line.split_once('\t')) {
        if let (Some(feature), Ok(saved)) = (features.iter_mut().find(|feature| format!("{:?}", feature.operation) == operation), saved.parse()) {
            feature.saved = saved;
            log(format!("{name}: {operation} não salvo de uma execução anterior (salvo = {saved}, atual = {})", feature.current));
        }
    }
}

/// Saved GPU saturation of a display (the driver forgets it on reboot, so it's re-applied on load).
pub(crate) fn gpu_path(name: &str) -> PathBuf {
    app_folder().join(format!("gpu-{name}.txt"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::features::Operation;

    #[test]
    fn pending_roundtrip() {
        let feature = |operation, current, saved| Feature { operation, name: String::new(), current, saved, maximum: 100, values: vec![] };
        let name = "teste-pendente";
        write_pending(name, &[feature(Operation::Vcp(0x10), 70, 20), feature(Operation::Aoc(0x51, 0), 3, 3), feature(Operation::DellQ(0xF4, 0x2F), 1, 2)]);
        let mut fresh = [feature(Operation::Vcp(0x10), 70, 70), feature(Operation::Aoc(0x51, 0), 3, 3), feature(Operation::DellQ(0xF4, 0x2F), 1, 1)];
        read_pending(name, &mut fresh);
        assert_eq!(fresh.iter().map(|loaded| loaded.saved).collect::<Vec<_>>(), [20, 3, 2]);
        write_pending(name, &fresh.map(|loaded| feature(loaded.operation, loaded.saved, loaded.saved)));
        assert!(!pending_path(name).exists(), "nothing unsaved -> file removed");
    }
}
