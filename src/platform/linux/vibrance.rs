// ---------- NVIDIA Digital Vibrance (driver proprietário no X11): saturação aplicada pela GPU ----------

use super::run;

/// Saturation of an output, 0..=100 (driver default = 50). DigitalVibrance is -1024..=1023.
pub(crate) fn vibrance(id: &str) -> Result<u16, String> {
    let text = run("nvidia-settings", &["-t", "-q", &format!("[DPY:{id}]/DigitalVibrance")])?;
    let level: f32 = text.trim().parse().map_err(|_| format!("DigitalVibrance ilegível: {text}"))?;
    Ok(((level + 1024.0) * 100.0 / 2047.0).round() as u16)
}

pub(crate) fn set_vibrance(id: &str, percent: u16) -> Result<(), String> {
    let level = (percent.min(100) as f32 * 2047.0 / 100.0 - 1024.0).round() as i32;
    run("nvidia-settings", &["-a", &format!("[DPY:{id}]/DigitalVibrance={level}")]).map(|_| ())
}
