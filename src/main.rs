#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
use linux as sys;
#[cfg(windows)]
mod win;
#[cfg(windows)]
use win as sys;

use eframe::egui;
use eframe::egui::{Color32, RichText, Stroke};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::{JoinHandle, sleep, spawn};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

// ---------- logging ----------

fn app_dir() -> PathBuf {
    let base = std::env::var("APPDATA")
        .or_else(|_| std::env::var("XDG_CONFIG_HOME"))
        .unwrap_or_else(|_| format!("{}/.config", std::env::var("HOME").unwrap_or_default()));
    PathBuf::from(base).join("monctl")
}

fn now() -> String {
    let t = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    let (days, s) = ((t.as_secs() / 86400) as i64, t.as_secs() % 86400);
    // civil-from-days (H. Hinnant)
    let z = days + 719468;
    let (era, doe) = (z.div_euclid(146097), z.rem_euclid(146097));
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let (d, m) = (doy - (153 * mp + 2) / 5 + 1, if mp < 10 { mp + 3 } else { mp - 9 });
    let y = yoe + era * 400 + (m <= 2) as i64;
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}.{:03}Z", s / 3600, s / 60 % 60, s % 60, t.subsec_millis())
}

/// Append a line to monctl.log (rotated at 1 MB). Never fails.
pub fn log(msg: impl AsRef<str>) {
    static LOCK: Mutex<()> = Mutex::new(());
    let _g = LOCK.lock();
    let p = app_dir().join("monctl.log");
    _ = std::fs::create_dir_all(app_dir());
    if std::fs::metadata(&p).is_ok_and(|m| m.len() > 1 << 20) {
        _ = std::fs::rename(&p, p.with_extension("log.old"));
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&p) {
        _ = writeln!(f, "{} [{:?}] {}", now(), std::thread::current().id(), msg.as_ref());
    }
}

pub trait Logged {
    fn logged(self, ctx: &str) -> Self;
}

impl<T> Logged for Result<T, String> {
    fn logged(self, ctx: &str) -> Self {
        if let Err(e) = &self {
            log(format!("ERRO {ctx}: {e}"));
        }
        self
    }
}

// ---------- DDC/CI framing (raw I2C paths: Linux i2c-dev, NVAPI) ----------

/// [0x51, 0x80|len, payload.., chk]; chk = XOR including the 0x6E destination address.
pub fn ddc_frame(payload: &[u8]) -> Vec<u8> {
    let mut f = vec![0x51, 0x80 | payload.len() as u8];
    f.extend_from_slice(payload);
    f.push(f.iter().fold(0x6E, |a, b| a ^ b));
    f
}

/// Parse a VCP reply `6E 88 02 rc op.. maxH maxL curH curL chk` -> (cur, max).
/// AOC `F8 E1 sub` replies echo `F8 sub` in bytes 4-5; checking it rejects stale buffers (NVAPI returns the
/// previous reply when the monitor ignores an opcode it doesn't support).
pub fn ddc_reply(b: &[u8], op: &[u8]) -> Option<(u16, u16)> {
    if b.len() < 11 || b[0] != 0x6E || b[1] & 0x80 == 0 || b[2] != 0x02 || (op.len() == 1 && (b[3] != 0 || b[4] != op[0])) || (op.len() == 3 && (b[4] != op[0] || b[5] != op[2])) {
        return None;
    }
    Some((u16::from_be_bytes([b[8], b[9]]), u16::from_be_bytes([b[6], b[7]])))
}

// ---------- features ----------

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
enum Op {
    /// Standard MCCS VCP code.
    Vcp(u8),
    /// AOC/TPV vendor opcode `F8 E1 sub`, raw I2C only (NVAPI on Windows). `bit` = flag in the `F8 E1 00`
    /// support mask, 0 = not in the mask (shown if it reads).
    Aoc(u8, u16),
    /// Dell multiplexed code (0xF4): write the selector, wait, read; the low byte is the value. Write = Set(v).
    DellQ(u8, u16),
    /// Dell bit field (AlienVision 0xEC/0xED): read as DellQ, write = read-modify-write.
    DellBits { code: u8, sel: u16, set: u16, keep: u16, shift: u8, mask: u16 },
    /// Dell preset: written to DC/14/F0, read back from E2 with its own numbering.
    DellPreset,
    /// AlienVision on/off + slot (0xEC).
    DellVision,
}

