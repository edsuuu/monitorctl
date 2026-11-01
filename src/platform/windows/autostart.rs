// ---------- iniciar com o sistema: HKCU\...\Run ----------

use super::error_text;
use windows::Win32::System::Registry::*;
use windows::core::{PCWSTR, w};

const RUN_KEY: PCWSTR = w!(r"Software\Microsoft\Windows\CurrentVersion\Run");

pub(crate) fn autostart() -> bool {
    unsafe { RegGetValueW(HKEY_CURRENT_USER, RUN_KEY, w!("monitorctl"), RRF_RT_REG_SZ, None, None, None) }.is_ok()
}

/// Points at this exe (opens minimized). ponytail: moving the exe leaves a stale entry; toggle again.
pub(crate) fn set_autostart(on: bool) -> Result<(), String> {
    let status = if on {
        let executable = std::env::current_exe().map_err(error_text)?;
        let command: Vec<u16> = format!("\"{}\" --min", executable.display()).encode_utf16().chain([0]).collect();
        unsafe { RegSetKeyValueW(HKEY_CURRENT_USER, RUN_KEY, w!("monitorctl"), REG_SZ.0, Some(command.as_ptr().cast()), (command.len() * 2) as u32) }
    } else {
        unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_KEY, w!("monitorctl")) }
    };
    status.ok().map_err(error_text)
}
