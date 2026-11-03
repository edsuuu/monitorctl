// Display profiles ("Só TV", "Monitores"): which outputs are on. Stored by display name, because
// the CCD ids carry the GPU adapter LUID, which changes on every boot.
use crate::log::app_folder;
use crate::platform::Display;
use std::path::PathBuf;

/// (profile name, names of the displays that stay on)
pub(crate) type Profile = (String, Vec<String>);

fn path() -> PathBuf {
    app_folder().join("perfis.txt")
}

/// One profile per line: "Só TV\tSAMSUNG".
pub(crate) fn load() -> Vec<Profile> {
    let text = std::fs::read_to_string(path()).unwrap_or_default();
    text.lines().filter_map(|line| line.split_once('\t')).map(|(name, displays)| (name.to_string(), displays.split(',').map(str::to_string).collect())).collect()
}

pub(crate) fn save(profiles: &[Profile]) -> Result<(), String> {
    let text: String = profiles.iter().map(|(name, displays)| format!("{name}\t{}\n", displays.join(","))).collect();
    std::fs::write(path(), text).map_err(|error| error.to_string())
}

/// Saves (or replaces) `name` with the displays that are on now.
pub(crate) fn save_current(name: &str, displays: &[Display]) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() || name.contains('\t') {
        return Err("nome de perfil inválido".into());
    }
    let mut profiles = load();
    profiles.retain(|profile| profile.0 != name);
    profiles.push((name.to_string(), displays.iter().filter(|display| display.active).map(|display| display.name.clone()).collect()));
    save(&profiles)
}

pub(crate) fn find(query: &str) -> Result<Profile, String> {
    let (query, profiles) = (query.to_lowercase(), load());
    let exact = profiles.iter().find(|profile| profile.0.to_lowercase() == query);
    let names: Vec<&str> = profiles.iter().map(|profile| profile.0.as_str()).collect();
    exact.or_else(|| profiles.iter().find(|profile| profile.0.to_lowercase().contains(&query))).cloned().ok_or(format!("perfil '{query}' não existe; perfis: {}", names.join(", ")))
}

/// Ids of the profile's displays among the ones connected now.
pub(crate) fn display_ids(profile: &Profile, displays: &[Display]) -> Result<Vec<String>, String> {
    profile.1.iter().map(|name| displays.iter().find(|display| display.name == *name).map(|display| display.id.clone()).ok_or(format!("a tela {name} do perfil {} não está conectada", profile.0))).collect()
}