/// (op, label, named values). Empty values = slider. Empty label = hide this code.
type Def = (Op, &'static str, &'static [(u16, &'static str)]);

const OFF_ON: &[(u16, &str)] = &[(0, "Desligado"), (1, "Ligado")];

const MCCS: &[Def] = &[
    (Op::Vcp(0x0C), "Temp. de cor (passos)", &[]),
    (Op::Vcp(0x10), "Brilho", &[]),
    (Op::Vcp(0x12), "Contraste", &[]),
    (Op::Vcp(0x14), "Preset de cor", &[(1, "sRGB"), (2, "Nativo"), (3, "4000K"), (4, "5000K"), (5, "6500K"), (6, "7500K"), (7, "8200K"), (8, "9300K"), (9, "10000K"), (0x0A, "11500K"), (0x0B, "Usuário 1"), (0x0C, "Usuário 2"), (0x0D, "Usuário 3")]),
    (Op::Vcp(0x16), "Ganho vermelho", &[]),
    (Op::Vcp(0x18), "Ganho verde", &[]),
    (Op::Vcp(0x1A), "Ganho azul", &[]),
    (Op::Vcp(0x60), "Entrada", &[(0x01, "VGA 1"), (0x03, "DVI 1"), (0x0F, "DisplayPort 1"), (0x10, "DisplayPort 2"), (0x11, "HDMI 1"), (0x12, "HDMI 2"), (0x13, "HDMI 3"), (0x15, "USB-C 1"), (0x16, "USB-C 2"), (0x1B, "USB-C")]),
    (Op::Vcp(0x62), "Volume", &[]),
    (Op::Vcp(0x6C), "Nível de preto R", &[]),
    (Op::Vcp(0x6E), "Nível de preto G", &[]),
    (Op::Vcp(0x70), "Nível de preto B", &[]),
    (Op::Vcp(0x86), "Escala da imagem", &[(1, "Sem escala"), (2, "Máxima, mantém proporção"), (3, "Máx. vertical, mantém proporção"), (4, "Máx. horizontal, mantém proporção"), (5, "Máx. vertical, distorce"), (6, "Máx. horizontal, distorce"), (7, "Expansão linear H"), (8, "Expansão linear H+V"), (9, "Squeeze"), (0x0A, "Expansão não linear")]),
    (Op::Vcp(0x87), "Nitidez", &[]),
    (Op::Vcp(0x8A), "Saturação", &[]),
    (Op::Vcp(0x8D), "Mudo", &[(1, "Ligado"), (2, "Desligado")]),
    (Op::Vcp(0x90), "Matiz", &[]),
    (Op::Vcp(0xCC), "Idioma do OSD", &[(1, "中文 (trad.)"), (2, "English"), (3, "Français"), (4, "Deutsch"), (5, "Italiano"), (6, "日本語"), (7, "한국어"), (8, "Português (PT)"), (9, "Русский"), (0x0A, "Español"), (0x0B, "Svenska"), (0x0C, "Türkçe"), (0x0D, "中文 (simpl.)"), (0x0E, "Português (BR)"), (0x0F, "العربية"), (0x12, "Čeština"), (0x14, "Nederlands"), (0x16, "Suomi"), (0x1A, "Magyar"), (0x1E, "Polski")]),
    (Op::Vcp(0xDC), "Modo de imagem", &[(0, "Padrão"), (1, "Produtividade"), (2, "Misto"), (3, "Filme"), (4, "Usuário"), (5, "Jogos"), (6, "Esportes"), (7, "Profissional")]),
];

/// AOC G-Menu 3.31 (monctl-rea/aoc-25g3zm.md).
const AOC: &[Def] = &[
    (Op::Vcp(0x14), "Temperatura de cor", &[(1, "sRGB"), (5, "Warm"), (6, "Normal"), (8, "Cool"), (0x0B, "User"), (0x0E, "AdobeRGB")]),
    (Op::Vcp(0xDC), "Modo de jogo", &[(0, "Desligado"), (0x0B, "FPS"), (0x0C, "RTS"), (0x0D, "Racing"), (0x0E, "Gamer 1"), (0x0F, "Gamer 2"), (0x10, "Gamer 3")]),
    (Op::Vcp(0x86), "Proporção", &[(1, "1:1"), (2, "Tela cheia"), (5, "Aspecto"), (0x0B, "17\" 4:3"), (0x0C, "19\" 4:3"), (0x0D, "19\" 16:10"), (0x0E, "21.5\" 16:9"), (0x0F, "22\" 16:10"), (0x10, "23\" 16:9"), (0x11, "23.6\" 16:9"), (0x12, "24\" 16:9"), (0x13, "24\" 16:10")]),
    (Op::Aoc(0x51, 0x0010), "Overdrive", &[(0, "Desligado"), (1, "Fraco"), (2, "Médio"), (3, "Forte"), (4, "Boost")]),
    (Op::Aoc(0x58, 0x0800), "Shadow Control", &[]),
    (Op::Aoc(0x57, 0x0400), "Game Color", &[]),
    (Op::Aoc(0x55, 0x0100), "Low Input Lag", OFF_ON),
    (Op::Aoc(0x5A, 0x4000), "Dial Point (mira)", OFF_ON),
    (Op::Aoc(0x5B, 0x8000), "Frame Counter", &[(0, "Desligado"), (1, "Superior direito"), (2, "Inferior direito"), (3, "Inferior esquerdo"), (4, "Superior esquerdo")]),
    (Op::Aoc(0x59, 0x1000), "Low Blue", &[(0, "Desligado"), (1, "Multimídia"), (2, "Internet"), (3, "Escritório"), (4, "Leitura")]),
    (Op::Aoc(0x11, 0x0001), "Modo ECO", &[(0, "Standard"), (1, "Text"), (2, "Internet"), (3, "Game"), (4, "Movie"), (5, "Sport"), (6, "Reading"), (7, "Uniformity")]),
    (Op::Aoc(0x12, 0x0002), "Gamma", &[(0, "Gamma 1"), (1, "Gamma 2"), (2, "Gamma 3"), (3, "Gamma 4"), (4, "Gamma 5")]),
    (Op::Aoc(0x13, 0x0004), "DCR", OFF_ON),
    (Op::Aoc(0x08, 0x0008), "DCB", &[(0, "Desligado"), (0x21, "Full Enhance"), (0x22, "Nature Skin"), (0x23, "Sky Blue"), (0x24, "Green Field"), (0x25, "Auto Detect"), (0x26, "Demo")]),
    (Op::Aoc(0x52, 0x0020), "HDR", &[]), // names depend on the reported max, see hdr_names
    (Op::Aoc(0x14, 0), "Clean Vision", &[(0, "Desligado"), (1, "Fraco"), (2, "Médio"), (3, "Forte")]),
    (Op::Aoc(0x71, 0), "Saturação vermelho", &[]),
    (Op::Aoc(0x72, 0), "Saturação amarelo", &[]),
    (Op::Aoc(0x73, 0), "Saturação verde", &[]),
    (Op::Aoc(0x74, 0), "Saturação ciano", &[]),
    (Op::Aoc(0x75, 0), "Saturação azul", &[]),
    (Op::Aoc(0x76, 0), "Saturação magenta", &[]),
    (Op::Aoc(0x78, 0), "Matiz vermelho", &[]),
    (Op::Aoc(0x79, 0), "Matiz amarelo", &[]),
    (Op::Aoc(0x7A, 0), "Matiz verde", &[]),
    (Op::Aoc(0x7B, 0), "Matiz ciano", &[]),
    (Op::Aoc(0x7C, 0), "Matiz azul", &[]),
    (Op::Aoc(0x7D, 0), "Matiz magenta", &[]),
];

const fn ed(shift: u8, mask: u16) -> Op {
    Op::DellBits { code: 0xED, sel: 1 << 13, set: 0x8000, keep: 0x1FFF, shift, mask }
}
const fn ec(shift: u8, mask: u16) -> Op {
    Op::DellBits { code: 0xEC, sel: 1 << 12, set: 0x4000, keep: 0x07FF, shift, mask }
}
const LEVELS: &[(u16, &str)] = &[(0, "Desligado"), (1, "Baixo"), (2, "Médio"), (3, "Alto")];

/// Alienware Command Center 6.14 (monctl-rea/alienware-aw2525hm.md).
const AW: &[Def] = &[
    (Op::Vcp(0x14), "", &[]), // presets are handled by DellPreset
    (Op::Vcp(0xDC), "", &[]),
    (Op::DellPreset, "Preset", &[(0, "Standard"), (32, "FPS"), (33, "MOBA/RTS"), (34, "RPG"), (47, "SPORTS"), (4, "Game 1"), (30, "Game 2"), (31, "Game 3"), (14, "Warm"), (18, "Cool"), (11, "sRGB"), (20, "Custom Color"), (127, "Indisponível (HDR)")]),
    (Op::DellQ(0xF4, 0x2F), "Tempo de resposta", &[(0x20, "Extreme"), (0x21, "Super Fast"), (0x22, "Fast")]),
    (Op::DellQ(0xF4, 0x3F), "Dark Stabilizer", &[(0x30, "Desligado"), (0x31, "Nível 1"), (0x32, "Nível 2"), (0x33, "Nível 3")]),
    (Op::DellQ(0xF4, 0x1F), "Game Enhancement", &[(0x10, "Desligado"), (0x11, "Contador de FPS"), (0x13, "Timer 30 min"), (0x14, "Timer 40 min"), (0x15, "Timer 50 min"), (0x16, "Timer 60 min"), (0x17, "Timer 90 min")]),
    (Op::DellQ(0xF4, 0xBF), "Posição do contador", &[(0xB1, "Superior direito"), (0xB2, "Superior esquerdo"), (0xB3, "Inferior direito"), (0xB4, "Inferior esquerdo")]),
    (Op::DellQ(0xF4, 0x4F), "Smart HDR", &[(0x40, "Desligado"), (0x41, "Desktop"), (0x42, "Movie HDR"), (0x43, "Game HDR"), (0x45, "Custom Color HDR"), (0x4E, "Indisponível")]),
    (Op::DellVision, "AlienVision", &[(0, "Desligado"), (1, "Vision 1"), (2, "Vision 2"), (3, "Vision 3")]),
    (ed(10, 7), "Modo · V1", &[(0, "Desligado"), (1, "Night"), (2, "Chroma"), (3, "Radar")]),
    (ed(8, 3), "Tamanho · V1", &[(0, "1"), (1, "2"), (2, "3"), (3, "4")]),
    (ed(6, 3), "Nitidez · V1", OFF_ON),
    (ed(2, 0xF), "Mira · V1", &[(0, "Desligada"), (1, "Ponto"), (2, "Cruz grande"), (3, "Barra/cruz pequena"), (4, "Cruz vazada pequena"), (5, "Custom 1"), (6, "Custom 2"), (7, "Custom 3"), (8, "Custom 4")]),
    (ed(0, 3), "Cor da mira · V1", &[(0, "Branca"), (1, "Vermelha"), (2, "Verde"), (3, "Dinâmica")]),
    (ec(5, 3), "Zoom · V1", LEVELS),
    (ec(3, 3), "Transparência · V1", LEVELS),
    (ec(0, 7), "Posição · V1", &[(0, "Centro"), (1, "Sup. esquerdo"), (2, "Sup. direito"), (3, "Inf. esquerdo"), (4, "Inf. direito")]),
];

/// Dell preset value (E2 numbering) -> (code, value) to write.
const DELL_PRESET_WRITE: &[(u16, u8, u16)] = &[(0, 0xDC, 0x00), (4, 0xDC, 0x05), (11, 0x14, 0x01), (18, 0x14, 0x08), (14, 0x14, 0x0B), (20, 0x14, 0x0C), (30, 0xF0, 0x0D), (31, 0xF0, 0x0E), (32, 0xF0, 0x0F), (33, 0xF0, 0x10), (34, 0xF0, 0x11), (47, 0xF0, 0x13)];

/// (vendor, vendor-specific controls) by EDID model name.
fn model(model: &str) -> (&'static str, &'static [Def]) {
    // ponytail: exact model names; other AOC (G-Menu) / Alienware models likely share these tables
    match model {
        "AW2525HM" => ("Alienware", AW),
        "25G3ZM" => ("AOC", AOC),
        _ => ("", &[]),
    }
}

fn known(name: &str) -> Vec<Def> {
    let m = model(name).1;
    // model entries override generic ones with the same op
    m.iter().chain(MCCS.iter().filter(|g| !m.iter().any(|v| v.0 == g.0))).copied().collect()
}

fn hdr_names(max: u16) -> &'static [(u16, &'static str)] {
    match max {
        0 | 1 => &[(0, "Desligado"), (1, "Auto")],
        3 => &[(0, "Desligado"), (1, "Picture"), (2, "Movie"), (3, "Game")],
        _ => &[(0, "Desligado"), (1, "Display"), (2, "Picture"), (3, "Movie"), (4, "Game"), (5, "Color"), (6, "Peak")],
    }
}

/// The capabilities code that must be listed for this op to be shown (None = probe instead).
fn gate(op: Op) -> Option<u8> {
    match op {
        Op::Vcp(c) | Op::DellQ(c, _) | Op::DellBits { code: c, .. } => Some(c),
        Op::DellPreset => Some(0xE2),
        Op::DellVision => Some(0xEC),
        Op::Aoc(..) => None,
    }
}

const STEP: Duration = Duration::from_millis(200); // Dell: pause between selector write and read

fn read(m: &sys::Monitor, op: Op) -> Result<(u16, u16), String> {
    match op {
        Op::Vcp(c) => m.get(c),
        Op::Aoc(sub, _) => m.raw_get(&[0xF8, 0xE1, sub]),
        Op::DellQ(c, sel) => {
            m.set(c, sel)?;
            sleep(STEP);
            m.get(c).map(|(v, x)| (v & 0xFF, x))
        }
        Op::DellBits { code, sel, shift, mask, .. } => {
            m.set(code, sel)?;
            sleep(STEP);
            m.get(code).map(|(v, _)| ((v >> shift) & mask, mask))
        }
        Op::DellPreset => m.get(0xE2).map(|(v, x)| (v & 0xFF, x)),
        Op::DellVision => m.get(0xEC).map(|(v, _)| (if v & 0xC000 == 0xC000 { (v & 0x3000) >> 12 } else { 0 }, 3)),
    }
}

fn write(m: &sys::Monitor, op: Op, v: u16) -> Result<(), String> {
    match op {
        Op::Vcp(c) | Op::DellQ(c, _) => m.set(c, v),
        Op::Aoc(sub, _) => m.raw_set(&[0xF8, 0xE1, sub], v),
        Op::DellBits { code, sel, set, keep, shift, mask } => {
            m.set(code, sel)?;
            sleep(STEP);
            let (cur, _) = m.get(code)?;
            sleep(STEP);
            m.set(code, (cur & keep & !(mask << shift)) | ((v & mask) << shift) | sel | set)
        }
        Op::DellPreset => {
            let &(_, c, val) = DELL_PRESET_WRITE.iter().find(|p| p.0 == v).ok_or(format!("preset {v} sem comando de escrita"))?;
            m.set(c, val)
        }
        Op::DellVision if v == 0 => {
            let (cur, _) = m.get(0xEC)?;
            m.set(0xEC, 0x8000 | (cur & 0x3000))
        }
        Op::DellVision => m.set(0xEC, 0xC000 | (v & 3) << 12),
    }
}

/// Parse the `vcp(...)` section of a DDC/CI capabilities string into (code, allowed values).
fn parse_vcp(caps: &str) -> Vec<(u8, Vec<u16>)> {
    let Some(i) = caps.find("vcp(") else { return vec![] };
    let (mut out, mut depth, mut tok) = (Vec::<(u8, Vec<u16>)>::new(), 0, String::new());
    let flush = |tok: &mut String, depth: i32, out: &mut Vec<(u8, Vec<u16>)>| {
        match (depth, out.last_mut()) {
            // codes: some monitors omit spaces ("100204"), so split every 2 hex digits
            (0, _) => out.extend(tok.as_bytes().chunks(2).filter_map(|c| u8::from_str_radix(std::str::from_utf8(c).ok()?, 16).ok()).map(|c| (c, vec![]))),
            // values: Dell lists 16-bit ones ("ED(0400 0800)", "F5(100 500)")
            (_, Some(last)) if !tok.is_empty() => last.1.extend(u16::from_str_radix(tok, 16).ok()),
            _ => {}
        }
        tok.clear();
    };
    for ch in caps[i + 4..].chars() {
        match ch {
            '(' => {
                flush(&mut tok, depth, &mut out);
                depth += 1;
            }
            ')' => {
                flush(&mut tok, depth, &mut out);
                if depth == 0 {
                    break;
                }
                depth -= 1;
            }
            c if c.is_ascii_hexdigit() => tok.push(c),
            _ => flush(&mut tok, depth, &mut out),
        }
    }
    out
}

/// Monitor name from a 128-byte EDID block (descriptor 0xFC), else PNP id + product code.
pub fn edid_name(e: &[u8]) -> Option<String> {
    if e.len() < 128 || e[..8] != [0, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0] {
        return None;
    }
    for d in e[54..126].chunks(18) {
        if d[..3] == [0, 0, 0] && d[3] == 0xFC {
            return Some(String::from_utf8_lossy(&d[5..]).split('\n').next().unwrap_or("").trim().to_string());
        }
    }
    let m = u16::from_be_bytes([e[8], e[9]]);
    let l = |s: u16| (b'@' + (m >> s & 31) as u8) as char;
    Some(format!("{}{}{}{:02X}{:02X}", l(10), l(5), l(0), e[11], e[10]))
}

struct Feat {
    op: Op,
    name: String,
    cur: u16,
    /// last saved value: `cur` differs until the user clicks Salvar (or Desfazer brings it back)
    saved: u16,
    max: u16,
    values: Vec<(u16, String)>,
}

/// Unsaved changes survive a crash here: "Op\tsaved" per line, rewritten after every write.
fn pending_path(name: &str) -> std::path::PathBuf {
    app_dir().join(format!("pendente-{name}.txt"))
}

fn write_pending(name: &str, feats: &[Feat]) {
    let dirty: String = feats.iter().filter(|f| f.cur != f.saved).map(|f| format!("{:?}\t{}\n", f.op, f.saved)).collect();
    let p = pending_path(name);
    let r = if dirty.is_empty() { std::fs::remove_file(&p).or_else(|e| if e.kind() == std::io::ErrorKind::NotFound { Ok(()) } else { Err(e) }) } else { std::fs::write(&p, dirty) };
    _ = r.map_err(|e| e.to_string()).logged(&format!("{} pendências", p.display()));
}

/// Baselines left by a previous run that closed without saving: those changes show up as unsaved again.
fn read_pending(name: &str, feats: &mut [Feat]) {
    let Ok(t) = std::fs::read_to_string(pending_path(name)) else { return };
    for (op, v) in t.lines().filter_map(|l| l.split_once('\t')) {
        if let (Some(f), Ok(v)) = (feats.iter_mut().find(|f| format!("{:?}", f.op) == op), v.parse()) {
            f.saved = v;
            log(format!("{name}: {op} não salvo de uma execução anterior (salvo = {v}, atual = {})", f.cur));
        }
    }
}

/// Read every supported feature of a monitor. Ok((features, unmapped capability codes)).
fn load(m: &sys::Monitor) -> Result<(Vec<Feat>, String), String> {
    let k = known(&m.name);
    let codes = match m.caps().logged(&format!("{} caps", m.name)) {
        Ok(c) => {
            log(format!("{} caps: {c}", m.name));
            Some(parse_vcp(&c))
        }
        // no capabilities string: show whatever answers, but bail out fast if nothing does
        Err(e) => m.get(0x10).logged(&format!("{} sonda 0x10", m.name)).map_err(|_| e).map(|_| None)?,
    };
    let aoc_mask = k
        .iter()
        .any(|d| matches!(d.0, Op::Aoc(..)))
        .then(|| m.raw_get(&[0xF8, 0xE1, 0x00]).logged(&format!("{} AOC F8E100", m.name)).ok())
        .flatten()
        .map(|(c, _)| c);
    if let Some(mask) = aoc_mask {
        log(format!("{} AOC máscara de suporte = 0x{mask:04X}", m.name));
    }
    let mut feats = vec![];
    for &(op, name, names) in &k {
        let listed = match (gate(op), &codes) {
            (Some(c), Some(cs)) => cs.iter().find(|x| x.0 == c).map(|x| x.1.clone()),
            (Some(_), None) => Some(vec![]),
            (None, _) => match op {
                Op::Aoc(_, bit) => aoc_mask.filter(|m| bit == 0 || m & bit != 0).map(|_| vec![]),
                _ => None,
            },
        };
        let (Some(listed), false) = (listed, name.is_empty()) else { continue };
        let Ok((mut cur, max)) = read(m, op).logged(&format!("{} ler {op:?}", m.name)) else { continue };
        let names = if let Op::Aoc(0x52, _) = op { hdr_names(max) } else { names };
        let mut set: Vec<u16> = if listed.is_empty() || !matches!(op, Op::Vcp(_)) { names.iter().map(|n| n.0).collect() } else { listed };
        if !set.contains(&cur) && set.contains(&(cur & 0xFF)) {
            cur &= 0xFF; // Dell answers input 0x0F as 0x0F0F
        }
        if !set.is_empty() && !set.contains(&cur) {
            set.push(cur);
        }
        let label = |v: u16| names.iter().find(|n| n.0 == v).map_or(format!("0x{v:02X}"), |n| n.1.to_string());
        let values = set.into_iter().map(|v| (v, label(v))).collect();
        feats.push(Feat { op, name: name.into(), cur, saved: cur, max, values });
    }
    let unknown = codes
        .unwrap_or_default()
        .iter()
        .filter(|(c, _)| !k.iter().any(|d| gate(d.0) == Some(*c)))
        .map(|(c, _)| format!("{c:02X}"))
        .collect::<Vec<_>>()
        .join(" ");
    read_pending(&m.name, &mut feats);
    log(format!("{}: {} controles, sem nome: {unknown}", m.name, feats.len()));
    Ok((feats, unknown))
}

// ---------- displays ----------

/// Activate exactly `want`. Audio is never switched here: only when the user picks an output.
fn apply(want: &[String]) -> Result<String, String> {
    log(format!("aplicar telas: {want:?}"));
    let r = apply_inner(want).logged("aplicar telas");
    if let Ok(m) = &r {
        log(m);
    }
    r
}

fn apply_inner(want: &[String]) -> Result<String, String> {
    if want.is_empty() {
        return Err("pelo menos uma tela precisa ficar ativa".into());
    }
    let before = sys::displays()?;
    sys::set_active(want)?;
    // safety lock: never leave the machine with zero active outputs
    if !sys::displays()?.iter().any(|d| d.active) {
        let prev: Vec<String> = before.iter().filter(|d| d.active).map(|d| d.id.clone()).collect();
        sys::set_active(&prev).logged("reverter telas")?;
        return Err("nenhuma tela ficou ativa, configuração revertida".into());
    }
    Ok("telas aplicadas".into())
}

// ---------- CLI ----------

const USAGE: &str = "monctl                      abre a interface
monctl list                 telas, monitores DDC/CI e saídas de áudio
monctl on|off|only NOME..   liga/desliga telas (NOME = parte do nome, ex: samsung)
monctl all                  liga todas as telas conectadas
monctl caps NOME            capabilities DDC/CI + todos os controles e valores
monctl get NOME CODIGO      ex: monctl get aw 10   |  AOC: monctl get aoc f8e151
monctl set NOME CODIGO VAL  ex: monctl set 25g3 10 70
monctl audio NOME           define a saída de áudio padrão
log: %APPDATA%\\monctl\\monctl.log  (Linux: ~/.config/monctl/monctl.log)";

/// "10" -> [0x10]; "f8e151" -> [F8, E1, 51]
fn opcode(s: &str) -> Result<Vec<u8>, String> {
    let s = s.trim_start_matches("0x");
    let v: Option<Vec<u8>> = (0..s.len()).step_by(2).map(|i| s.get(i..i + 2).and_then(|h| u8::from_str_radix(h, 16).ok())).collect();
    v.filter(|v| !v.is_empty()).ok_or(format!("código inválido: {s}"))
}

fn monitor(q: &str) -> Result<sys::Monitor, String> {
    let q = q.to_lowercase();
    sys::monitors().into_iter().find(|m| m.name.to_lowercase().contains(&q)).ok_or(format!("monitor '{q}' não encontrado"))
}

fn cli(a: &[String]) -> Result<(), String> {
    let arg = |i: usize| a.get(i).map(String::as_str).ok_or(USAGE.to_string());
    match a[0].as_str() {
        "list" => {
            for d in sys::displays()? {
                println!("{} {:<12} {:<14} {}", if d.active { "●" } else { "○" }, d.name, d.gdi, d.id);
            }
            for m in sys::monitors() {
                println!("ddc: {}", m.name);
            }
            let def = sys::default_audio().logged("áudio padrão").unwrap_or_default();
            for (id, n) in sys::audio_outputs()? {
                println!("áudio: {} {n}", if id == def { "*" } else { " " });
            }
        }
        "on" | "off" | "only" | "all" => {
            let ds = sys::displays()?;
            let mut want: Vec<String> = ds.iter().filter(|d| d.active).map(|d| d.id.clone()).collect();
            let mut pick = vec![];
            for q in &a[1..] {
                let q = q.to_lowercase();
                pick.push(ds.iter().find(|d| d.name.to_lowercase().contains(&q) || d.id == q).ok_or(format!("tela '{q}' não encontrada"))?.id.clone());
            }
            match a[0].as_str() {
                "on" => want.extend(pick.into_iter().filter(|p| !want.contains(p)).collect::<Vec<_>>()),
                "off" => want.retain(|w| !pick.contains(w)),
                "only" => want = pick,
                _ => want = ds.iter().map(|d| d.id.clone()).collect(),
            }
            println!("{}", apply(&want)?);
        }
        "caps" => {
            let m = monitor(arg(1)?)?;
            println!("{}\n", m.caps().unwrap_or_else(|e| format!("(sem capabilities: {e})")));
            let (feats, unknown) = load(&m)?;
            for f in feats {
                let label = f.values.iter().find(|v| v.0 == f.cur).map_or(String::new(), |v| format!(" ({})", v.1));
                println!("{:<26} {:>5}{label:<22} max {:<5} {:?}", f.name, f.cur, f.max, f.op);
            }
            println!("\ncódigos sem nome: {unknown}");
        }
        "get" => {
            let (m, op) = (monitor(arg(1)?)?, opcode(arg(2)?)?);
            let (c, x) = if op.len() == 1 { m.get(op[0]) } else { m.raw_get(&op) }?;
            println!("{c} (0x{c:04X}) / max {x}");
        }
        "set" => {
            let (m, op) = (monitor(arg(1)?)?, opcode(arg(2)?)?);
            let v: u16 = arg(3)?.parse().map_err(|_| "valor inválido")?;
            log(format!("cli set {} {op:02X?} = {v}", m.name));
            if op.len() == 1 { m.set(op[0], v) } else { m.raw_set(&op, v) }?;
        }
        "audio" => {
            let q = arg(1)?.to_lowercase();
            let outs = sys::audio_outputs()?;
            let (id, n) = outs.iter().find(|o| o.1.to_lowercase().contains(&q)).ok_or(format!("áudio '{q}' não encontrado"))?;
            sys::set_default_audio(id)?;
            log(format!("áudio → {n}"));
            println!("áudio → {n}");
        }
        _ => return Err(USAGE.into()),
    }
    Ok(())
}

// ---------- GUI ----------

type MonState = Result<(Vec<Feat>, String), String>;

struct Mon {
    name: String,
    state: Arc<Mutex<MonState>>,
    /// Some((op, value)) = write · None = DDC "Save Current Settings"
    tx: mpsc::Sender<Option<(Op, u16)>>,
    worker: JoinHandle<()>,
}

impl Mon {
    /// Run `f` on the loaded features (no-op while loading or on error).
    fn feats<R: Default>(&self, f: impl FnOnce(&mut Vec<Feat>) -> R) -> R {
        match &mut *self.state.lock().unwrap() {
            Ok((fs, _)) => f(fs),
            Err(_) => R::default(),
        }
    }

    fn dirty(&self) -> usize {
        self.feats(|fs| fs.iter().filter(|f| f.cur != f.saved).count())
    }

    /// Desfazer: write the saved values back.
    fn discard(&self) {
        self.feats(|fs| {
            for f in fs.iter_mut().filter(|f| f.cur != f.saved) {
                log(format!("{}: desfazer {} {} → {}", self.name, f.name, f.cur, f.saved));
                f.cur = f.saved;
                _ = self.tx.send(Some((f.op, f.cur)));
            }
        })
    }

    /// Salvar: current values become the baseline and the monitor commits them.
    fn save(&self) {
        self.feats(|fs| fs.iter_mut().for_each(|f| f.saved = f.cur));
        log(format!("{}: salvar", self.name));
        _ = self.tx.send(None);
    }

    /// Window closing: unsaved changes don't persist. Waits until the worker wrote them back.
    fn close(self) {
        if self.dirty() > 0 {
            log(format!("{}: fechando com {} alterações não salvas, desfazendo", self.name, self.dirty()));
            self.discard();
        }
        drop(self.tx);
        _ = self.worker.join();
    }
}

/// One worker per monitor: loads features, then applies writes, coalescing slider drags.
/// It's also the only thing touching that monitor, which Dell's select-then-read codes require.
fn spawn_mon(m: sys::Monitor, ctx: egui::Context) -> Mon {
    let state = Arc::new(Mutex::new(Err("carregando…".to_string())));
    let (tx, rx) = mpsc::channel::<Option<(Op, u16)>>();
    let (st, name) = (state.clone(), m.name.clone());
    let worker = spawn(move || {
        *st.lock().unwrap() = load(&m).logged(&format!("{} carregar", m.name));
        ctx.request_repaint();
        while let Ok(first) = rx.recv() {
            let (mut last, mut save) = (BTreeMap::new(), false);
            for msg in std::iter::once(first).chain(rx.try_iter()) {
                match msg {
                    Some((op, v)) => _ = last.insert(op, v),
                    None => save = true,
                }
            }
            for (op, v) in last {
                log(format!("{}: {op:?} = {v}", m.name));
                _ = write(&m, op, v).logged(&format!("{} gravar {op:?}={v}", m.name));
            }
            if save {
                _ = m.save().logged(&format!("{} salvar (DDC 0x0C)", m.name));
            }
            if let Ok((fs, _)) = &*st.lock().unwrap() {
                write_pending(&m.name, fs);
            }
        }
    });
    Mon { name, state, tx, worker }
}

// Look ported from the Claude Design canvas "monctl UI" (https://claude.ai/artifact/Rf1HUMVeKckdNW75yiJLVQ).
const BG: Color32 = Color32::from_rgb(0x0E, 0x10, 0x14);
const HEADER: Color32 = Color32::from_rgb(0x12, 0x15, 0x1B);
const CARD: Color32 = Color32::from_rgb(0x16, 0x1A, 0x21);
const CARD_LINE: Color32 = Color32::from_rgb(0x25, 0x2B, 0x36);
const CTRL: Color32 = Color32::from_rgb(0x1D, 0x22, 0x2B);
const CTRL_LINE: Color32 = Color32::from_rgb(0x2C, 0x33, 0x40);
const TEXT: Color32 = Color32::from_rgb(0xE7, 0xEA, 0xF0);
const SOFT: Color32 = Color32::from_rgb(0xCF, 0xD5, 0xDF);
const MUTED: Color32 = Color32::from_rgb(0x8C, 0x95, 0xA6);
const LABEL: Color32 = Color32::from_rgb(0x7F, 0x88, 0x99);
const ACCENT: Color32 = Color32::from_rgb(0x45, 0xD0, 0xC1);
const INK: Color32 = Color32::from_rgb(0x06, 0x27, 0x23);
const CHIP: Color32 = Color32::from_rgb(0x17, 0x3A, 0x37);
const CHIP_TEXT: Color32 = Color32::from_rgb(0x7F, 0xE3, 0xD8);
const AMBER: Color32 = Color32::from_rgb(0xF5, 0xB9, 0x4A);

fn bold() -> egui::FontFamily {
    egui::FontFamily::Name("bold".into())
}

/// Fonts (Segoe UI on Windows, closest to the design's Inter; egui's own elsewhere) + dark theme.
fn setup_look(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let dir = PathBuf::from(std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into())).join("Fonts");
    for (name, file) in [("ui", "segoeui.ttf"), ("ui-bold", "seguisb.ttf"), ("mono", "consola.ttf")] {
        if let Ok(b) = std::fs::read(dir.join(file)) {
            fonts.font_data.insert(name.into(), Arc::new(egui::FontData::from_owned(b)));
        }
    }
    let has = |n: &str| fonts.font_data.contains_key(n);
    let (ui, ui_bold, mono) = (has("ui"), has("ui-bold"), has("mono"));
    let fam = |f: egui::FontFamily, fonts: &mut egui::FontDefinitions| fonts.families.entry(f).or_default().clone();
    let mut prop = fam(egui::FontFamily::Proportional, &mut fonts);
    if ui {
        prop.insert(0, "ui".into());
    }
    let mut b = prop.clone();
    if ui_bold {
        b.insert(0, "ui-bold".into());
    }
    let mut m = fam(egui::FontFamily::Monospace, &mut fonts);
    if mono {
        m.insert(0, "mono".into());
    }
    fonts.families.insert(egui::FontFamily::Proportional, prop);
    fonts.families.insert(egui::FontFamily::Monospace, m);
    fonts.families.insert(bold(), b);
    ctx.set_fonts(fonts);

    let mut v = egui::Visuals::dark();
    v.panel_fill = BG;
    v.window_fill = CTRL;
    v.window_stroke = Stroke::new(1.0, CTRL_LINE);
    v.extreme_bg_color = Color32::from_rgb(0x2A, 0x31, 0x3D);
    v.selection.bg_fill = ACCENT;
    v.selection.stroke = Stroke::new(1.0, INK);
    v.slider_trailing_fill = true;
    v.handle_shape = egui::style::HandleShape::Circle;
    let r = egui::CornerRadius::same(8);
    for (w, fill, line) in [
        (&mut v.widgets.inactive, CTRL, CTRL_LINE),
        (&mut v.widgets.hovered, Color32::from_rgb(0x23, 0x29, 0x35), Color32::from_rgb(0x3B, 0x44, 0x54)),
        (&mut v.widgets.active, Color32::from_rgb(0x2A, 0x31, 0x40), ACCENT),
        (&mut v.widgets.open, CTRL, ACCENT),
    ] {
        (w.bg_fill, w.weak_bg_fill, w.bg_stroke, w.fg_stroke, w.corner_radius) = (fill, fill, Stroke::new(1.0, line), Stroke::new(1.5, TEXT), r);
    }
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, CARD_LINE);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, SOFT);
    for t in [egui::Theme::Dark, egui::Theme::Light] {
        ctx.set_visuals_of(t, v.clone());
    }
    ctx.all_styles_mut(|s| {
        s.spacing.item_spacing = egui::vec2(8.0, 5.0);
        s.spacing.button_padding = egui::vec2(10.0, 4.0);
        s.spacing.interact_size.y = 28.0;
        s.text_styles.insert(egui::TextStyle::Body, egui::FontId::proportional(14.0));
        s.text_styles.insert(egui::TextStyle::Button, egui::FontId::proportional(13.5));
    });
}

