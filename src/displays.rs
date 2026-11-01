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
    Ok("telas aplicadas".into())
}
