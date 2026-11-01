use crate::ddc::opcode;
use crate::displays::apply;
use crate::features::{Definition, known, load, read, write};
use crate::log::{Logged, log};
use crate::platform;

const USAGE: &str = "monitorctl                         abre a interface
monitorctl list                    telas, monitores DDC/CI e saídas de áudio
monitorctl on|off|only TELA..      liga/desliga telas (TELA = parte do nome, ex: samsung)
monitorctl all                     liga todas as telas conectadas
monitorctl caps MONITOR            capabilities DDC/CI + todos os controles e valores
monitorctl get MONITOR CONTROLE    CONTROLE = nome (brilho, preset...) ou código: 10 | AOC f8e151
monitorctl set MONITOR CONTROLE V  V = número ou nome da opção, ex: set aw brilho 30 | set aw preset fps
monitorctl sat TELA [0-100]        saturação pela placa de vídeo (NVIDIA), funciona na TV
monitorctl audio SAIDA             define a saída de áudio padrão
monitorctl mcp                     servidor MCP (stdio) para o Claude controlar tudo isso
log: %APPDATA%\\monitorctl\\monitorctl.log  (Linux: ~/.config/monitorctl/monitorctl.log)";

fn find_monitor(query: &str) -> Result<platform::Monitor, String> {
    let query = query.to_lowercase();
    platform::monitors().into_iter().find(|monitor| monitor.name.to_lowercase().contains(&query)).ok_or(format!("monitor '{query}' não encontrado"))
}

fn find_display(query: &str) -> Result<platform::Display, String> {
    let query = query.to_lowercase();
    platform::displays()?.into_iter().find(|display| display.name.to_lowercase().contains(&query) || display.id == query).ok_or(format!("tela '{query}' não encontrada"))
}

/// A named control of this monitor ("brilho", "preset"): exact name first, then partial.
fn find_control(monitor: &platform::Monitor, query: &str) -> Option<Definition> {
    let (query, known_controls) = (query.to_lowercase(), known(&monitor.name));
    let exact = known_controls.iter().find(|definition| definition.1.to_lowercase() == query);
    exact.or_else(|| known_controls.iter().find(|definition| !definition.1.is_empty() && definition.1.to_lowercase().contains(&query))).copied()
}

/// "30" or an option name ("fps", "desligado").
fn parse_value(definition: &Definition, text: &str) -> Result<u16, String> {
    let query = text.to_lowercase();
    let named = || definition.2.iter().find(|option| option.1.to_lowercase() == query).or_else(|| definition.2.iter().find(|option| option.1.to_lowercase().contains(&query))).map(|option| option.0);
    text.parse().ok().or_else(named).ok_or_else(|| {
        let options: Vec<&str> = definition.2.iter().map(|option| option.1).collect();
        format!("valor '{text}' inválido para {}; opções: {}", definition.1, options.join(", "))
    })
}

fn option_label(definition: &Definition, value: u16) -> String {
    definition.2.iter().find(|option| option.0 == value).map_or(String::new(), |option| format!(" ({})", option.1))
}