const SECTIONS: [&str; 5] = ["Imagem", "Jogo", "AlienVision", "Conforto", "Cor e entrada"];

fn section(name: &str) -> &'static str {
    match name {
        "Tempo de resposta" | "Dark Stabilizer" | "Game Enhancement" | "Posição do contador" | "Smart HDR" | "Overdrive" | "Shadow Control" | "Game Color" | "Low Input Lag"
        | "Dial Point (mira)" | "Frame Counter" | "HDR" => "Jogo",
        "AlienVision" => "AlienVision",
        n if n.ends_with("· V1") => "AlienVision",
        "Low Blue" | "Modo ECO" | "Gamma" | "DCR" | "DCB" | "Clean Vision" => "Conforto",
        "Ganho vermelho" | "Ganho verde" | "Ganho azul" | "Nível de preto R" | "Nível de preto G" | "Nível de preto B" | "Entrada" | "Volume" | "Mudo" | "Idioma do OSD" => "Cor e entrada",
        _ => "Imagem",
    }
}

/// What each control does, with an example (the "?" button).
fn help(name: &str) -> &'static str {
    match name {
        "Brilho" => "Luz de fundo do painel.\nEx.: 20–30 à noite, 70–100 de dia.",
        "Contraste" => "Diferença entre claro e escuro. Alto demais estoura os brancos.\nEx.: 50–75 costuma ser o ponto certo.",
        "Preset de cor" | "Temperatura de cor" => "Tom do branco.\nWarm/6500K = amarelado, confortável · Cool/9300K = azulado · sRGB = cores fiéis (web/fotos) · User/Custom libera os ganhos RGB.",
        "Temp. de cor (passos)" => "Ajuste fino da temperatura de cor em passos (MCCS 0x0C).",
        "Ganho vermelho" | "Ganho verde" | "Ganho azul" => "Intensidade de cada cor, vale no modo User/Custom.\nEx.: baixar o azul deixa a tela mais quente.",
        "Nível de preto R" | "Nível de preto G" | "Nível de preto B" => "Nível de preto de cada canal (offset).",
        "Entrada" => "Troca a entrada de vídeo do monitor.\nCuidado: numa entrada sem o PC, este app perde o controle até você voltar pelo menu do monitor.",
        "Volume" => "Volume do alto-falante / saída de fone do próprio monitor.",
        "Mudo" => "Silencia o áudio do monitor.",
        "Escala da imagem" | "Proporção" => "Como uma resolução menor é esticada.\nEx.: 1:1 mostra sem escalar (bom para 4:3 em jogo competitivo) · Tela cheia estica.",
        "Nitidez" => "Realce de bordas. Alto demais cria halos ao redor de textos.",
        "Saturação" => "Intensidade das cores. 0 = preto e branco.",
        "Matiz" => "Gira o tom de todas as cores.",
        "Idioma do OSD" => "Idioma do menu na tela do monitor.",
        "Modo de imagem" => "Preset de imagem do fabricante (Jogos, Filme, Esportes...).",
        "Modo de jogo" => "Presets de jogo da AOC.\nEx.: FPS clareia sombras e realça contraste · RTS/Racing ajustam cor · Gamer 1–3 são perfis seus.",
        "Overdrive" => "Acelera a troca dos pixels (menos rastro/ghosting em movimento).\nEx.: Médio é um bom início em 240 Hz; Forte/Boost pode criar contorno claro (overshoot).",
        "Shadow Control" => "Clareia áreas escuras sem estourar o resto.\nEx.: subir para enxergar inimigos em cantos escuros.",
        "Game Color" => "Saturação extra para jogos.\nEx.: 12–15 deixa as cores mais vivas.",
        "Low Input Lag" => "Desliga processamentos de imagem para reduzir o atraso entre mouse e tela.",
        "Dial Point (mira)" => "Desenha uma mira fixa no centro da tela, por cima de qualquer jogo.",
        "Frame Counter" => "Mostra os FPS/Hz atuais num canto da tela.",
        "Low Blue" => "Reduz luz azul (tela amarelada) para cansar menos a vista.\nEx.: Leitura é o mais forte.",
        "Modo ECO" => "Presets de brilho/contraste da AOC (Text, Internet, Game, Movie, Sport, Reading).",
        "Gamma" => "Curva de gama. Gamma 1 ≈ 2.2 (padrão); os outros clareiam ou escurecem os meios-tons.",
        "DCR" => "Contraste dinâmico: ajusta a luz de fundo conforme a cena. O brilho pode 'respirar'.",
        "DCB" => "Realce de cores da AOC (pele, céu azul, verde).",
        "HDR" => "Modo de imagem HDR simulado do monitor.",
        "Clean Vision" => "Redução de ruído da imagem.",
        "Preset" => "Preset de imagem do Alienware.\nFPS clareia sombras · MOBA/RTS, RPG e SPORTS realçam cor · Game 1–3 são perfis seus · sRGB = cores fiéis.",
        "Tempo de resposta" => "Overdrive do Alienware.\nExtreme = menos rastro, pode criar halos · Fast = imagem mais limpa.",
        "Dark Stabilizer" => "Clareia áreas escuras para enxergar em jogos escuros.\nEx.: Nível 2–3 em jogos de tiro à noite.",
        "Game Enhancement" => "Mostra um contador de FPS ou um timer (30–90 min) na tela.",
        "Posição do contador" => "Canto da tela onde aparecem o contador de FPS e o timer.",
        "Smart HDR" => "Modo HDR do monitor (Desktop, Movie, Game).\nSó tem efeito com o HDR ligado no Windows.",
        "AlienVision" => "Filtros de visão do Alienware para jogos (Night clareia, Chroma muda cores, Radar...) com mira.\nLiga um dos 3 perfis Vision.",
        n if n.ends_with("· V1") => "Configuração do perfil Vision 1 do AlienVision.\nAtive em AlienVision → Vision 1 para ver o efeito.",
        _ => "",
    }
}

