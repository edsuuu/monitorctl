#![allow(non_snake_case)] // IPolicyConfig keeps its COM method names
// Windows backend: CCD API (display topology), dxva2 (DDC/CI), NVAPI I2C (vendor opcodes),
// Core Audio + IPolicyConfig (default output).
use crate::{Logged, log};
use std::collections::HashSet;
use std::ffi::c_void;
use std::sync::OnceLock;
use std::thread::sleep;
use std::time::Duration;
use windows::Win32::Devices::Display::*;
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::core::{s, w};
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::Media::Audio::*;
use windows::Win32::System::Com::*;
use windows::core::{BOOL, GUID, HRESULT, HSTRING, PCWSTR};

pub struct Display {
    pub id: String,
    pub name: String,
    pub active: bool,
    pub gdi: String,
    /// connector type: HDMI, DisplayPort...
    pub kind: String,
}

fn s(e: impl std::fmt::Display) -> String {
    e.to_string()
}

fn wstr(w: &[u16]) -> String {
    String::from_utf16_lossy(&w[..w.iter().position(|&c| c == 0).unwrap_or(w.len())])
}

fn query(flags: QUERY_DISPLAY_CONFIG_FLAGS) -> Result<(Vec<DISPLAYCONFIG_PATH_INFO>, Vec<DISPLAYCONFIG_MODE_INFO>), String> {
    loop {
        let (mut np, mut nm) = (0u32, 0u32);
        unsafe { GetDisplayConfigBufferSizes(flags, &mut np, &mut nm) }.ok().map_err(s)?;
        let mut p = vec![DISPLAYCONFIG_PATH_INFO::default(); np as usize];
        let mut m = vec![DISPLAYCONFIG_MODE_INFO::default(); nm as usize];
        let r = unsafe { QueryDisplayConfig(flags, &mut np, p.as_mut_ptr(), &mut nm, m.as_mut_ptr(), None) };
        if r == ERROR_INSUFFICIENT_BUFFER {
            continue; // topology changed between the two calls
        }
        r.ok().map_err(s)?;
        p.truncate(np as usize);
        m.truncate(nm as usize);
        return Ok((p, m));
    }
}

fn tid(p: &DISPLAYCONFIG_PATH_INFO) -> String {
    let a = p.targetInfo.adapterId;
    format!("{:x}{:08x}:{}", a.HighPart, a.LowPart, p.targetInfo.id)
}

fn src_key(p: &DISPLAYCONFIG_PATH_INFO) -> (i32, u32, u32) {
    let a = p.sourceInfo.adapterId;
    (a.HighPart, a.LowPart, p.sourceInfo.id)
}

fn target_name(p: &DISPLAYCONFIG_PATH_INFO) -> String {
    let mut t = DISPLAYCONFIG_TARGET_DEVICE_NAME::default();
    t.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME;
    t.header.size = size_of::<DISPLAYCONFIG_TARGET_DEVICE_NAME>() as u32;
    t.header.adapterId = p.targetInfo.adapterId;
    t.header.id = p.targetInfo.id;
    if let r @ 1.. = unsafe { DisplayConfigGetDeviceInfo(&mut t.header) } {
        log(format!("ERRO DisplayConfigGetDeviceInfo(target {}) = {r}", p.targetInfo.id));
    }
    let n = wstr(&t.monitorFriendlyDeviceName);
    if n.is_empty() { format!("Tela {}", p.targetInfo.id) } else { n }
}

fn source_name(p: &DISPLAYCONFIG_PATH_INFO) -> String {
    let mut t = DISPLAYCONFIG_SOURCE_DEVICE_NAME::default();
    t.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME;
    t.header.size = size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32;
    t.header.adapterId = p.sourceInfo.adapterId;
    t.header.id = p.sourceInfo.id;
    if let r @ 1.. = unsafe { DisplayConfigGetDeviceInfo(&mut t.header) } {
        log(format!("ERRO DisplayConfigGetDeviceInfo(source {}) = {r}", p.sourceInfo.id));
    }
    wstr(&t.viewGdiDeviceName)
}

/// DISPLAYCONFIG_VIDEO_OUTPUT_TECHNOLOGY -> connector label
fn kind(t: i32) -> &'static str {
    match t {
        0 => "VGA",
        4 => "DVI",
        5 => "HDMI",
        10 | 11 | 18 => "DisplayPort",
        i32::MIN => "Interna",
        _ => "",
    }
}