/// Runs a command, returning what it would print.
pub(crate) fn run(arguments: &[String]) -> Result<String, String> {
    let argument = |index: usize| arguments.get(index).map(String::as_str).ok_or(USAGE.to_string());
    let mut output = String::new();
    match arguments[0].as_str() {
        "list" => {
            for display in platform::displays()? {
                output += &format!("{} {:<12} {:<14} {:<12} {}\n", if display.active { "●" } else { "○" }, display.name, display.gdi, display.kind, display.id);
            }
            for monitor in platform::monitors() {
                output += &format!("ddc: {}\n", monitor.name);
            }
            let default_output = platform::default_audio().logged("áudio padrão").unwrap_or_default();
            for (id, name) in platform::audio_outputs()? {
                output += &format!("áudio: {} {name}\n", if id == default_output { "*" } else { " " });
            }
        }
        "on" | "off" | "only" | "all" => {
            let displays = platform::displays()?;
            let mut wanted: Vec<String> = displays.iter().filter(|display| display.active).map(|display| display.id.clone()).collect();
            let mut picked = vec![];
            for query in &arguments[1..] {
                let query = query.to_lowercase();
                picked.push(displays.iter().find(|display| display.name.to_lowercase().contains(&query) || display.id == query).ok_or(format!("tela '{query}' não encontrada"))?.id.clone());
            }
            match arguments[0].as_str() {
                "on" => wanted.extend(picked.into_iter().filter(|id| !wanted.contains(id)).collect::<Vec<_>>()),
                "off" => wanted.retain(|id| !picked.contains(id)),
                "only" => wanted = picked,
                _ => wanted = displays.iter().map(|display| display.id.clone()).collect(),
            }
            output += &apply(&wanted)?;
        }
        "caps" => {
            let monitor = find_monitor(argument(1)?)?;
            output += &format!("{}\n\n", monitor.capabilities().unwrap_or_else(|error| format!("(sem capabilities: {error})")));
            let (features, unknown) = load(&monitor, &mut |_| {})?;
            for feature in features {
                let label = feature.values.iter().find(|value| value.0 == feature.current).map_or(String::new(), |value| format!(" ({})", value.1));
                let options = feature.values.iter().map(|value| value.1.as_str()).collect::<Vec<_>>().join(" | ");
                output += &format!("{:<26} {:>5}{label:<22} max {:<5} {options}\n", feature.name, feature.current, feature.maximum);
            }
            output += &format!("\ncódigos sem nome: {unknown}");
        }
        "get" => {
            let monitor = find_monitor(argument(1)?)?;
            if let Some(definition) = find_control(&monitor, argument(2)?) {
                let (current, maximum) = read(&monitor, definition.0)?;
                output += &format!("{} = {current}{} / max {maximum}", definition.1, option_label(&definition, current));
            } else {
                let opcode = opcode(argument(2)?)?;
                let (current, maximum) = if opcode.len() == 1 { monitor.get(opcode[0]) } else { monitor.raw_get(&opcode) }?;
                output += &format!("{current} (0x{current:04X}) / max {maximum}");
            }
        }
        "set" => {
            let monitor = find_monitor(argument(1)?)?;
            if let Some(definition) = find_control(&monitor, argument(2)?) {
                let value = parse_value(&definition, argument(3)?)?;
                log(format!("cli set {} {} = {value}", monitor.name, definition.1));
                write(&monitor, definition.0, value)?;
                output += &format!("{} {} = {value}{}", monitor.name, definition.1, option_label(&definition, value));
            } else {
                let opcode = opcode(argument(2)?)?;
                let value: u16 = argument(3)?.parse().map_err(|_| "valor inválido")?;
                log(format!("cli set {} {opcode:02X?} = {value}", monitor.name));
                if opcode.len() == 1 { monitor.set(opcode[0], value) } else { monitor.raw_set(&opcode, value) }?;
                output += &format!("{} {opcode:02X?} = {value}", monitor.name);
            }
        }
        "sat" => {
            let display = find_display(argument(1)?)?;
            if let Some(value) = arguments.get(2) {
                let value: u16 = value.parse().ok().filter(|value| *value <= 100).ok_or("saturação vai de 0 a 100 (50 = padrão)")?;
                log(format!("cli saturação GPU {} = {value}", display.name));
                platform::set_vibrance(&display.gdi, value)?;
            }
            output += &format!("{} saturação (GPU) = {}", display.name, platform::vibrance(&display.gdi)?);
        }
        "audio" => {
            let query = argument(1)?.to_lowercase();
            let outputs = platform::audio_outputs()?;
            let (id, name) = outputs.iter().find(|audio_output| audio_output.1.to_lowercase().contains(&query)).ok_or(format!("áudio '{query}' não encontrado"))?;
            platform::set_default_audio(id)?;
            log(format!("áudio → {name}"));
            output += &format!("áudio → {name}");
        }
        "mcp" => crate::mcp::serve()?,
        _ => return Err(USAGE.into()),
    }
    Ok(output)
}
