use crate::log::{Logged, log};
use crate::platform;

/// Activate exactly `wanted`. Audio is never switched here: only when the user picks an output.
pub(crate) fn apply(wanted: &[String]) -> Result<String, String> {
    log(format!("aplicar telas: {wanted:?}"));
    let result = apply_inner(wanted).logged("aplicar telas");
    if let Ok(message) = &result {
        log(message);
    }
    result
}

fn apply_inner(wanted: &[String]) -> Result<String, String> {
    if wanted.is_empty() {
        return Err("pelo menos uma tela precisa ficar ativa".into());
    }
    let before = platform::displays()?;
    platform::set_active(wanted)?;
    // safety lock: never leave the machine with zero active outputs
    if !platform::displays()?.iter().any(|display| display.active) {
        let previously_active: Vec<String> = before.iter().filter(|display| display.active).map(|display| display.id.clone()).collect();
        platform::set_active(&previously_active).logged("reverter telas")?;
        return Err("nenhuma tela ficou ativa, configuração revertida".into());
    }
    max_refresh_on_newly_active(&before);
    Ok("telas aplicadas".into())
}

/// Windows turns an output back on at 60 Hz; bump each newly active one to its highest rate.
/// Outputs that were already on keep whatever Hz the user picked.
fn max_refresh_on_newly_active(before: &[platform::Display]) {
    let Ok(after) = platform::displays().logged("listar telas") else { return };
    let was_active = |id: &str| before.iter().any(|display| display.id == id && display.active);
    for display in after.iter().filter(|display| display.active && !was_active(&display.id)) {
        let Ok((current, rates)) = platform::refresh_rates(&display.gdi).logged(&format!("taxas de {}", display.name)) else { continue };
        if let Some(&highest) = rates.last().filter(|&&highest| highest > current) {
            let _ = platform::set_refresh_rate(&display.gdi, highest).logged(&format!("taxa de {}", display.name));
        }
    }
}