/// Every connected output (active or not). QDC_ALL_PATHS lists active paths first.
pub fn displays() -> Result<Vec<Display>, String> {
    let (paths, _) = query(QDC_ALL_PATHS)?;
    let mut out: Vec<Display> = vec![];
    for p in paths.iter().filter(|p| p.targetInfo.targetAvailable.as_bool()) {
        let (id, active) = (tid(p), p.flags & DISPLAYCONFIG_PATH_ACTIVE != 0);
        match out.iter_mut().find(|d| d.id == id) {
            Some(d) if active && !d.active => (d.active, d.gdi) = (true, source_name(p)),
            Some(_) => {}
            None => out.push(Display { name: target_name(p), gdi: if active { source_name(p) } else { String::new() }, kind: kind(p.targetInfo.outputTechnology.0).into(), id, active }),
        }
    }
    Ok(out)
}

/// Make exactly `ids` active (like Win+P, but per output).
pub fn set_active(ids: &[String]) -> Result<(), String> {
    const INVALID: u32 = DISPLAYCONFIG_PATH_MODE_IDX_INVALID;
    let (all, modes) = query(QDC_ALL_PATHS)?;
    let (mut paths, mut new_modes, mut used) = (vec![], vec![], HashSet::new());
    // keep the wanted outputs that are already on, with their current modes
    for p in all.iter().filter(|p| p.flags & DISPLAYCONFIG_PATH_ACTIVE != 0 && ids.contains(&tid(p))) {
        let mut p = *p;
        unsafe {
            for idx in [&mut p.sourceInfo.Anonymous.modeInfoIdx, &mut p.targetInfo.Anonymous.modeInfoIdx] {
                if *idx != INVALID {
                    new_modes.push(modes[*idx as usize]);
                    *idx = new_modes.len() as u32 - 1;
                }
            }
        }
        used.insert(src_key(&p));
        paths.push(p);
    }
    // turn on the rest: any path for that target whose source isn't taken; Windows picks the mode
    for id in ids {
        if paths.iter().any(|p| tid(p) == *id) {
            continue;
        }
        let Some(p) = all.iter().find(|p| tid(p) == *id && p.targetInfo.targetAvailable.as_bool() && !used.contains(&src_key(p))) else {
            return Err(format!("sem saída livre na GPU para {id}"));
        };
        let mut p = *p;
        p.flags = DISPLAYCONFIG_PATH_ACTIVE;
        p.sourceInfo.Anonymous.modeInfoIdx = INVALID;
        p.targetInfo.Anonymous.modeInfoIdx = INVALID;
        used.insert(src_key(&p));
        paths.push(p);
    }
    if paths.is_empty() {
        return Err("nenhuma tela selecionada".into());
    }
    normalize(&mut new_modes);
    // Only turning outputs off: hand Windows the exact current modes of the ones that stay and
    // forbid changes, so their timing isn't re-set (that re-set is what makes them blink).
    // ALLOW_CHANGES only when an output is being added (its mode is unknown) or the driver refuses.
    let adding = paths.iter().any(|p| unsafe { p.targetInfo.Anonymous.modeInfoIdx } == INVALID);
    let base = SDC_APPLY | SDC_USE_SUPPLIED_DISPLAY_CONFIG | SDC_SAVE_TO_DATABASE;
    let mut tries = vec![base | SDC_ALLOW_CHANGES];
    if !adding {
        tries.insert(0, base);
    }
    let mut errs = vec![];
    for f in tries {
        let r = unsafe { SetDisplayConfig(Some(&paths), Some(&new_modes), f) };
        if r == 0 {
            log(format!("SetDisplayConfig(0x{:X}) ok", f.0));
            return Ok(());
        }
        log(format!("ERRO SetDisplayConfig(0x{:X}) = {r}", f.0));
        errs.push(r);
    }
    log("tentando topologia do banco do Windows");
    // fallback: hand Windows only the topology and let it pull modes/positions from its database
    for p in &mut paths {
        p.sourceInfo.Anonymous.modeInfoIdx = INVALID;
        p.targetInfo.Anonymous.modeInfoIdx = INVALID;
    }
    let r2 = unsafe { SetDisplayConfig(Some(&paths), None, SDC_APPLY | SDC_TOPOLOGY_SUPPLIED | SDC_ALLOW_PATH_ORDER_CHANGES) };
    if r2 == 0 { Ok(()) } else { Err(format!("SetDisplayConfig falhou ({errs:?} / {r2})")) }
}

