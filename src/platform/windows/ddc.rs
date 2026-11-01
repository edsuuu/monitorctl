// ---------- DDC/CI ----------

use super::nvapi::query_interface;
use super::{displays, wide_string};
use crate::ddc;
use crate::log::{Logged, log};
use std::ffi::c_void;
use std::thread::sleep;
use std::time::Duration;
use windows::Win32::Devices::Display::*;
use windows::Win32::Foundation::{HANDLE, LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO, MONITORINFOEXW};
use windows::core::BOOL;

pub(crate) struct Monitor {
    pub(crate) name: String,
    gdi: String,
    handle: HANDLE,
}

unsafe impl Send for Monitor {}

impl Drop for Monitor {
    fn drop(&mut self) {
        _ = unsafe { DestroyPhysicalMonitor(self.handle) }.map_err(|error| error.to_string()).logged(&format!("{} DestroyPhysicalMonitor", self.name));
    }
}

// ---------- raw DDC/CI over NVAPI I2C: vendor opcodes (AOC F8 E1 xx) that dxva2 can't carry ----------

/// NV_I2C_INFO_V3
#[repr(C)]
struct I2cRequest {
    version: u32,
    display_mask: u32,
    is_ddc_port: u8,
    device_address: u8,
    register_address: *const u8,
    register_size: u32,
    data: *mut u8,
    size: u32,
    speed: u32,
    speed_khz: u32,
    port_id: u8,
    is_port_id_set: u32,
}

impl Monitor {
    /// (GPU handle, NVAPI display id) of this monitor.
    fn nvapi_target(&self) -> Result<(*mut c_void, u32), String> {
        let display_id_by_name: unsafe extern "C" fn(*const u8, *mut u32) -> i32 = query_interface(0xAE457190)?; // DISP_GetDisplayIdByDisplayName
        let gpu_from_display_id: unsafe extern "C" fn(u32, *mut *mut c_void) -> i32 = query_interface(0x9EA74659)?; // SYS_GetPhysicalGpuFromDisplayId
        let (name, mut id, mut gpu) = (format!("{}\0", self.gdi), 0, std::ptr::null_mut());
        match unsafe { display_id_by_name(name.as_ptr(), &mut id) } {
            0 => {}
            status => return Err(format!("NvAPI_DISP_GetDisplayIdByDisplayName({}) = {status}", self.gdi)),
        }
        match unsafe { gpu_from_display_id(id, &mut gpu) } {
            0 => Ok((gpu, id)),
            status => Err(format!("NvAPI_SYS_GetPhysicalGpuFromDisplayId(0x{id:X}) = {status}")),
        }
    }

    fn i2c(&self, write: bool, buffer: &mut [u8]) -> Result<(), String> {
        let (gpu, id) = self.nvapi_target()?;
        let transfer: unsafe extern "C" fn(*mut c_void, *mut I2cRequest) -> i32 = query_interface(if write { 0xE812EB07 } else { 0x2FDE12C5 })?;
        let mut request = I2cRequest {
            version: size_of::<I2cRequest>() as u32 | 3 << 16,
            display_mask: id,
            is_ddc_port: 1,
            device_address: if write { 0x6E } else { 0x6F },
            register_address: std::ptr::null(),
            register_size: 0,
            data: buffer.as_mut_ptr(),
            size: buffer.len() as u32,
            speed: 0xFFFF, // deprecated field, NVAPI wants this sentinel
            speed_khz: 0,  // default
            port_id: 0,
            is_port_id_set: 0,
        };
        match unsafe { transfer(gpu, &mut request) } {
            0 => Ok(()),
            status => Err(format!("NvAPI_I2C{} = {status}", if write { "Write" } else { "Read" })),
        }
    }

    pub(crate) fn raw_get(&self, opcode: &[u8]) -> Result<(u16, u16), String> {
        let mut last_error = String::new();
        for _ in 0..3 {
            self.i2c(true, &mut ddc::frame(&[&[0x01], opcode].concat()))?;
            sleep(Duration::from_millis(100)); // G-Menu timing
            let mut reply = [0u8; 11];
            match self.i2c(false, &mut reply) {
                Ok(()) => match ddc::reply(&reply, opcode).inspect(|_| log(format!("{} i2c {opcode:02X?} -> {reply:02X?}", self.name))) {
                    Some(parsed) => return Ok(parsed),
                    None => last_error = format!("resposta inválida {reply:02X?}"),
                },
                Err(error) => last_error = error,
            }
            sleep(Duration::from_millis(50));
        }
        Err(format!("{opcode:02X?}: {last_error}"))
    }