fn help_button(ui: &mut egui::Ui, text: &str) {
    let b = egui::Button::new(RichText::new("?").size(11.0).color(MUTED))
        .corner_radius(11)
        .min_size(egui::vec2(22.0, 22.0))
        .fill(Color32::TRANSPARENT)
        .stroke(Stroke::new(1.0, Color32::from_rgb(0x39, 0x41, 0x4F)));
    let r = ui.add_enabled(!text.is_empty(), b);
    if !text.is_empty() {
        r.on_hover_text(text);
    }
}

fn chip(ui: &mut egui::Ui, text: &str, on: bool) {
    let (fill, color) = if on { (CHIP, CHIP_TEXT) } else { (CTRL, MUTED) };
    egui::Frame::new().fill(fill).corner_radius(10).inner_margin(egui::Margin::symmetric(8, 2)).show(ui, |ui| {
        ui.label(RichText::new(text).size(11.0).family(bold()).color(color));
    });
}

/// iOS-style switch (from the egui demo), accent when on.
fn toggle(ui: &mut egui::Ui, on: &mut bool, enabled: bool, label: &str) -> egui::Response {
    let (rect, mut resp) = ui.allocate_exact_size(egui::vec2(44.0, 24.0), if enabled { egui::Sense::click() } else { egui::Sense::hover() });
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, enabled, *on, label));
    let t = ui.ctx().animate_bool_responsive(resp.id, *on);
    let a = if enabled { 1.0 } else { 0.45 };
    ui.painter().rect_filled(rect, 12.0, (if *on { ACCENT } else { CTRL_LINE }).gamma_multiply(a));
    let x = egui::lerp((rect.left() + 12.0)..=(rect.right() - 12.0), t);
    ui.painter().circle_filled(egui::pos2(x, rect.center().y), 9.0, (if *on { INK } else { SOFT }).gamma_multiply(a));
    resp
}