/// Close horizontal gaps left by removed outputs and keep a primary at (0,0).
fn normalize(modes: &mut [DISPLAYCONFIG_MODE_INFO]) {
    let mut src: Vec<&mut DISPLAYCONFIG_SOURCE_MODE> = modes
        .iter_mut()
        .filter(|m| m.infoType == DISPLAYCONFIG_MODE_INFO_TYPE_SOURCE)
        .map(|m| unsafe { &mut m.Anonymous.sourceMode })
        .collect();
    src.sort_by_key(|m| m.position.x);
    let anchor = src.iter().position(|m| m.position.x == 0 && m.position.y == 0).unwrap_or(0);
    let (mut shift, mut right) = (0, None::<i32>);
    for m in src.iter_mut() {
        m.position.x -= shift;
        if let Some(r) = right.filter(|&r| m.position.x > r) {
            shift += m.position.x - r;
            m.position.x = r;
        }
        right = Some(right.unwrap_or(i32::MIN).max(m.position.x + m.width as i32));
    }
    if let Some(a) = src.get(anchor).map(|m| m.position) {
        for m in src.iter_mut() {
            m.position.x -= a.x;
            m.position.y -= a.y;
        }
    }
}

// ---------- DDC/CI ----------

pub struct Monitor {
    pub name: String,
    gdi: String,
    h: HANDLE,
}

unsafe impl Send for Monitor {}

impl Drop for Monitor {
    fn drop(&mut self) {
        _ = unsafe { DestroyPhysicalMonitor(self.h) }.map_err(|e| e.to_string()).logged(&format!("{} DestroyPhysicalMonitor", self.name));
    }
}

// ---------- raw DDC/CI over NVAPI I2C: vendor opcodes (AOC F8 E1 xx) that dxva2 can't carry ----------

type QueryInterface = unsafe extern "C" fn(u32) -> *const c_void;

/// nvapi_QueryInterface(id) as a typed fn pointer. IDs from AOC G-Menu's DDCHelperLib (monctl-rea).
fn nvapi<F: Copy>(id: u32) -> Result<F, String> {
    static QI: OnceLock<Result<usize, String>> = OnceLock::new();
    let qi = QI
        .get_or_init(|| unsafe {
            let h = LoadLibraryW(w!("nvapi64.dll")).map_err(|e| format!("nvapi64.dll (sem GPU NVIDIA?): {e}"))?;
            let qi: QueryInterface = std::mem::transmute(GetProcAddress(h, s!("nvapi_QueryInterface")).ok_or("nvapi_QueryInterface ausente")?);
            let init = qi(0x0150E828);
            if init.is_null() {
                return Err("NvAPI_Initialize ausente".into());
            }
            match std::mem::transmute::<_, unsafe extern "C" fn() -> i32>(init)() {
                0 => Ok(qi as usize),
                r => Err(format!("NvAPI_Initialize = {r}")),
            }
        })
        .clone()
        .logged("NVAPI")?;
    let p = unsafe { std::mem::transmute::<usize, QueryInterface>(qi)(id) };
    if p.is_null() {
        return Err(format!("NVAPI 0x{id:08X} indisponível"));
    }
    Ok(unsafe { std::mem::transmute_copy(&p) })
}

/// NV_I2C_INFO_V3
#[repr(C)]
struct NvI2cInfo {
    version: u32,
    display_mask: u32,
    is_ddc_port: u8,
    dev_addr: u8,
    reg_addr: *const u8,
    reg_size: u32,
    data: *mut u8,
    size: u32,
    speed: u32,
    speed_khz: u32,
    port_id: u8,
    port_set: u32,
}

