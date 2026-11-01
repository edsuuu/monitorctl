// ---------- NVIDIA Digital Vibrance: saturação aplicada pela GPU no sinal (vale para TV sem DDC/CI) ----------
// Fora do SDK público; IDs e struct do NvAPIWrapper (FunctionId.cs / PrivateDisplayDVCInfoEx).

use super::nvapi::query_interface;
use crate::log::log;
use std::ffi::c_void;

#[repr(C)]
#[derive(Default, Debug)]
struct VibranceLevels {
    version: u32,
    current: i32,
    minimum: i32,
    maximum: i32,
    default: i32,
}

type DisplayHandle = *mut c_void;

fn levels(gdi: &str) -> Result<(DisplayHandle, VibranceLevels), String> {
    let associated_handle: unsafe extern "C" fn(*const u8, *mut DisplayHandle) -> i32 = query_interface(0x35C29134)?;
    let get_levels: unsafe extern "C" fn(DisplayHandle, u32, *mut VibranceLevels) -> i32 = query_interface(0x0E45002D)?;
    let name = format!("{gdi}\0");
    let mut handle = std::ptr::null_mut();
    let status = unsafe { associated_handle(name.as_ptr(), &mut handle) };
    if status != 0 {
        return Err(format!("{gdi} não é uma saída NVIDIA (NvAPI_GetAssociatedNvidiaDisplayHandle = {status})"));
    }
    let mut levels = VibranceLevels { version: size_of::<VibranceLevels>() as u32 | 1 << 16, ..Default::default() };
    match unsafe { get_levels(handle, 0, &mut levels) } {
        0 => Ok((handle, levels)),
        status => Err(format!("NvAPI_GetDVCInfoEx({gdi}) = {status}")),
    }
}

/// Saturation of an output, 0..=100 (driver default = 50), by GDI name.
pub(crate) fn vibrance(gdi: &str) -> Result<u16, String> {
    let (_, levels) = levels(gdi)?;
    log(format!("{gdi} Digital Vibrance: {levels:?}"));
    Ok(((levels.current - levels.minimum) as f32 * 100.0 / (levels.maximum - levels.minimum).max(1) as f32).round() as u16)
}

pub(crate) fn set_vibrance(gdi: &str, percent: u16) -> Result<(), String> {
    let set_level: unsafe extern "C" fn(DisplayHandle, u32, *mut VibranceLevels) -> i32 = query_interface(0x4A82C2B1)?;
    let (handle, mut levels) = levels(gdi)?;
    levels.current = levels.minimum + ((levels.maximum - levels.minimum) as f32 * percent.min(100) as f32 / 100.0).round() as i32;
    match unsafe { set_level(handle, 0, &mut levels) } {
        0 => Ok(()),
        status => Err(format!("NvAPI_SetDVCLevelEx({gdi}) = {status}")),
    }
}
