// Linux backend: /dev/i2c-* (DDC/CI, needs `i2c-dev` loaded + user in group `i2c`),
// xrandr / kscreen-doctor / wlr-randr (outputs), pactl (default audio; works on PipeWire too).
mod audio;
mod autostart;
mod ddc;
mod displays;
mod refresh;
mod vibrance;

pub(crate) use audio::{audio_outputs, default_audio, set_default_audio, set_volume, volume};
pub(crate) use autostart::{autostart, set_autostart};
pub(crate) use ddc::{Monitor, monitors};
pub(crate) use displays::{Display, displays, set_active};
pub(crate) use refresh::{refresh_rates, set_refresh_rate};
pub(crate) use vibrance::{set_vibrance, vibrance};

fn run(command: &str, arguments: &[&str]) -> Result<String, String> {
    let output = std::process::Command::new(command).args(arguments).output().map_err(|error| format!("{command}: {error}"))?;
    if !output.status.success() {
        return Err(format!("{command}: {}", String::from_utf8_lossy(&output.stderr).trim()));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

pub(crate) fn hide_console() {}

// ponytail: Rust panics are logged by the hook in main; native signals (SIGSEGV) aren't — add a
// sigaction handler writing to a pre-opened fd if FFI crashes ever show up on Linux.
pub(crate) fn crash_handler() {}