impl Monitor {
    /// (GPU handle, NVAPI display id) of this monitor.
    fn nv(&self) -> Result<(*mut c_void, u32), String> {
        let by_name: unsafe extern "C" fn(*const u8, *mut u32) -> i32 = nvapi(0xAE457190)?; // DISP_GetDisplayIdByDisplayName
        let gpu_of: unsafe extern "C" fn(u32, *mut *mut c_void) -> i32 = nvapi(0x9EA74659)?; // SYS_GetPhysicalGpuFromDisplayId
        let (name, mut id, mut gpu) = (format!("{}\0", self.gdi), 0, std::ptr::null_mut());
        match unsafe { by_name(name.as_ptr(), &mut id) } {
            0 => {}
            r => return Err(format!("NvAPI_DISP_GetDisplayIdByDisplayName({}) = {r}", self.gdi)),
        }
        match unsafe { gpu_of(id, &mut gpu) } {
            0 => Ok((gpu, id)),
            r => Err(format!("NvAPI_SYS_GetPhysicalGpuFromDisplayId(0x{id:X}) = {r}")),
        }
    }

    fn i2c(&self, write: bool, buf: &mut [u8]) -> Result<(), String> {
        let (gpu, id) = self.nv()?;
        let f: unsafe extern "C" fn(*mut c_void, *mut NvI2cInfo) -> i32 = nvapi(if write { 0xE812EB07 } else { 0x2FDE12C5 })?;
        let mut info = NvI2cInfo {
            version: size_of::<NvI2cInfo>() as u32 | 3 << 16,
            display_mask: id,
            is_ddc_port: 1,
            dev_addr: if write { 0x6E } else { 0x6F },
            reg_addr: std::ptr::null(),
            reg_size: 0,
            data: buf.as_mut_ptr(),
            size: buf.len() as u32,
            speed: 0xFFFF, // deprecated field, NVAPI wants this sentinel
            speed_khz: 0,  // default
            port_id: 0,
            port_set: 0,
        };
        match unsafe { f(gpu, &mut info) } {
            0 => Ok(()),
            r => Err(format!("NvAPI_I2C{} = {r}", if write { "Write" } else { "Read" })),
        }
    }

    pub fn raw_get(&self, op: &[u8]) -> Result<(u16, u16), String> {
        let mut last = String::new();
        for _ in 0..3 {
            self.i2c(true, &mut crate::ddc_frame(&[&[0x01], op].concat()))?;
            sleep(Duration::from_millis(100)); // G-Menu timing
            let mut b = [0u8; 11];
            match self.i2c(false, &mut b) {
                Ok(()) => match crate::ddc_reply(&b, op).inspect(|_| log(format!("{} i2c {op:02X?} -> {b:02X?}", self.name))) {
                    Some(r) => return Ok(r),
                    None => last = format!("resposta inválida {b:02X?}"),
                },
                Err(e) => last = e,
            }
            sleep(Duration::from_millis(50));
        }
        Err(format!("{op:02X?}: {last}"))
    }

    pub fn raw_set(&self, op: &[u8], v: u16) -> Result<(), String> {
        self.i2c(true, &mut crate::ddc_frame(&[&[0x03], op, &v.to_be_bytes()].concat()))?;
        sleep(Duration::from_millis(100));
        Ok(())
    }
}

impl Monitor {
    pub fn get(&self, code: u8) -> Result<(u16, u16), String> {
        for _ in 0..3 {
            let (mut cur, mut max) = (0, 0);
            if unsafe { GetVCPFeatureAndVCPFeatureReply(self.h, code, None, &mut cur, Some(&mut max)) } != 0 {
                return Ok((cur as u16, max as u16));
            }
        }
        Err(format!("VCP 0x{code:02X} sem resposta"))
    }

    pub fn set(&self, code: u8, v: u16) -> Result<(), String> {
        if unsafe { SetVCPFeature(self.h, code, v as u32) } != 0 { Ok(()) } else { Err(format!("falha ao gravar VCP 0x{code:02X}")) }
    }

    /// DDC/CI "Save Current Settings" (0x0C): commit to the monitor's NVRAM.
    pub fn save(&self) -> Result<(), String> {
        if unsafe { SaveCurrentMonitorSettings(self.h) } != 0 { Ok(()) } else { Err("falha no comando salvar (0x0C)".into()) }
    }

    pub fn caps(&self) -> Result<String, String> {
        let mut n = 0;
        if unsafe { GetCapabilitiesStringLength(self.h, &mut n) } == 0 {
            return Err("monitor não respondeu ao DDC/CI".into());
        }
        let mut b = vec![0u8; n as usize];
        if unsafe { CapabilitiesRequestAndCapabilitiesReply(self.h, &mut b) } == 0 {
            return Err("falha ao ler capabilities".into());
        }
        Ok(String::from_utf8_lossy(&b).trim_end_matches('\0').to_string())
    }
}