/// One control row: label, "?", slider + value or combo.
fn row(ui: &mut egui::Ui, f: &mut Feat, m: &Mon) {
    // fixed widths from the column width, so long values truncate instead of widening the card
    let cw = ui.available_width();
    ui.horizontal(|ui| {
        ui.set_min_height(30.0);
        let lw = (cw * 0.40).clamp(90.0, 160.0);
        ui.allocate_ui_with_layout(egui::vec2(lw, 20.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.set_width(lw);
            ui.add(egui::Label::new(RichText::new(&f.name).color(if f.cur != f.saved { AMBER } else { SOFT })).truncate());
        });
        help_button(ui, help(&f.name));
        let w = (cw - lw - 22.0 - 3.0 * ui.spacing().item_spacing.x).max(60.0);
        let changed = if f.values.is_empty() {
            ui.spacing_mut().slider_width = w - 36.0;
            let c = ui.add(egui::Slider::new(&mut f.cur, 0..=f.max.max(1)).show_value(false)).changed();
            ui.add_sized([28.0, 20.0], egui::Label::new(RichText::new(f.cur.to_string()).monospace().size(12.5).color(SOFT)));
            c
        } else {
            let sel = f.values.iter().find(|v| v.0 == f.cur).map_or(f.cur.to_string(), |v| v.1.clone());
            let mut c = false;
            egui::ComboBox::from_id_salt((&m.name, f.op)).width(w).truncate().selected_text(sel).show_ui(ui, |ui| {
                for (v, n) in &f.values {
                    c |= ui.selectable_value(&mut f.cur, *v, n).changed();
                }
            });
            c
        };
        if changed {
            _ = m.tx.send(Some((f.op, f.cur)));
        }
    });
}

