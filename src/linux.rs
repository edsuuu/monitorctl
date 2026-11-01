// Linux backend: /dev/i2c-* (DDC/CI, needs `i2c-dev` loaded + user in group `i2c`),
// xrandr / kscreen-doctor / wlr-randr (outputs), pactl (default audio; works on PipeWire too).
use crate::{Logged, edid_name, log};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::process::Command;
use std::thread::sleep;
use std::time::Duration;

pub struct Display {
    pub id: String,
    pub name: String,
    pub active: bool,
    pub gdi: String,
    /// connector type: HDMI, DisplayPort...
    pub kind: String,
}

fn run(cmd: &str, args: &[&str]) -> Result<String, String> {
    let o = Command::new(cmd).args(args).output().map_err(|e| format!("{cmd}: {e}"))?;
    if !o.status.success() {
        return Err(format!("{cmd}: {}", String::from_utf8_lossy(&o.stderr).trim()));
    }
    Ok(String::from_utf8_lossy(&o.stdout).into_owned())
}

fn wayland() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_some()
}

fn unhex(h: &str) -> Vec<u8> {
    (0..h.len() / 2).filter_map(|i| u8::from_str_radix(&h[i * 2..i * 2 + 2], 16).ok()).collect()
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

pub fn displays() -> Result<Vec<Display>, String> {
    let mut out: Vec<Display> = vec![];
    if !wayland() {
        // X11: xrandr output names differ from DRM connector names, so take everything (EDID too) from xrandr
        let (mut edid, mut in_edid) = (String::new(), false);
        for l in run("xrandr", &["--prop"])?.lines().chain(["END"]) {
            let t = l.trim();
            if in_edid && !t.is_empty() && t.chars().all(|c| c.is_ascii_hexdigit()) {
                edid += t;
                continue;
            }
            if in_edid {
                if let (Some(d), Some(n)) = (out.last_mut(), edid_name(&unhex(&edid))) {
                    d.name = n;
                }
                (in_edid, edid) = (false, String::new());
            }
            if t == "EDID:" {
                in_edid = true;
            } else if !l.starts_with(char::is_whitespace) && l.contains(" connected") {
                let id = l.split(' ').next().unwrap_or_default().to_string();
                // "HDMI-0 connected 1920x1080+0+0 ..." -> active; "HDMI-0 connected (normal ...)" -> off
                let active = l.split(' ').any(|t| t.starts_with(|c: char| c.is_ascii_digit()) && t.contains('x') && t.contains('+'));
                out.push(Display { name: id.clone(), gdi: id.clone(), kind: kind(&id).into(), id, active });
            }
        }
        return Ok(out);
    }
    // Wayland compositors use DRM connector names: read them from sysfs (card1-HDMI-A-1 -> HDMI-A-1)
    for e in fs::read_dir("/sys/class/drm").map_err(|e| e.to_string())?.flatten() {
        let (p, f) = (e.path(), e.file_name().to_string_lossy().into_owned());
        let Some((_, conn)) = f.split_once('-') else { continue };
        if fs::read_to_string(p.join("status")).map_or(true, |s| s.trim() != "connected") {
            continue;
        }
        let active = fs::read_to_string(p.join("enabled")).is_ok_and(|s| s.trim() == "enabled");
        let name = fs::read(p.join("edid")).ok().and_then(|e| edid_name(&e)).unwrap_or(conn.to_string());
        out.push(Display { id: conn.into(), name, active, gdi: conn.into(), kind: kind(conn).into() });
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(out)
}

pub fn set_active(ids: &[String]) -> Result<(), String> {
    let ds = displays()?;
    let anchor = ds.iter().find(|d| d.active && ids.contains(&d.id)).map(|d| d.id.clone());
    // turn on before turning off so there's never zero outputs
    for d in ds.iter().filter(|d| !d.active && ids.contains(&d.id)) {
        toggle(&d.id, true, anchor.as_deref())?;
    }
    for d in ds.iter().filter(|d| d.active && !ids.contains(&d.id)) {
        toggle(&d.id, false, None)?;
    }
    Ok(())
}

fn toggle(id: &str, on: bool, right_of: Option<&str>) -> Result<(), String> {
    if !wayland() {
        let mut a = vec!["--output", id, if on { "--auto" } else { "--off" }];
        if let (true, Some(r)) = (on, right_of) {
            // ponytail: placed right of the first kept output, not the rightmost one
            a.extend(["--right-of", r]);
        }
        run("xrandr", &a)
    } else if std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|d| d.contains("KDE")) {
        run("kscreen-doctor", &[&format!("output.{id}.{}", if on { "enable" } else { "disable" })])
    } else {
        // wlroots (Sway, Hyprland, river...). GNOME Wayland has no such CLI before `gdctl` (GNOME 48).
        run("wlr-randr", &["--output", id, if on { "--on" } else { "--off" }])
    }
    .map(|_| ())
}

// ---------- DDC/CI over i2c-dev ----------

const I2C_SLAVE: u64 = 0x0703;

fn i2c(bus: &str, addr: u16) -> std::io::Result<File> {
    let f = OpenOptions::new().read(true).write(true).open(bus)?;
    if unsafe { libc::ioctl(f.as_raw_fd(), I2C_SLAVE as _, addr as libc::c_ulong) } < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(f)
}

pub struct Monitor {
    pub name: String,
    f: File,
}

pub fn monitors() -> Vec<Monitor> {
    let mut buses: Vec<String> = fs::read_dir("/dev")
        .map(|r| r.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.starts_with("i2c-")).collect())
        .unwrap_or_default();
    buses.sort_by_key(|n| n[4..].parse::<u32>().unwrap_or(0));
    let (mut out, mut seen) = (vec![], vec![]);
    for b in buses {
        // never poke SMBus adapters (RAM SPD lives at 0x50 there)
        if fs::read_to_string(format!("/sys/bus/i2c/devices/{b}/name")).is_ok_and(|n| n.to_lowercase().contains("smbus")) {
            continue;
        }
        let dev = format!("/dev/{b}");
        let edid = i2c(&dev, 0x50).map_err(|e| format!("{dev}: {e} (módulo i2c-dev carregado? usuário no grupo i2c?)")).logged("abrir i2c").ok().and_then(|mut f| {
            f.write_all(&[0]).ok()?;
            let mut e = [0u8; 128];
            f.read_exact(&mut e).ok()?;
            Some(e)
        });
        let Some(edid) = edid else {
            log(format!("{dev}: sem EDID (não é monitor)"));
            continue;
        };
        // same monitor is often reachable on several buses
        let (Some(name), false) = (edid_name(&edid), seen.contains(&edid)) else { continue };
        seen.push(edid);
        match i2c(&dev, 0x37) {
            Ok(f) => out.push(Monitor { name, f }),
            Err(e) => log(format!("ERRO {dev} ({name}) DDC 0x37: {e}")),
        }
    }
    out
}

