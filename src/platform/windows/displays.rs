// CCD API: display topology (which outputs are on, like Win+P but per output).
use super::{error_text, wide_string};
use crate::log::log;
use std::collections::HashSet;
use windows::Win32::Devices::Display::*;
use windows::Win32::Foundation::ERROR_INSUFFICIENT_BUFFER;
use windows::Win32::Graphics::Gdi::{DISPLAYCONFIG_PATH_ACTIVE, DISPLAYCONFIG_PATH_MODE_IDX_INVALID};

pub(crate) struct Display {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) active: bool,
    pub(crate) gdi: String,
    /// connector type: HDMI, DisplayPort...
    pub(crate) kind: String,
}

fn query(flags: QUERY_DISPLAY_CONFIG_FLAGS) -> Result<(Vec<DISPLAYCONFIG_PATH_INFO>, Vec<DISPLAYCONFIG_MODE_INFO>), String> {
    loop {
        let (mut path_count, mut mode_count) = (0u32, 0u32);
        unsafe { GetDisplayConfigBufferSizes(flags, &mut path_count, &mut mode_count) }.ok().map_err(error_text)?;
        let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); path_count as usize];
        let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); mode_count as usize];
        let status = unsafe { QueryDisplayConfig(flags, &mut path_count, paths.as_mut_ptr(), &mut mode_count, modes.as_mut_ptr(), None) };
        if status == ERROR_INSUFFICIENT_BUFFER {
            continue; // topology changed between the two calls
        }
        status.ok().map_err(error_text)?;
        paths.truncate(path_count as usize);
        modes.truncate(mode_count as usize);
        return Ok((paths, modes));
    }
}

fn target_id(path: &DISPLAYCONFIG_PATH_INFO) -> String {
    let adapter = path.targetInfo.adapterId;
    format!("{:x}{:08x}:{}", adapter.HighPart, adapter.LowPart, path.targetInfo.id)
}

fn source_key(path: &DISPLAYCONFIG_PATH_INFO) -> (i32, u32, u32) {
    let adapter = path.sourceInfo.adapterId;
    (adapter.HighPart, adapter.LowPart, path.sourceInfo.id)
}

fn target_name(path: &DISPLAYCONFIG_PATH_INFO) -> String {
    let mut request = DISPLAYCONFIG_TARGET_DEVICE_NAME::default();
    request.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME;
    request.header.size = size_of::<DISPLAYCONFIG_TARGET_DEVICE_NAME>() as u32;
    request.header.adapterId = path.targetInfo.adapterId;
    request.header.id = path.targetInfo.id;
    if let status @ 1.. = unsafe { DisplayConfigGetDeviceInfo(&mut request.header) } {
        log(format!("ERRO DisplayConfigGetDeviceInfo(target {}) = {status}", path.targetInfo.id));
    }
    let name = wide_string(&request.monitorFriendlyDeviceName);
    if name.is_empty() { format!("Tela {}", path.targetInfo.id) } else { name }
}

fn source_name(path: &DISPLAYCONFIG_PATH_INFO) -> String {
    let mut request = DISPLAYCONFIG_SOURCE_DEVICE_NAME::default();
    request.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME;
    request.header.size = size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32;
    request.header.adapterId = path.sourceInfo.adapterId;
    request.header.id = path.sourceInfo.id;
    if let status @ 1.. = unsafe { DisplayConfigGetDeviceInfo(&mut request.header) } {
        log(format!("ERRO DisplayConfigGetDeviceInfo(source {}) = {status}", path.sourceInfo.id));
    }
    wide_string(&request.viewGdiDeviceName)
}

/// DISPLAYCONFIG_VIDEO_OUTPUT_TECHNOLOGY -> connector label
fn kind(technology: i32) -> &'static str {
    match technology {
        0 => "VGA",
        4 => "DVI",
        5 => "HDMI",
        10 | 11 | 18 => "DisplayPort",
        i32::MIN => "Interna",
        _ => "",
    }
}

/// Every connected output (active or not). QDC_ALL_PATHS lists active paths first.
pub(crate) fn displays() -> Result<Vec<Display>, String> {
    let (paths, _) = query(QDC_ALL_PATHS)?;
    let mut found: Vec<Display> = vec![];
    for path in paths.iter().filter(|path| path.targetInfo.targetAvailable.as_bool()) {
        let (id, active) = (target_id(path), path.flags & DISPLAYCONFIG_PATH_ACTIVE != 0);
        match found.iter_mut().find(|display| display.id == id) {
            Some(display) if active && !display.active => (display.active, display.gdi) = (true, source_name(path)),
            Some(_) => {}
            None => found.push(Display { name: target_name(path), gdi: if active { source_name(path) } else { String::new() }, kind: kind(path.targetInfo.outputTechnology.0).into(), id, active }),
        }
    }
    Ok(found)
}

