// One backend per OS, both exposing the same API: call sites use `platform::` only.
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
use self::linux as backend;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
use self::windows as backend;

pub(crate) use backend::{Display, Monitor, audio_outputs, autostart, crash_handler, default_audio, displays, hide_console, monitors, set_active, set_autostart, set_default_audio, set_vibrance, set_volume, vibrance, volume};

#[cfg(test)]
mod tests {
    use super::*;

    /// Reads NVIDIA Digital Vibrance of every active output, writes +1 and back: `cargo test -- --ignored vib --nocapture`
    #[test]
    #[ignore]
    fn vibrance_probe() {
        for display in displays().unwrap().iter().filter(|display| display.active) {
            let level = vibrance(&display.gdi);
            println!("{} {} -> {level:?}", display.name, display.gdi);
            // write +1 and back: proves SetDVCLevelEx round-trips without a visible change
            if let Ok(level) = level {
                let nudged = if level < 100 { level + 1 } else { level - 1 };
                set_vibrance(&display.gdi, nudged).unwrap();
                assert_eq!(vibrance(&display.gdi).unwrap(), nudged);
                set_vibrance(&display.gdi, level).unwrap();
                assert_eq!(vibrance(&display.gdi).unwrap(), level);
            }
        }
    }

    /// Touches the real HKCU\...\Run / ~/.config/autostart: `cargo test -- --ignored autostart`
    #[test]
    #[ignore]
    fn autostart() {
        set_autostart(true).unwrap();
        assert!(super::autostart());
        set_autostart(false).unwrap();
        assert!(!super::autostart());
    }
}