/// A monitor's controls, sections split in two balanced sub-columns so everything fits without scrolling.
fn controls(ui: &mut egui::Ui, m: &Mon) {
    let mut st = m.state.lock().unwrap();
    let feats = match &mut *st {
        Err(e) => {
            ui.label(RichText::new(e.as_str()).color(MUTED));
            return;
        }
        Ok((feats, _)) => feats,
    };
    let mut secs: Vec<(&str, Vec<&mut Feat>)> = SECTIONS.iter().map(|s| (*s, vec![])).collect();
    for f in feats.iter_mut() {
        let s = section(&f.name);
        secs.iter_mut().find(|x| x.0 == s).unwrap().1.push(f);
    }
    secs.retain(|s| !s.1.is_empty());
    for (_, fs) in &mut secs {
        fs.sort_by_key(|f| ["Brilho", "Contraste"].iter().position(|n| *n == f.name).unwrap_or(2));
    }
    let n = if ui.available_width() >= 520.0 { 2 } else { 1 };
    let (mut cols, mut rows): (Vec<Vec<(&str, Vec<&mut Feat>)>>, Vec<usize>) = ((0..n).map(|_| vec![]).collect(), vec![0; n]);
    for s in secs {
        let i = (0..n).min_by_key(|&i| rows[i]).unwrap();
        rows[i] += s.1.len() + 1;
        cols[i].push(s);
    }
    ui.columns(n, |uis| {
        for (ui, col) in uis.iter_mut().zip(cols.iter_mut()) {
            for (title, fs) in col.iter_mut() {
                ui.add_space(6.0);
                ui.label(RichText::new(title.to_uppercase()).size(11.0).family(bold()).color(LABEL));
                for f in fs.iter_mut() {
                    row(ui, f, m);
                }
            }
        }
    });
}

/// Salvar / Desfazer: changes apply live, but only persist once saved.
fn save_bar(ui: &mut egui::Ui, m: &Mon) {
    let n = m.dirty();
    ui.add_space(6.0);
    ui.separator();
    ui.horizontal(|ui| {
        let txt = match n {
            0 => "Tudo salvo".to_string(),
            1 => "1 alteração não salva".to_string(),
            n => format!("{n} alterações não salvas"),
        };
        ui.label(RichText::new(txt).size(12.5).color(if n == 0 { MUTED } else { AMBER }));
        help_button(ui, "As mudanças valem na hora, mas só ficam gravadas quando você clica em Salvar.\nDesfazer volta para os valores salvos. Fechar o monctl sem salvar também desfaz.\nEx.: subiu o brilho para testar e não gostou → Desfazer.");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let save = egui::Button::new(RichText::new("Salvar").family(bold()).color(if n > 0 { INK } else { MUTED })).fill(if n > 0 { ACCENT } else { CTRL }).min_size(egui::vec2(84.0, 30.0));
            if ui.add_enabled(n > 0, save).on_hover_text("Grava as alterações no monitor").clicked() {
                m.save();
            }
            if ui.add_enabled(n > 0, egui::Button::new("Desfazer").min_size(egui::vec2(84.0, 30.0))).on_hover_text("Volta para os valores salvos").clicked() {
                m.discard();
            }
        });
    });
}

struct Gui {
    displays: Vec<sys::Display>,
    mons: Vec<Mon>,
    status: Arc<Mutex<String>>,
    busy: Arc<AtomicBool>,
    reload: bool,
    styled: bool,
    /// (deadline, previous outputs): set when a change turned off every output you were looking at
    revert: Arc<Mutex<Option<(Instant, Vec<String>)>>>,
    audio: Vec<(String, String)>,
    audio_def: String,
    volume: f32,
    autostart: bool,
    /// started with --min (autostart): minimize on the first frame
    start_min: bool,
}

impl Gui {
    fn reload(&mut self, ctx: &egui::Context) {
        self.displays = sys::displays().logged("listar telas").unwrap_or_default();
        self.mons = sys::monitors().into_iter().map(|m| spawn_mon(m, ctx.clone())).collect();
        self.reload_audio();
        self.reload = false;
    }

    fn reload_audio(&mut self) {
        self.audio = sys::audio_outputs().logged("listar áudio").unwrap_or_default();
        self.audio_def = sys::default_audio().logged("áudio padrão").unwrap_or_default();
        self.volume = sys::volume().logged("ler volume").map_or(0.0, |v| v * 100.0);
    }

