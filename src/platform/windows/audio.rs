#![allow(non_snake_case)] // IPolicyConfig keeps its COM method names
// ---------- audio ----------

use super::error_text;
use std::ffi::c_void;
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::*;
use windows::Win32::System::Com::*;
use windows::core::{GUID, HRESULT, HSTRING, PCWSTR};

// Undocumented but stable since Vista; what every "switch default audio device" tool uses.
#[windows_core::interface("f8679f50-850a-41cf-9c72-430f290290c8")]
unsafe trait IPolicyConfig: windows_core::IUnknown {
    fn GetMixFormat(&self, device_id: PCWSTR, format: *mut *mut c_void) -> HRESULT;
    fn GetDeviceFormat(&self, device_id: PCWSTR, use_default: i32, format: *mut *mut c_void) -> HRESULT;
    fn ResetDeviceFormat(&self, device_id: PCWSTR) -> HRESULT;
    fn SetDeviceFormat(&self, device_id: PCWSTR, endpoint_format: *mut c_void, mix_format: *mut c_void) -> HRESULT;
    fn GetProcessingPeriod(&self, device_id: PCWSTR, use_default: i32, default_period: *mut i64, minimum_period: *mut i64) -> HRESULT;
    fn SetProcessingPeriod(&self, device_id: PCWSTR, period: *mut i64) -> HRESULT;
    fn GetShareMode(&self, device_id: PCWSTR, mode: *mut c_void) -> HRESULT;
    fn SetShareMode(&self, device_id: PCWSTR, mode: *mut c_void) -> HRESULT;
    fn GetPropertyValue(&self, device_id: PCWSTR, key: *const c_void, value: *mut c_void) -> HRESULT;
    fn SetPropertyValue(&self, device_id: PCWSTR, key: *const c_void, value: *mut c_void) -> HRESULT;
    fn SetDefaultEndpoint(&self, id: PCWSTR, role: ERole) -> HRESULT;
}
const POLICY_CONFIG_CLIENT: GUID = GUID::from_u128(0x870af99c_171d_4f9e_af0d_e63df40c2bc9);

fn enumerator() -> Result<IMMDeviceEnumerator, String> {
    unsafe {
        _ = CoInitializeEx(None, COINIT_MULTITHREADED); // already-initialized threads just keep their apartment
        CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).map_err(error_text)
    }
}

fn device_id(device: &IMMDevice) -> Result<String, String> {
    unsafe {
        let id_pointer = device.GetId().map_err(error_text)?;
        let id = id_pointer.to_string().map_err(error_text);
        CoTaskMemFree(Some(id_pointer.0 as _));
        id
    }
}

/// (endpoint id, friendly name) of every active playback device.
pub(crate) fn audio_outputs() -> Result<Vec<(String, String)>, String> {
    unsafe {
        let collection = enumerator()?.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE).map_err(error_text)?;
        let mut outputs = vec![];
        for index in 0..collection.GetCount().map_err(error_text)? {
            let device = collection.Item(index).map_err(error_text)?;
            let name = device.OpenPropertyStore(STGM_READ).and_then(|store| store.GetValue(&PKEY_Device_FriendlyName)).map_err(error_text)?;
            outputs.push((device_id(&device)?, name.to_string()));
        }
        Ok(outputs)
    }
}

pub(crate) fn default_audio() -> Result<String, String> {
    device_id(&unsafe { enumerator()?.GetDefaultAudioEndpoint(eRender, eConsole) }.map_err(error_text)?)
}

pub(crate) fn set_default_audio(id: &str) -> Result<(), String> {
    _ = enumerator()?; // COM init
    let policy_config: IPolicyConfig = unsafe { CoCreateInstance(&POLICY_CONFIG_CLIENT, None, CLSCTX_ALL) }.map_err(error_text)?;
    let id = HSTRING::from(id);
    for role in [eConsole, eMultimedia, eCommunications] {
        unsafe { policy_config.SetDefaultEndpoint(PCWSTR(id.as_ptr()), role) }.ok().map_err(error_text)?;
    }
    Ok(())
}

fn endpoint_volume() -> Result<IAudioEndpointVolume, String> {
    unsafe { enumerator()?.GetDefaultAudioEndpoint(eRender, eConsole).and_then(|device| device.Activate(CLSCTX_ALL, None)) }.map_err(error_text)
}

/// Master volume of the default output, 0.0..=1.0.
pub(crate) fn volume() -> Result<f32, String> {
    unsafe { endpoint_volume()?.GetMasterVolumeLevelScalar() }.map_err(error_text)
}

pub(crate) fn set_volume(level: f32) -> Result<(), String> {
    unsafe { endpoint_volume()?.SetMasterVolumeLevelScalar(level.clamp(0.0, 1.0), std::ptr::null()) }.map_err(error_text)
}