impl Monitor {
    fn tx(&self, payload: &[u8]) -> Result<(), String> {
        (&self.f).write_all(&crate::ddc_frame(payload)).map_err(|e| format!("{} i2c write: {e}", self.name))
    }

    fn rx(&self, n: usize) -> Result<Vec<u8>, String> {
        let mut b = vec![0; n];
        (&self.f).read_exact(&mut b).map(|_| b).map_err(|e| format!("{} i2c read: {e}", self.name))
    }

    /// Standard VCP (1-byte op) or vendor opcode (AOC F8 E1 xx): same framing on i2c-dev.
    pub fn raw_get(&self, op: &[u8]) -> Result<(u16, u16), String> {
        let mut last = String::new();
        for _ in 0..3 {
            self.tx(&[&[0x01], op].concat())?;
            sleep(Duration::from_millis(50));
            match self.rx(11) {
                Ok(b) => match crate::ddc_reply(&b, op) {
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
        self.tx(&[&[0x03], op, &v.to_be_bytes()].concat())?;
        sleep(Duration::from_millis(50));
        Ok(())
    }

    pub fn get(&self, code: u8) -> Result<(u16, u16), String> {
        self.raw_get(&[code])
    }

    pub fn set(&self, code: u8, v: u16) -> Result<(), String> {
        self.raw_set(&[code], v)
    }

    pub fn caps(&self) -> Result<String, String> {
        let (mut s, mut off) = (vec![], 0u16);
        loop {
            let mut chunk = None;
            for _ in 0..3 {
                self.tx(&[0xF3, (off >> 8) as u8, off as u8])?;
                sleep(Duration::from_millis(50));
                // 6E len E3 offH offL data... chk
                chunk = self.rx(38).logged("caps").ok().filter(|b| b[2] == 0xE3 && u16::from_be_bytes([b[3], b[4]]) == off);
                if chunk.is_some() {
                    break;
                }
            }
            let b = chunk.ok_or("monitor não respondeu ao DDC/CI")?;
            let n = ((b[1] & 0x7F) as usize).saturating_sub(3).min(32);
            if n == 0 {
                break;
            }
            s.extend_from_slice(&b[5..5 + n]);
            off += n as u16;
        }
        Ok(String::from_utf8_lossy(&s).trim_end_matches('\0').to_string())
    }
}

// ---------- audio ----------

/// (sink name, description + any product/nick names, so the TV name from ELD can match)
pub fn audio_outputs() -> Result<Vec<(String, String)>, String> {
    let t = run("pactl", &["list", "sinks"])?;
    Ok(t.split("Sink #")
        .skip(1)
        .filter_map(|b| {
            let field = |k: &str| b.lines().find_map(|l| l.trim().strip_prefix(k).map(|v| v.trim().trim_matches('"').to_string()));
            let mut label = field("Description:")?;
            for k in ["device.product.name = ", "node.nick = "] {
                if let Some(v) = field(k) {
                    label += &format!(" [{v}]");
                }
            }
            Some((field("Name:")?, label))
        })
        .collect())
}

pub fn default_audio() -> Result<String, String> {
    Ok(run("pactl", &["get-default-sink"])?.trim().to_string())
}

pub fn set_default_audio(id: &str) -> Result<(), String> {
    run("pactl", &["set-default-sink", id]).map(|_| ())
}

pub fn hide_console() {}

// ponytail: Rust panics are logged by the hook in main; native signals (SIGSEGV) aren't — add a
// sigaction handler writing to a pre-opened fd if FFI crashes ever show up on Linux.
pub fn crash_handler() {}

/// Volume of the default sink, 0.0..=1.0 ("Volume: front-left: 65536 / 100% / 0.00 dB, ...").
pub fn volume() -> Result<f32, String> {
    let t = run("pactl", &["get-sink-volume", "@DEFAULT_SINK@"])?;
    t.split('/').nth(1).and_then(|p| p.trim().trim_end_matches('%').parse::<f32>().ok()).map(|p| p / 100.0).ok_or(format!("volume ilegível: {t}"))
}

pub fn set_volume(v: f32) -> Result<(), String> {
    run("pactl", &["set-sink-volume", "@DEFAULT_SINK@", &format!("{:.0}%", v.clamp(0.0, 1.0) * 100.0)]).map(|_| ())
}
