// ---------- iniciar com o sistema: XDG autostart ----------

use std::fs;
use std::path::PathBuf;

fn autostart_file() -> PathBuf {
    crate::log::app_folder().with_file_name("autostart").join("monitorctl.desktop")
}

pub(crate) fn autostart() -> bool {
    autostart_file().exists()
}

pub(crate) fn set_autostart(on: bool) -> Result<(), String> {
    let file = autostart_file();
    if !on {
        return fs::remove_file(&file).map_err(|error| error.to_string());
    }
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    fs::create_dir_all(file.parent().unwrap_or(&file)).map_err(|error| error.to_string())?;
    fs::write(&file, format!("[Desktop Entry]\nType=Application\nName=monitorctl\nExec=\"{}\" --min\nX-GNOME-Autostart-enabled=true\n", executable.display())).map_err(|error| error.to_string())
}