unsafe extern "system" fn push_hmon(h: HMONITOR, _: HDC, _: *mut RECT, l: LPARAM) -> BOOL {
    unsafe { (*(l.0 as *mut Vec<HMONITOR>)).push(h) };
    true.into()
}

pub fn monitors() -> Vec<Monitor> {
    let names: Vec<(String, String)> = displays().logged("listar telas").unwrap_or_default().into_iter().filter(|d| d.active).map(|d| (d.gdi, d.name)).collect();
    let mut hs: Vec<HMONITOR> = vec![];
    if !unsafe { EnumDisplayMonitors(None, None, Some(push_hmon), LPARAM(&mut hs as *mut _ as isize)) }.as_bool() {
        log("ERRO EnumDisplayMonitors");
    }
    let mut out = vec![];
    for h in hs {
        let mut mi = MONITORINFOEXW::default();
        mi.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
        if !unsafe { GetMonitorInfoW(h, &mut mi as *mut _ as *mut MONITORINFO) }.as_bool() {
            log(format!("ERRO GetMonitorInfoW({h:?})"));
            continue;
        }
        let gdi = wstr(&mi.szDevice);
        let name = names.iter().find(|(g, _)| *g == gdi).map_or(gdi.clone(), |n| n.1.clone());
        let mut n = 0;
        if let Err(e) = unsafe { GetNumberOfPhysicalMonitorsFromHMONITOR(h, &mut n) } {
            log(format!("ERRO {name} GetNumberOfPhysicalMonitorsFromHMONITOR: {e}"));
            continue;
        }
        let mut pm = vec![PHYSICAL_MONITOR::default(); n as usize];
        if let Err(e) = unsafe { GetPhysicalMonitorsFromHMONITOR(h, &mut pm) } {
            log(format!("ERRO {name} GetPhysicalMonitorsFromHMONITOR: {e}"));
            continue;
        }
        out.extend(pm.into_iter().map(|p| Monitor { name: name.clone(), gdi: gdi.clone(), h: p.hPhysicalMonitor }));
    }
    out
}

// ---------- audio ----------

// Undocumented but stable since Vista; what every "switch default audio device" tool uses.
#[windows_core::interface("f8679f50-850a-41cf-9c72-430f290290c8")]
unsafe trait IPolicyConfig: windows_core::IUnknown {
    fn GetMixFormat(&self, a: PCWSTR, b: *mut *mut std::ffi::c_void) -> HRESULT;
    fn GetDeviceFormat(&self, a: PCWSTR, b: i32, c: *mut *mut std::ffi::c_void) -> HRESULT;
    fn ResetDeviceFormat(&self, a: PCWSTR) -> HRESULT;
    fn SetDeviceFormat(&self, a: PCWSTR, b: *mut std::ffi::c_void, c: *mut std::ffi::c_void) -> HRESULT;
    fn GetProcessingPeriod(&self, a: PCWSTR, b: i32, c: *mut i64, d: *mut i64) -> HRESULT;
    fn SetProcessingPeriod(&self, a: PCWSTR, b: *mut i64) -> HRESULT;
    fn GetShareMode(&self, a: PCWSTR, b: *mut std::ffi::c_void) -> HRESULT;
    fn SetShareMode(&self, a: PCWSTR, b: *mut std::ffi::c_void) -> HRESULT;
    fn GetPropertyValue(&self, a: PCWSTR, b: *const std::ffi::c_void, c: *mut std::ffi::c_void) -> HRESULT;
    fn SetPropertyValue(&self, a: PCWSTR, b: *const std::ffi::c_void, c: *mut std::ffi::c_void) -> HRESULT;
    fn SetDefaultEndpoint(&self, id: PCWSTR, role: ERole) -> HRESULT;
}
const POLICY_CONFIG_CLIENT: GUID = GUID::from_u128(0x870af99c_171d_4f9e_af0d_e63df40c2bc9);

fn enumerator() -> Result<IMMDeviceEnumerator, String> {
    unsafe {
        _ = CoInitializeEx(None, COINIT_MULTITHREADED); // already-initialized threads just keep their apartment
        CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).map_err(s)
    }
}

fn dev_id(d: &IMMDevice) -> Result<String, String> {
    unsafe {
        let p = d.GetId().map_err(s)?;
        let r = p.to_string().map_err(s);
        CoTaskMemFree(Some(p.0 as _));
        r
    }
}

