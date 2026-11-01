// xrandr (X11) / kscreen-doctor (KDE Wayland) / wlr-randr (wlroots): outputs on and off.
use super::run;
use crate::ddc::edid_name;
use std::fs;

pub(crate) struct Display {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) active: bool,
    pub(crate) gdi: String,
    /// connector type: HDMI, DisplayPort...
    pub(crate) kind: String,
}

fn wayland() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_some()
}

fn hex_to_bytes(hex: &str) -> Vec<u8> {
    (0..hex.len() / 2).filter_map(|index| u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16).ok()).collect()
}

/// connector name -> label ("DP-1", "DisplayPort-0", "HDMI-A-1", "eDP-1"...)
fn kind(id: &str) -> &'static str {
    match id.split('-').next().unwrap_or_default() {
        "DP" | "DisplayPort" => "DisplayPort",
        "HDMI" => "HDMI",
        "DVI" => "DVI",
        "VGA" => "VGA",
        "eDP" | "LVDS" => "Interna",
        _ => "",
    }
}

pub(crate) fn displays() -> Result<Vec<Display>, String> {
    let mut found: Vec<Display> = vec![];
    if !wayland() {
        // X11: xrandr output names differ from DRM connector names, so take everything (EDID too) from xrandr
        let (mut edid, mut in_edid) = (String::new(), false);
        for line in run("xrandr", &["--prop"])?.lines().chain(["END"]) {
            let trimmed = line.trim();
            if in_edid && !trimmed.is_empty() && trimmed.chars().all(|character| character.is_ascii_hexdigit()) {
                edid += trimmed;
                continue;
            }
            if in_edid {
                if let (Some(display), Some(name)) = (found.last_mut(), edid_name(&hex_to_bytes(&edid))) {
                    display.name = name;
                }
                (in_edid, edid) = (false, String::new());
            }
            if trimmed == "EDID:" {
                in_edid = true;
            } else if !line.starts_with(char::is_whitespace) && line.contains(" connected") {
                let id = line.split(' ').next().unwrap_or_default().to_string();
                // "HDMI-0 connected 1920x1080+0+0 ..." -> active; "HDMI-0 connected (normal ...)" -> off
                let active = line.split(' ').any(|token| token.starts_with(|character: char| character.is_ascii_digit()) && token.contains('x') && token.contains('+'));
                found.push(Display { name: id.clone(), gdi: id.clone(), kind: kind(&id).into(), id, active });
            }
        }
        return Ok(found);
    }
    // Wayland compositors use DRM connector names: read them from sysfs (card1-HDMI-A-1 -> HDMI-A-1)
    for entry in fs::read_dir("/sys/class/drm").map_err(|error| error.to_string())?.flatten() {
        let (path, file_name) = (entry.path(), entry.file_name().to_string_lossy().into_owned());
        let Some((_, connector)) = file_name.split_once('-') else { continue };
        if fs::read_to_string(path.join("status")).map_or(true, |status| status.trim() != "connected") {
            continue;
        }
        let active = fs::read_to_string(path.join("enabled")).is_ok_and(|enabled| enabled.trim() == "enabled");
        let name = fs::read(path.join("edid")).ok().and_then(|edid| edid_name(&edid)).unwrap_or(connector.to_string());
        found.push(Display { id: connector.into(), name, active, gdi: connector.into(), kind: kind(connector).into() });
    }
    found.sort_by(|first, second| first.id.cmp(&second.id));
    Ok(found)
}

pub(crate) fn set_active(ids: &[String]) -> Result<(), String> {
    let current = displays()?;
    let anchor = current.iter().find(|display| display.active && ids.contains(&display.id)).map(|display| display.id.clone());
    // turn on before turning off so there's never zero outputs
    for display in current.iter().filter(|display| !display.active && ids.contains(&display.id)) {
        toggle(&display.id, true, anchor.as_deref())?;
    }
    for display in current.iter().filter(|display| display.active && !ids.contains(&display.id)) {
        toggle(&display.id, false, None)?;
    }
    Ok(())
}

fn toggle(id: &str, on: bool, right_of: Option<&str>) -> Result<(), String> {
    if !wayland() {
        let mut arguments = vec!["--output", id, if on { "--auto" } else { "--off" }];
        if let (true, Some(anchor)) = (on, right_of) {
            // ponytail: placed right of the first kept output, not the rightmost one
            arguments.extend(["--right-of", anchor]);
        }
        run("xrandr", &arguments)
    } else if std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|desktop| desktop.contains("KDE")) {
        run("kscreen-doctor", &[&format!("output.{id}.{}", if on { "enable" } else { "disable" })])
    } else {
        // wlroots (Sway, Hyprland, river...). GNOME Wayland has no such CLI before `gdctl` (GNOME 48).
        run("wlr-randr", &["--output", id, if on { "--on" } else { "--off" }])
    }
    .map(|_| ())
}
