// Windows backend: CCD API (display topology), dxva2 (DDC/CI), NVAPI I2C (vendor opcodes),
// Core Audio + IPolicyConfig (default output).
mod audio;
mod autostart;
mod crash;
mod ddc;
mod displays;
mod refresh;
mod nvapi;
mod vibrance;

pub(crate) use audio::{audio_outputs, default_audio, set_default_audio, set_volume, volume};
pub(crate) use autostart::{autostart, set_autostart};
pub(crate) use crash::crash_handler;
pub(crate) use ddc::{Monitor, monitors};
pub(crate) use displays::{Display, displays, set_active};
pub(crate) use refresh::{refresh_rates, set_refresh_rate};
pub(crate) use vibrance::{set_vibrance, vibrance};

fn error_text(error: impl std::fmt::Display) -> String {
    error.to_string()
}

fn wide_string(wide: &[u16]) -> String {
    String::from_utf16_lossy(&wide[..wide.iter().position(|&character| character == 0).unwrap_or(wide.len())])
}

/// Close the console window when started by double-click (we own the console alone).
pub(crate) fn hide_console() {
    use windows::Win32::System::Console::{FreeConsole, GetConsoleProcessList};
    let mut process_ids = [0u32; 2];
    if unsafe { GetConsoleProcessList(&mut process_ids) } == 1 {
        unsafe { _ = FreeConsole() };
    }
}