/// (endpoint id, friendly name) of every active playback device.
pub fn audio_outputs() -> Result<Vec<(String, String)>, String> {
    unsafe {
        let c = enumerator()?.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE).map_err(s)?;
        let mut out = vec![];
        for i in 0..c.GetCount().map_err(s)? {
            let d = c.Item(i).map_err(s)?;
            let name = d.OpenPropertyStore(STGM_READ).and_then(|p| p.GetValue(&PKEY_Device_FriendlyName)).map_err(s)?;
            out.push((dev_id(&d)?, name.to_string()));
        }
        Ok(out)
    }
}

pub fn default_audio() -> Result<String, String> {
    dev_id(&unsafe { enumerator()?.GetDefaultAudioEndpoint(eRender, eConsole) }.map_err(s)?)
}

pub fn set_default_audio(id: &str) -> Result<(), String> {
    _ = enumerator()?; // COM init
    let pc: IPolicyConfig = unsafe { CoCreateInstance(&POLICY_CONFIG_CLIENT, None, CLSCTX_ALL) }.map_err(s)?;
    let id = HSTRING::from(id);
    for role in [eConsole, eMultimedia, eCommunications] {
        unsafe { pc.SetDefaultEndpoint(PCWSTR(id.as_ptr()), role) }.ok().map_err(s)?;
    }
    Ok(())
}

/// Log native crashes (access violations in FFI etc.) that never reach Rust's panic hook.
pub fn crash_handler() {
    use windows::Win32::System::Diagnostics::Debug::{EXCEPTION_POINTERS, SetUnhandledExceptionFilter};
    unsafe extern "system" fn filter(e: *const EXCEPTION_POINTERS) -> i32 {
        let r = unsafe { &*(*e).ExceptionRecord };
        log(format!("CRASH exceção 0x{:08X} em {:?}\n{}", r.ExceptionCode.0, r.ExceptionAddress, std::backtrace::Backtrace::force_capture()));
        0 // EXCEPTION_CONTINUE_SEARCH: let Windows handle/report it as usual
    }
    unsafe { SetUnhandledExceptionFilter(Some(filter)) };
}

/// Close the console window when started by double-click (we own the console alone).
pub fn hide_console() {
    use windows::Win32::System::Console::{FreeConsole, GetConsoleProcessList};
    let mut p = [0u32; 2];
    if unsafe { GetConsoleProcessList(&mut p) } == 1 {
        unsafe { _ = FreeConsole() };
    }
}

fn endpoint_volume() -> Result<windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume, String> {
    unsafe { enumerator()?.GetDefaultAudioEndpoint(eRender, eConsole).and_then(|d| d.Activate(CLSCTX_ALL, None)) }.map_err(s)
}

/// Master volume of the default output, 0.0..=1.0.
pub fn volume() -> Result<f32, String> {
    unsafe { endpoint_volume()?.GetMasterVolumeLevelScalar() }.map_err(s)
}

pub fn set_volume(v: f32) -> Result<(), String> {
    unsafe { endpoint_volume()?.SetMasterVolumeLevelScalar(v.clamp(0.0, 1.0), std::ptr::null()) }.map_err(s)
}

// ---------- iniciar com o sistema: HKCU\...\Run ----------

const RUN: PCWSTR = w!(r"Software\Microsoft\Windows\CurrentVersion\Run");

pub fn autostart() -> bool {
    use windows::Win32::System::Registry::*;
    unsafe { RegGetValueW(HKEY_CURRENT_USER, RUN, w!("monctl"), RRF_RT_REG_SZ, None, None, None) }.is_ok()
}

/// Points at this exe (opens minimized). ponytail: moving the exe leaves a stale entry; toggle again.
pub fn set_autostart(on: bool) -> Result<(), String> {
    use windows::Win32::System::Registry::*;
    let r = if on {
        let exe = std::env::current_exe().map_err(s)?;
        let v: Vec<u16> = format!("\"{}\" --min", exe.display()).encode_utf16().chain([0]).collect();
        unsafe { RegSetKeyValueW(HKEY_CURRENT_USER, RUN, w!("monctl"), REG_SZ.0, Some(v.as_ptr().cast()), (v.len() * 2) as u32) }
    } else {
        unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN, w!("monctl")) }
    };
    r.ok().map_err(s)
}
