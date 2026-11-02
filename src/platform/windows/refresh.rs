// Refresh rate (Hz) of an output, like Settings > Display > Advanced display.
use crate::log::log;
use windows::Win32::Graphics::Gdi::{CDS_UPDATEREGISTRY, ChangeDisplaySettingsExW, DEVMODEW, DISP_CHANGE_SUCCESSFUL, DM_DISPLAYFREQUENCY, ENUM_CURRENT_SETTINGS, ENUM_DISPLAY_SETTINGS_MODE, EnumDisplaySettingsW};
use windows::core::HSTRING;

fn display_mode(device: &HSTRING, mode: ENUM_DISPLAY_SETTINGS_MODE) -> Option<DEVMODEW> {
    let mut settings = DEVMODEW { dmSize: size_of::<DEVMODEW>() as u16, ..Default::default() };
    unsafe { EnumDisplaySettingsW(device, mode, &mut settings) }.as_bool().then_some(settings)
}

/// (current Hz, every Hz the driver offers at the current resolution and color depth).
pub(crate) fn refresh_rates(gdi: &str) -> Result<(u32, Vec<u32>), String> {
    let device = HSTRING::from(gdi);
    let current = display_mode(&device, ENUM_CURRENT_SETTINGS).ok_or(format!("{gdi}: sem modo de vídeo atual"))?;
    let mut rates: Vec<u32> = (0..)
        .map_while(|index| display_mode(&device, ENUM_DISPLAY_SETTINGS_MODE(index)))
        .filter(|mode| mode.dmPelsWidth == current.dmPelsWidth && mode.dmPelsHeight == current.dmPelsHeight && mode.dmBitsPerPel == current.dmBitsPerPel)
        .map(|mode| mode.dmDisplayFrequency)
        .collect();
    rates.sort_unstable();
    rates.dedup();
    Ok((current.dmDisplayFrequency, rates))
}

pub(crate) fn set_refresh_rate(gdi: &str, hertz: u32) -> Result<(), String> {
    let device = HSTRING::from(gdi);
    let mut mode = display_mode(&device, ENUM_CURRENT_SETTINGS).ok_or(format!("{gdi}: sem modo de vídeo atual"))?;
    mode.dmDisplayFrequency = hertz;
    mode.dmFields = DM_DISPLAYFREQUENCY;
    let status = unsafe { ChangeDisplaySettingsExW(&device, Some(&raw const mode), None, CDS_UPDATEREGISTRY, None) };
    if status != DISP_CHANGE_SUCCESSFUL {
        return Err(format!("ChangeDisplaySettingsEx({gdi}, {hertz} Hz) = {}", status.0));
    }
    log(format!("{gdi}: {hertz} Hz"));
    Ok(())
}
