use crate::log::Logged;
use std::ffi::c_void;
use std::sync::OnceLock;
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::core::{s, w};

type QueryInterface = unsafe extern "C" fn(u32) -> *const c_void;

/// nvapi_QueryInterface(id) as a typed fn pointer. IDs from AOC G-Menu's DDCHelperLib (monitorctl-rea).
pub(super) fn query_interface<Function: Copy>(id: u32) -> Result<Function, String> {
    static QUERY_INTERFACE: OnceLock<Result<usize, String>> = OnceLock::new();
    let query_interface = QUERY_INTERFACE
        .get_or_init(|| unsafe {
            let library = LoadLibraryW(w!("nvapi64.dll")).map_err(|error| format!("nvapi64.dll (sem GPU NVIDIA?): {error}"))?;
            let query_interface: QueryInterface = std::mem::transmute(GetProcAddress(library, s!("nvapi_QueryInterface")).ok_or("nvapi_QueryInterface ausente")?);
            let initialize = query_interface(0x0150E828);
            if initialize.is_null() {
                return Err("NvAPI_Initialize ausente".into());
            }
            match std::mem::transmute::<_, unsafe extern "C" fn() -> i32>(initialize)() {
                0 => Ok(query_interface as usize),
                status => Err(format!("NvAPI_Initialize = {status}")),
            }
        })
        .clone()
        .logged("NVAPI")?;
    let function = unsafe { std::mem::transmute::<usize, QueryInterface>(query_interface)(id) };
    if function.is_null() {
        return Err(format!("NVAPI 0x{id:08X} indisponível"));
    }
    Ok(unsafe { std::mem::transmute_copy(&function) })
}