    fn set_audio(&mut self, id: &str, name: &str) {
        if sys::set_default_audio(id).logged(&format!("escolher áudio {name}")).is_ok() {
            log(format!("áudio → {name}"));
            self.reload_audio();
        }
    }

    /// Monitors of a display that answer DDC/CI (or are still loading). A TV that only shows up
    /// in dxva2 but never answers is treated as "no image controls", not as an error card.
    fn ddc<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Mon> + 'a {
        self.mons.iter().filter(move |m| m.name == name && !matches!(&*m.state.lock().unwrap(), Err(e) if e != "carregando…"))
    }

    /// Audio endpoint that belongs to a display (TV over HDMI), matched by name.
    fn audio_of(&self, display: &str) -> Option<(String, String)> {
        let key = display.to_lowercase();
        self.audio.iter().find(|o| !key.is_empty() && o.1.to_lowercase().contains(&key)).cloned()
    }

    fn header(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            // monitor glyph
            let (r, _) = ui.allocate_exact_size(egui::vec2(24.0, 24.0), egui::Sense::hover());
            let p = ui.painter();
            p.rect_stroke(egui::Rect::from_min_size(r.min + egui::vec2(1.0, 3.0), egui::vec2(22.0, 14.0)), 2.0, Stroke::new(2.0, ACCENT), egui::StrokeKind::Inside);
            p.line_segment([r.min + egui::vec2(8.0, 21.0), r.min + egui::vec2(16.0, 21.0)], Stroke::new(2.0, ACCENT));
            p.line_segment([r.min + egui::vec2(12.0, 17.0), r.min + egui::vec2(12.0, 21.0)], Stroke::new(2.0, ACCENT));
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.label(RichText::new("monctl").size(17.0).family(bold()).color(TEXT));
                let ddc = self.displays.iter().filter(|d| self.ddc(&d.name).next().is_some()).count();
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("{} telas · {ddc} com DDC/CI", self.displays.len())).size(12.0).color(MUTED));
                    let st = self.status.lock().unwrap().clone();
                    if !st.is_empty() {
                        let c = if self.busy.load(Ordering::Relaxed) { AMBER } else if st.starts_with("erro") { egui::Color32::from_rgb(0xF0, 0x6A, 0x6A) } else { ACCENT };
                        ui.label(RichText::new(format!("· {st}")).size(12.0).color(c));
                    }
                });
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let reload = egui::Button::new(RichText::new("⟳").size(18.0)).min_size(egui::vec2(38.0, 38.0)).corner_radius(10).fill(CARD);
                if ui.add(reload).on_hover_text("Recarregar telas, monitores e áudio").clicked() {
                    self.reload = true;
                }
                egui::Frame::new().fill(CARD).stroke(Stroke::new(1.0, CARD_LINE)).corner_radius(10).inner_margin(egui::Margin::symmetric(12, 6)).show(ui, |ui| {
                    let mut on = self.autostart;
                    let t = toggle(ui, &mut on, true, "Iniciar com o sistema").on_hover_text("Abre o monctl minimizado quando você entra no Windows/Linux");
                    if t.changed() && sys::set_autostart(on).logged("iniciar com o sistema").is_ok() {
                        log(format!("iniciar com o sistema: {on}"));
                        self.autostart = on;
                    }
                    ui.label(RichText::new("Iniciar com o sistema").size(12.5).color(MUTED));
                });
                egui::Frame::new().fill(CARD).stroke(Stroke::new(1.0, CARD_LINE)).corner_radius(10).inner_margin(egui::Margin::symmetric(12, 6)).show(ui, |ui| {
                    ui.horizontal(|ui| self.audio_bar(ui));
                });
            });
        });
    }

    /// Default output picker + system volume. Laid out right-to-left (header's right side), so in reverse.
    fn audio_bar(&mut self, ui: &mut egui::Ui) {
        help_button(ui, "Saída de áudio padrão do sistema e o volume dela.\nLigar/desligar telas nunca troca o áudio: ele só muda quando você escolhe aqui ou em \"Usar o áudio desta tela\".");
        ui.label(RichText::new(format!("{:>3.0}%", self.volume)).monospace().size(12.5).color(SOFT));
        ui.spacing_mut().slider_width = 120.0;
        let r = ui.add(egui::Slider::new(&mut self.volume, 0.0..=100.0).show_value(false));
        if r.changed() {
            _ = sys::set_volume(self.volume / 100.0).logged("volume");
        }
        if r.drag_stopped() {
            log(format!("volume → {:.0}%", self.volume));
        }
        ui.label(RichText::new("Volume").size(12.5).color(MUTED));
        let cur = self.audio.iter().find(|o| o.0 == self.audio_def).map_or("—".to_string(), |o| o.1.clone());
        let mut pick = None;
        egui::ComboBox::from_id_salt("audio").selected_text(cur).width(290.0).truncate().show_ui(ui, |ui| {
            for (id, n) in &self.audio {
                if ui.selectable_label(*id == self.audio_def, n).clicked() {
                    pick = Some((id.clone(), n.clone()));
                }
            }
        });
        if let Some((id, n)) = pick {
            self.set_audio(&id, &n);
        }
        ui.label(RichText::new("Saída").size(12.5).color(MUTED));
    }

    fn ui(&mut self, ui: &mut egui::Ui) {
        if !self.styled {
            setup_look(ui.ctx());
            if self.start_min {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Minimized(true));
            }
            self.styled = true;
        }
        // closing: unsaved monitor changes are written back before the window goes away
        if ui.ctx().input(|i| i.viewport().close_requested()) {
            std::mem::take(&mut self.mons).into_iter().for_each(Mon::close);
        }
        if self.reload && !self.busy.load(Ordering::Relaxed) {
            self.reload(ui.ctx());
        }
        egui::Panel::top("header")
            .frame(egui::Frame::new().fill(HEADER).inner_margin(egui::Margin::symmetric(20, 10)).stroke(Stroke::new(1.0, CARD_LINE)))
            .show(ui, |ui| self.header(ui));
        let (mut want, mut set_audio) = (None, None::<(String, String)>);
        let active: Vec<String> = self.displays.iter().filter(|d| d.active).map(|d| d.id.clone()).collect();
        egui::CentralPanel::default().frame(egui::Frame::new().fill(BG).inner_margin(egui::Margin::same(16))).show(ui, |ui| {
            if let Some((deadline, prev)) = self.revert.lock().unwrap().clone() {
                let left = deadline.saturating_duration_since(Instant::now()).as_secs();
                egui::Frame::new().fill(Color32::from_rgb(0x2E, 0x26, 0x12)).stroke(Stroke::new(1.0, AMBER)).corner_radius(10).inner_margin(egui::Margin::symmetric(14, 8)).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(format!("Manter esta configuração de telas? Revertendo em {left}s")).color(AMBER));
                        if ui.button("Manter").clicked() {
                            *self.revert.lock().unwrap() = None;
                        }
                        if ui.button("Reverter").clicked() || left == 0 {
                            *self.revert.lock().unwrap() = None;
                            want = Some(prev);
                        }
                    });
                });
                ui.add_space(10.0);
                ui.ctx().request_repaint_after(Duration::from_millis(250));
            }
            // ponytail: ScrollArea only as a fallback for tiny windows; the layout is sized to fit 1080p
            egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                let busy = self.busy.load(Ordering::Relaxed);
                let has_ddc = |d: &sys::Display| self.ddc(&d.name).next().is_some();
                // cards side by side: monitors with controls get twice the width of plain outputs (TV)
                let units: f32 = self.displays.iter().map(|d| if has_ddc(d) { 2.0 } else { 1.0 }).sum();
                let gap = 14.0;
                let unit = (ui.available_width() - gap * (self.displays.len().max(1) - 1) as f32) / units.max(1.0);
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = gap;
                    for d in &self.displays {
                        let w = unit * if has_ddc(d) { 2.0 } else { 1.0 };
                        ui.allocate_ui_with_layout(egui::vec2(w, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                            ui.set_width(w);
                            let audio = self.audio_of(&d.name);
                            egui::Frame::new().fill(CARD).stroke(Stroke::new(1.0, CARD_LINE)).corner_radius(14).inner_margin(egui::Margin::same(14)).show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                let title = |ui: &mut egui::Ui| {
                                    ui.vertical(|ui| {
                                        ui.spacing_mut().item_spacing.y = 2.0;
                                        ui.horizontal_wrapped(|ui| {
                                            ui.label(RichText::new(&d.name).size(19.0).family(bold()).color(TEXT));
                                            match (&audio, d.active) {
                                                (Some(_), true) => chip(ui, "Áudio HDMI", true),
                                                (_, true) => chip(ui, "Ativa", true),
                                                _ => chip(ui, "Desligada", false),
                                            }
                                        });
                                        let vendor = model(&d.name).0;
                                        let sub = [d.gdi.trim_start_matches(r"\\.\"), d.kind.as_str(), vendor].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join(" · ");
                                        ui.label(RichText::new(if d.active { sub } else { format!("desligada · {}", d.kind) }).size(12.0).color(MUTED));
                                    });
                                };
                                let mut buttons = |ui: &mut egui::Ui| {
                                    let mut on = d.active;
                                    let last = d.active && active.len() == 1;
                                    let t = toggle(ui, &mut on, !busy && !last, &d.name);
                                    let t = if last { t.on_hover_text("Precisa ficar pelo menos uma tela ativa") } else { t.on_hover_text("Liga/desliga esta tela") };
                                    if t.changed() {
                                        let mut w = active.clone();
                                        if on { w.push(d.id.clone()) } else { w.retain(|x| *x != d.id) }
                                        want = Some(w);
                                    }
                                    let so = egui::Button::new(RichText::new("Só esta").size(12.5)).min_size(egui::vec2(0.0, 30.0));
                                    if ui.add_enabled(!busy, so).on_hover_text("Deixa só esta tela ligada (como o Win+P)").clicked() {
                                        want = Some(vec![d.id.clone()]);
                                    }
                                };
                                // narrow card (small window, TV column): buttons get their own row instead of overlapping the name
                                if ui.available_width() < 380.0 {
                                    title(ui);
                                    ui.add_space(4.0);
                                    ui.horizontal(|ui| buttons(ui));
                                } else {
                                    ui.horizontal(|ui| {
                                        title(ui);
                                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| buttons(ui));
                                    });
                                }
                                ui.add_space(4.0);
                                ui.separator();
                                // ponytail: matched by name, two identical monitors would share a card
                                let mut mons = self.ddc(&d.name).peekable();
                                if mons.peek().is_none() {
                                    ui.add_space(8.0);
                                    if !d.active {
                                        ui.label(RichText::new("Tela desligada. Ligue no botão acima.").color(MUTED));
                                    } else {
                                        ui.label(RichText::new("Sem controle de imagem").size(15.0).family(bold()).color(SOFT));
                                        ui.label(RichText::new("TVs normalmente não aceitam DDC/CI pelo cabo. Ligar/desligar a tela e o áudio funcionam normalmente.").color(MUTED));
                                    }
                                    if let Some((id, n)) = &audio {
                                        ui.add_space(6.0);
                                        if *id == self.audio_def {
                                            chip(ui, "Áudio ativo nesta tela", true);
                                        } else if ui.button("Usar o áudio desta tela").clicked() {
                                            set_audio = Some((id.clone(), n.clone()));
                                        }
                                    }
                                }
                                for m in mons {
                                    controls(ui, m);
                                    save_bar(ui, m);
                                }
                            });
                        });
                    }
                });
            });
        });
        if let Some((id, n)) = set_audio {
            self.set_audio(&id, &n);
        }
        if let Some(w) = want {
            let (st, busy, rv, ctx) = (self.status.clone(), self.busy.clone(), self.revert.clone(), ui.ctx().clone());
            busy.store(true, Ordering::Relaxed);
            *st.lock().unwrap() = "aplicando…".into();
            self.reload = true;
            let risky = !w.iter().any(|x| active.contains(x));
            spawn(move || {
                let r = apply(&w);
                if r.is_ok() && risky {
                    *rv.lock().unwrap() = Some((Instant::now() + Duration::from_secs(15), active));
                }
                *st.lock().unwrap() = r.unwrap_or_else(|e| format!("erro: {e}"));
                busy.store(false, Ordering::Relaxed);
                ctx.request_repaint();
            });
        }
    }
}

