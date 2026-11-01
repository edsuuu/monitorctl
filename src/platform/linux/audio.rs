// ---------- audio ----------

use super::run;

/// (sink name, description + any product/nick names, so the TV name from ELD can match)
pub(crate) fn audio_outputs() -> Result<Vec<(String, String)>, String> {
    let listing = run("pactl", &["list", "sinks"])?;
    Ok(listing
        .split("Sink #")
        .skip(1)
        .filter_map(|block| {
            let field = |key: &str| block.lines().find_map(|line| line.trim().strip_prefix(key).map(|value| value.trim().trim_matches('"').to_string()));
            let mut label = field("Description:")?;
            for key in ["device.product.name = ", "node.nick = "] {
                if let Some(value) = field(key) {
                    label += &format!(" [{value}]");
                }
            }
            Some((field("Name:")?, label))
        })
        .collect())
}

pub(crate) fn default_audio() -> Result<String, String> {
    Ok(run("pactl", &["get-default-sink"])?.trim().to_string())
}

pub(crate) fn set_default_audio(id: &str) -> Result<(), String> {
    run("pactl", &["set-default-sink", id]).map(|_| ())
}

/// Volume of the default sink, 0.0..=1.0 ("Volume: front-left: 65536 / 100% / 0.00 dB, ...").
pub(crate) fn volume() -> Result<f32, String> {
    let text = run("pactl", &["get-sink-volume", "@DEFAULT_SINK@"])?;
    text.split('/').nth(1).and_then(|percent| percent.trim().trim_end_matches('%').parse::<f32>().ok()).map(|percent| percent / 100.0).ok_or(format!("volume ilegível: {text}"))
}

pub(crate) fn set_volume(level: f32) -> Result<(), String> {
    run("pactl", &["set-sink-volume", "@DEFAULT_SINK@", &format!("{:.0}%", level.clamp(0.0, 1.0) * 100.0)]).map(|_| ())
}