    pub(crate) fn raw_set(&self, opcode: &[u8], value: u16) -> Result<(), String> {
        self.i2c(true, &mut ddc::frame(&[&[0x03], opcode, &value.to_be_bytes()].concat()))?;
        sleep(Duration::from_millis(100));
        Ok(())
    }
}

impl Monitor {
    pub(crate) fn get(&self, code: u8) -> Result<(u16, u16), String> {
        for _ in 0..3 {
            let (mut current, mut maximum) = (0, 0);
            if unsafe { GetVCPFeatureAndVCPFeatureReply(self.handle, code, None, &mut current, Some(&mut maximum)) } != 0 {
                return Ok((current as u16, maximum as u16));
            }
        }
        Err(format!("VCP 0x{code:02X} sem resposta"))
    }

    pub(crate) fn set(&self, code: u8, value: u16) -> Result<(), String> {
        if unsafe { SetVCPFeature(self.handle, code, value as u32) } != 0 { Ok(()) } else { Err(format!("falha ao gravar VCP 0x{code:02X}")) }
    }

    /// DDC/CI "Save Current Settings" (0x0C): commit to the monitor's NVRAM.
    pub(crate) fn save(&self) -> Result<(), String> {
        if unsafe { SaveCurrentMonitorSettings(self.handle) } != 0 { Ok(()) } else { Err("falha no comando salvar (0x0C)".into()) }
    }

    pub(crate) fn capabilities(&self) -> Result<String, String> {
        let mut length = 0;
        if unsafe { GetCapabilitiesStringLength(self.handle, &mut length) } == 0 {
            return Err("monitor não respondeu ao DDC/CI".into());
        }
        let mut buffer = vec![0u8; length as usize];
        if unsafe { CapabilitiesRequestAndCapabilitiesReply(self.handle, &mut buffer) } == 0 {
            return Err("falha ao ler capabilities".into());
        }
        Ok(String::from_utf8_lossy(&buffer).trim_end_matches('\0').to_string())
    }
}

unsafe extern "system" fn push_monitor_handle(monitor: HMONITOR, _: HDC, _: *mut RECT, handles: LPARAM) -> BOOL {
    unsafe { (*(handles.0 as *mut Vec<HMONITOR>)).push(monitor) };
    true.into()
}

pub(crate) fn monitors() -> Vec<Monitor> {
    let display_names: Vec<(String, String)> = displays().logged("listar telas").unwrap_or_default().into_iter().filter(|display| display.active).map(|display| (display.gdi, display.name)).collect();
    let mut handles: Vec<HMONITOR> = vec![];
    if !unsafe { EnumDisplayMonitors(None, None, Some(push_monitor_handle), LPARAM(&mut handles as *mut _ as isize)) }.as_bool() {
        log("ERRO EnumDisplayMonitors");
    }
    let mut found = vec![];
    for handle in handles {
        let mut monitor_info = MONITORINFOEXW::default();
        monitor_info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
        if !unsafe { GetMonitorInfoW(handle, &mut monitor_info as *mut _ as *mut MONITORINFO) }.as_bool() {
            log(format!("ERRO GetMonitorInfoW({handle:?})"));
            continue;
        }
        let gdi = wide_string(&monitor_info.szDevice);
        let name = display_names.iter().find(|(display_gdi, _)| *display_gdi == gdi).map_or(gdi.clone(), |names| names.1.clone());
        let mut count = 0;
        if let Err(error) = unsafe { GetNumberOfPhysicalMonitorsFromHMONITOR(handle, &mut count) } {
            log(format!("ERRO {name} GetNumberOfPhysicalMonitorsFromHMONITOR: {error}"));
            continue;
        }
        let mut physical_monitors = vec![PHYSICAL_MONITOR::default(); count as usize];
        if let Err(error) = unsafe { GetPhysicalMonitorsFromHMONITOR(handle, &mut physical_monitors) } {
            log(format!("ERRO {name} GetPhysicalMonitorsFromHMONITOR: {error}"));
            continue;
        }
        found.extend(physical_monitors.into_iter().map(|physical| Monitor { name: name.clone(), gdi: gdi.clone(), handle: physical.hPhysicalMonitor }));
    }
    found
}