fn main() {
    std::panic::set_hook(Box::new(|info| {
        log(format!("PANIC {info}\n{}", std::backtrace::Backtrace::force_capture()));
        eprintln!("monctl: {info} (detalhes em {})", app_dir().join("monctl.log").display());
    }));
    sys::crash_handler();
    let args: Vec<String> = std::env::args().skip(1).collect();
    log(format!("monctl {} ({}) args={args:?}", env!("CARGO_PKG_VERSION"), std::env::consts::OS));
    // --min: started by "iniciar com o sistema"
    let min = args.iter().any(|a| a == "--min");
    if !args.is_empty() && !min {
        if let Err(e) = cli(&args).logged(&format!("cli {args:?}")) {
            eprintln!("{e}");
            std::process::exit(1);
        }
        return;
    }
    sys::hide_console();
    let mut gui = Gui {
        displays: vec![],
        mons: vec![],
        status: Default::default(),
        busy: Default::default(),
        reload: true,
        styled: false,
        revert: Default::default(),
        audio: vec![],
        audio_def: String::new(),
        volume: 0.0,
        autostart: sys::autostart(),
        start_min: min,
    };
    let opts = eframe::NativeOptions { viewport: egui::ViewportBuilder::default().with_inner_size([1720.0, 900.0]).with_min_inner_size([900.0, 560.0]), ..Default::default() };
    let r = eframe::run_ui_native("monctl", opts, move |ui, _| gui.ui(ui));
    if let Err(e) = r {
        log(format!("ERRO interface: {e}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caps_parser() {
        // trimmed from the real AW2525HM string
        let c = "(prot(monitor)cmds(01 02 03 0C E3 F3)vcp(02 10 14(01 05 08 0B 0C) 60( 0F 11 12) E2(00 04 0E)E3 EC(FF02) ED(0400 0800 03FF) F5(100 500) E0FF)mccs_ver(2.1))";
        let v = parse_vcp(c);
        assert_eq!(v.iter().map(|x| x.0).collect::<Vec<_>>(), [0x02, 0x10, 0x14, 0x60, 0xE2, 0xE3, 0xEC, 0xED, 0xF5, 0xE0, 0xFF]);
        assert_eq!(v[2].1, [1, 5, 8, 0x0B, 0x0C]);
        assert_eq!(v[3].1, [0x0F, 0x11, 0x12]);
        assert!(v[5].1.is_empty());
        assert_eq!(v[6].1, [0xFF02]);
        assert_eq!(v[7].1, [0x400, 0x800, 0x3FF]);
        assert_eq!(v[8].1, [0x100, 0x500]);
    }

    #[test]
    fn edid() {
        let mut e = [0u8; 128];
        e[..8].copy_from_slice(&[0, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0]);
        e[8..10].copy_from_slice(&[0x10, 0xAC]); // DEL
        e[10..12].copy_from_slice(&[0x8F, 0x43]);
        assert_eq!(edid_name(&e).unwrap(), "DEL438F");
        e[72..77].copy_from_slice(&[0, 0, 0, 0xFC, 0]);
        e[77..86].copy_from_slice(b"AW2525HM\n");
        assert_eq!(edid_name(&e).unwrap(), "AW2525HM");
    }

    #[test]
    fn pending_roundtrip() {
        let f = |op, cur, saved| Feat { op, name: String::new(), cur, saved, max: 100, values: vec![] };
        let name = "teste-pendente";
        write_pending(name, &[f(Op::Vcp(0x10), 70, 20), f(Op::Aoc(0x51, 0), 3, 3), f(Op::DellQ(0xF4, 0x2F), 1, 2)]);
        let mut fresh = [f(Op::Vcp(0x10), 70, 70), f(Op::Aoc(0x51, 0), 3, 3), f(Op::DellQ(0xF4, 0x2F), 1, 1)];
        read_pending(name, &mut fresh);
        assert_eq!(fresh.iter().map(|f| f.saved).collect::<Vec<_>>(), [20, 3, 2]);
        write_pending(name, &fresh.map(|x| f(x.op, x.saved, x.saved)));
        assert!(!pending_path(name).exists(), "nothing unsaved -> file removed");
    }

    /// Touches the real HKCU\...\Run / ~/.config/autostart: `cargo test -- --ignored autostart`
    #[test]
    #[ignore]
    fn autostart() {
        sys::set_autostart(true).unwrap();
        assert!(sys::autostart());
        sys::set_autostart(false).unwrap();
        assert!(!sys::autostart());
    }

    #[test]
    fn framing() {
        // MCCS example: get brightness = 51 82 01 10 AC
        assert_eq!(ddc_frame(&[0x01, 0x10]), [0x51, 0x82, 0x01, 0x10, 0xAC]);
        let mut r = [0x6E, 0x88, 0x02, 0x00, 0x10, 0x00, 0x00, 0x64, 0x00, 0x32, 0];
        assert_eq!(ddc_reply(&r, &[0x10]), Some((50, 100)));
        r[3] = 1; // rc error
        assert_eq!(ddc_reply(&r, &[0x10]), None);
        // real 25G3ZM replies: overdrive, and a stale overdrive buffer returned for unsupported 0x71
        let od = [0x6E, 0x88, 0x02, 0x00, 0xF8, 0x51, 0x00, 0x03, 0x00, 0x03, 0x1D];
        assert_eq!(ddc_reply(&od, &[0xF8, 0xE1, 0x51]), Some((3, 3)));
        assert_eq!(ddc_reply(&od, &[0xF8, 0xE1, 0x71]), None);
        assert_eq!(opcode("f8e151").unwrap(), [0xF8, 0xE1, 0x51]);
        assert_eq!(opcode("0x10").unwrap(), [0x10]);
        assert!(now().len() == 24);
    }
}
