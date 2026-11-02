// Refresh rate (Hz) of an output through xrandr.
use super::run;
use crate::log::log;

// ponytail: X11 only; Wayland needs kscreen-doctor / wlr-randr mode strings (WxH@R) per compositor.
fn x11_only() -> Result<(), String> {
    if std::env::var_os("WAYLAND_DISPLAY").is_some() { Err("trocar Hz no Wayland ainda não é suportado".into()) } else { Ok(()) }
}

/// (current Hz, every Hz of the current resolution), rounded: "1920x1080  240.00*+  144.00  60.00".
pub(crate) fn refresh_rates(output: &str) -> Result<(u32, Vec<u32>), String> {
    x11_only()?;
    let text = run("xrandr", &[])?;
    let mut lines = text.lines().skip_while(|line| line.split(' ').next() != Some(output)).skip(1).take_while(|line| line.starts_with(char::is_whitespace));
    let current_mode = lines.find(|line| line.contains('*')).ok_or(format!("{output}: sem modo atual"))?;
    let (mut current, mut rates) = (0, vec![]);
    for token in current_mode.split_whitespace().skip(1) {
        let Ok(rate) = token.trim_end_matches(['*', '+']).parse::<f32>() else { continue };
        let rate = rate.round() as u32;
        if token.contains('*') {
            current = rate;
        }
        rates.push(rate);
    }
    rates.sort_unstable();
    rates.dedup();
    Ok((current, rates))
}

pub(crate) fn set_refresh_rate(output: &str, hertz: u32) -> Result<(), String> {
    x11_only()?;
    run("xrandr", &["--output", output, "--rate", &hertz.to_string()])?;
    log(format!("{output}: {hertz} Hz"));
    Ok(())
}