/// Make exactly `ids` active (like Win+P, but per output).
pub(crate) fn set_active(ids: &[String]) -> Result<(), String> {
    const INVALID: u32 = DISPLAYCONFIG_PATH_MODE_IDX_INVALID;
    let (all_paths, modes) = query(QDC_ALL_PATHS)?;
    let (mut paths, mut new_modes, mut used_sources) = (vec![], vec![], HashSet::new());
    // keep the wanted outputs that are already on, with their current modes
    for path in all_paths.iter().filter(|path| path.flags & DISPLAYCONFIG_PATH_ACTIVE != 0 && ids.contains(&target_id(path))) {
        let mut path = *path;
        unsafe {
            for mode_index in [&mut path.sourceInfo.Anonymous.modeInfoIdx, &mut path.targetInfo.Anonymous.modeInfoIdx] {
                if *mode_index != INVALID {
                    new_modes.push(modes[*mode_index as usize]);
                    *mode_index = new_modes.len() as u32 - 1;
                }
            }
        }
        used_sources.insert(source_key(&path));
        paths.push(path);
    }
    // turn on the rest: any path for that target whose source isn't taken; Windows picks the mode
    for id in ids {
        if paths.iter().any(|path| target_id(path) == *id) {
            continue;
        }
        let Some(path) = all_paths.iter().find(|path| target_id(path) == *id && path.targetInfo.targetAvailable.as_bool() && !used_sources.contains(&source_key(path))) else {
            return Err(format!("sem saída livre na GPU para {id}"));
        };
        let mut path = *path;
        path.flags = DISPLAYCONFIG_PATH_ACTIVE;
        path.sourceInfo.Anonymous.modeInfoIdx = INVALID;
        path.targetInfo.Anonymous.modeInfoIdx = INVALID;
        used_sources.insert(source_key(&path));
        paths.push(path);
    }
    if paths.is_empty() {
        return Err("nenhuma tela selecionada".into());
    }
    normalize(&mut new_modes);
    // Only turning outputs off: hand Windows the exact current modes of the ones that stay and
    // forbid changes, so their timing isn't re-set (that re-set is what makes them blink).
    // ALLOW_CHANGES only when an output is being added (its mode is unknown) or the driver refuses.
    let adding = paths.iter().any(|path| unsafe { path.targetInfo.Anonymous.modeInfoIdx } == INVALID);
    let base = SDC_APPLY | SDC_USE_SUPPLIED_DISPLAY_CONFIG | SDC_SAVE_TO_DATABASE;
    let mut attempts = vec![base | SDC_ALLOW_CHANGES];
    if !adding {
        attempts.insert(0, base);
    }
    let mut errors = vec![];
    for flags in attempts {
        let status = unsafe { SetDisplayConfig(Some(&paths), Some(&new_modes), flags) };
        if status == 0 {
            log(format!("SetDisplayConfig(0x{:X}) ok", flags.0));
            return Ok(());
        }
        log(format!("ERRO SetDisplayConfig(0x{:X}) = {status}", flags.0));
        errors.push(status);
    }
    log("tentando topologia do banco do Windows");
    // fallback: hand Windows only the topology and let it pull modes/positions from its database
    for path in &mut paths {
        path.sourceInfo.Anonymous.modeInfoIdx = INVALID;
        path.targetInfo.Anonymous.modeInfoIdx = INVALID;
    }
    let fallback_status = unsafe { SetDisplayConfig(Some(&paths), None, SDC_APPLY | SDC_TOPOLOGY_SUPPLIED | SDC_ALLOW_PATH_ORDER_CHANGES) };
    if fallback_status == 0 { Ok(()) } else { Err(format!("SetDisplayConfig falhou ({errors:?} / {fallback_status})")) }
}

/// Close horizontal gaps left by removed outputs and keep a primary at (0,0).
fn normalize(modes: &mut [DISPLAYCONFIG_MODE_INFO]) {
    let mut sources: Vec<&mut DISPLAYCONFIG_SOURCE_MODE> = modes
        .iter_mut()
        .filter(|mode| mode.infoType == DISPLAYCONFIG_MODE_INFO_TYPE_SOURCE)
        .map(|mode| unsafe { &mut mode.Anonymous.sourceMode })
        .collect();
    sources.sort_by_key(|source| source.position.x);
    let anchor_index = sources.iter().position(|source| source.position.x == 0 && source.position.y == 0).unwrap_or(0);
    let (mut shift, mut right_edge) = (0, None::<i32>);
    for source in sources.iter_mut() {
        source.position.x -= shift;
        if let Some(edge) = right_edge.filter(|&edge| source.position.x > edge) {
            shift += source.position.x - edge;
            source.position.x = edge;
        }
        right_edge = Some(right_edge.unwrap_or(i32::MIN).max(source.position.x + source.width as i32));
    }
    if let Some(anchor) = sources.get(anchor_index).map(|source| source.position) {
        for source in sources.iter_mut() {
            source.position.x -= anchor.x;
            source.position.y -= anchor.y;
        }
    }
}
