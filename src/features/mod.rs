mod tables;

use crate::ddc::parse_vcp;
use crate::log::{Logged, app_folder, log};
use crate::platform;
use std::thread::sleep;
use std::time::Duration;
use tables::{ALIENWARE, AOC, DELL_PRESET_WRITE, MCCS};

// Variant names (and the DellBits field names) are written to pendente-*.txt through `{:?}`: don't rename them.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub(crate) enum Operation {
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
    /// Not DDC: NVIDIA Digital Vibrance, applied by the GPU to the output signal (works on the TV too).
    Vibrance,
}

/// (op, label, named values). Empty values = slider. Empty label = hide this code.
pub(crate) type Definition = (Operation, &'static str, &'static [(u16, &'static str)]);

/// (vendor, vendor-specific controls) by EDID model name.
pub(crate) fn model(model: &str) -> (&'static str, &'static [Definition]) {
    // ponytail: exact model names; other AOC (G-Menu) / Alienware models likely share these tables
    match model {
        "AW2525HM" => ("Alienware", ALIENWARE),
        "25G3ZM" => ("AOC", AOC),
        _ => ("", &[]),
    }
}

pub(crate) fn known(name: &str) -> Vec<Definition> {
    let model_controls = model(name).1;
    // model entries override generic ones with the same op
    model_controls.iter().chain(MCCS.iter().filter(|generic| !model_controls.iter().any(|specific| specific.0 == generic.0))).copied().collect()
}

fn hdr_names(maximum: u16) -> &'static [(u16, &'static str)] {
    match maximum {
        0 | 1 => &[(0, "Desligado"), (1, "Auto")],
        3 => &[(0, "Desligado"), (1, "Picture"), (2, "Movie"), (3, "Game")],
        _ => &[(0, "Desligado"), (1, "Display"), (2, "Picture"), (3, "Movie"), (4, "Game"), (5, "Color"), (6, "Peak")],
    }
}

/// The capabilities code that must be listed for this op to be shown (None = probe instead).
fn gate(operation: Operation) -> Option<u8> {
    match operation {
        Operation::Vcp(code) | Operation::DellQ(code, _) | Operation::DellBits { code, .. } => Some(code),
        Operation::DellPreset => Some(0xE2),
        Operation::DellVision => Some(0xEC),
        Operation::Aoc(..) | Operation::Vibrance => None,
    }
}

const STEP: Duration = Duration::from_millis(200); // Dell: pause between selector write and read

pub(crate) fn read(monitor: &platform::Monitor, operation: Operation) -> Result<(u16, u16), String> {
    match operation {
        Operation::Vcp(code) => monitor.get(code),
        Operation::Aoc(subcode, _) => monitor.raw_get(&[0xF8, 0xE1, subcode]),
        Operation::DellQ(code, selector) => {
            monitor.set(code, selector)?;
            sleep(STEP);
            monitor.get(code).map(|(value, maximum)| (value & 0xFF, maximum))
        }
        Operation::DellBits { code, sel, shift, mask, .. } => {
            monitor.set(code, sel)?;
            sleep(STEP);
            monitor.get(code).map(|(value, _)| ((value >> shift) & mask, mask))
        }
        Operation::DellPreset => monitor.get(0xE2).map(|(value, maximum)| (value & 0xFF, maximum)),
        Operation::DellVision => monitor.get(0xEC).map(|(value, _)| (if value & 0xC000 == 0xC000 { (value & 0x3000) >> 12 } else { 0 }, 3)),
        Operation::Vibrance => Err("saturação da GPU não é DDC/CI".into()),
    }
}

pub(crate) fn write(monitor: &platform::Monitor, operation: Operation, value: u16) -> Result<(), String> {
    match operation {
        Operation::Vcp(code) | Operation::DellQ(code, _) => monitor.set(code, value),
        Operation::Aoc(subcode, _) => monitor.raw_set(&[0xF8, 0xE1, subcode], value),
        Operation::DellBits { code, sel, set, keep, shift, mask } => {
            monitor.set(code, sel)?;
            sleep(STEP);
            let (current, _) = monitor.get(code)?;
            sleep(STEP);
            monitor.set(code, (current & keep & !(mask << shift)) | ((value & mask) << shift) | sel | set)
        }
        Operation::DellPreset => {
            let &(_, code, code_value) = DELL_PRESET_WRITE.iter().find(|preset| preset.0 == value).ok_or(format!("preset {value} sem comando de escrita"))?;
            monitor.set(code, code_value)
        }
        Operation::DellVision if value == 0 => {
            let (current, _) = monitor.get(0xEC)?;
            monitor.set(0xEC, 0x8000 | (current & 0x3000))
        }
        Operation::DellVision => monitor.set(0xEC, 0xC000 | (value & 3) << 12),
        Operation::Vibrance => Err("saturação da GPU não é DDC/CI".into()),
    }
}

#[derive(Clone)]
pub(crate) struct Feature {
    pub(crate) operation: Operation,
    pub(crate) name: String,
    pub(crate) current: u16,
    /// last saved value: `current` differs until the user clicks Salvar (or Desfazer brings it back)
    pub(crate) saved: u16,
    pub(crate) maximum: u16,
    pub(crate) values: Vec<(u16, String)>,
}

/// Read every supported feature of a monitor. Ok((features, unmapped capability codes)).
/// `progress` gets the list after each feature, so the GUI fills in while the slow DDC reads go on.
pub(crate) fn load(monitor: &platform::Monitor, progress: &mut dyn FnMut(&[Feature])) -> Result<(Vec<Feature>, String), String> {
    let known_controls = known(&monitor.name);
    // the capabilities string never changes for a monitor and takes ~3 s to read over DDC/CI: cache it
    let capabilities_cache = app_folder().join(format!("caps-{}.txt", monitor.name));
    let capabilities = match std::fs::read_to_string(&capabilities_cache) {
        Ok(cached) => Ok(cached),
        Err(_) => monitor.capabilities().inspect(|capabilities| {
            _ = std::fs::write(&capabilities_cache, capabilities).map_err(|error| error.to_string()).logged("salvar cache de capabilities");
        }),
    };
    let codes = match capabilities.logged(&format!("{} caps", monitor.name)) {
        Ok(capabilities) => {
            log(format!("{} caps: {capabilities}", monitor.name));
            Some(parse_vcp(&capabilities))
        }
        // no capabilities string: show whatever answers, but bail out fast if nothing does
        Err(error) => monitor.get(0x10).logged(&format!("{} sonda 0x10", monitor.name)).map_err(|_| error).map(|_| None)?,
    };
    let aoc_mask = known_controls
        .iter()
        .any(|definition| matches!(definition.0, Operation::Aoc(..)))
        .then(|| monitor.raw_get(&[0xF8, 0xE1, 0x00]).logged(&format!("{} AOC F8E100", monitor.name)).ok())
        .flatten()
        .map(|(current, _)| current);
    if let Some(mask) = aoc_mask {
        log(format!("{} AOC máscara de suporte = 0x{mask:04X}", monitor.name));
    }
    let mut features = vec![];
    for &(operation, name, names) in &known_controls {
        let listed = match (gate(operation), &codes) {
            (Some(code), Some(listed_codes)) => listed_codes.iter().find(|entry| entry.0 == code).map(|entry| entry.1.clone()),
            (Some(_), None) => Some(vec![]),
            (None, _) => match operation {
                Operation::Aoc(_, bit) => aoc_mask.filter(|mask| bit == 0 || mask & bit != 0).map(|_| vec![]),
                _ => None,
            },
        };
        let (Some(listed), false) = (listed, name.is_empty()) else { continue };
        let Ok((mut current, maximum)) = read(monitor, operation).logged(&format!("{} ler {operation:?}", monitor.name)) else { continue };
        let names = if let Operation::Aoc(0x52, _) = operation { hdr_names(maximum) } else { names };
        let mut options: Vec<u16> = if listed.is_empty() || !matches!(operation, Operation::Vcp(_)) { names.iter().map(|named| named.0).collect() } else { listed };
        if !options.contains(&current) && options.contains(&(current & 0xFF)) {
            current &= 0xFF; // Dell answers input 0x0F as 0x0F0F
        }
        if !options.is_empty() && !options.contains(&current) {
            options.push(current);
        }
        let label = |value: u16| names.iter().find(|named| named.0 == value).map_or(format!("0x{value:02X}"), |named| named.1.to_string());
        let values = options.into_iter().map(|value| (value, label(value))).collect();
        features.push(Feature { operation, name: name.into(), current, saved: current, maximum, values });
        progress(&features);
    }
    let unknown = codes
        .unwrap_or_default()
        .iter()
        .filter(|(code, _)| !known_controls.iter().any(|definition| gate(definition.0) == Some(*code)))
        .map(|(code, _)| format!("{code:02X}"))
        .collect::<Vec<_>>()
        .join(" ");
    log(format!("{}: {} controles, sem nome: {unknown}", monitor.name, features.len()));
    Ok((features, unknown))
}
