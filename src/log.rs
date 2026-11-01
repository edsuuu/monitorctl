use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) fn app_folder() -> PathBuf {
    let base = std::env::var("APPDATA")
        .or_else(|_| std::env::var("XDG_CONFIG_HOME"))
        .unwrap_or_else(|_| format!("{}/.config", std::env::var("HOME").unwrap_or_default()));
    let folder = PathBuf::from(&base).join("monitorctl");
    // the app was called monctl: move its folder (log, saved values, caches) over once
    let old_folder = PathBuf::from(&base).join("monctl");
    if !folder.exists() && old_folder.exists() {
        _ = std::fs::rename(&old_folder, &folder);
        _ = std::fs::rename(folder.join("monctl.log"), folder.join("monitorctl.log"));
    }
    folder
}

pub(crate) fn now() -> String {
    let elapsed = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    let (days, seconds) = ((elapsed.as_secs() / 86400) as i64, elapsed.as_secs() % 86400);
    // civil-from-days (H. Hinnant)
    let shifted_days = days + 719468;
    let (era, day_of_era) = (shifted_days.div_euclid(146097), shifted_days.rem_euclid(146097));
    let year_of_era = (day_of_era - day_of_era / 1460 + day_of_era / 36524 - day_of_era / 146096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_from_march = (5 * day_of_year + 2) / 153;
    let (day, month) = (day_of_year - (153 * month_from_march + 2) / 5 + 1, if month_from_march < 10 { month_from_march + 3 } else { month_from_march - 9 });
    let year = year_of_era + era * 400 + (month <= 2) as i64;
    format!("{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02}.{:03}Z", seconds / 3600, seconds / 60 % 60, seconds % 60, elapsed.subsec_millis())
}

/// Append a line to monitorctl.log (rotated at 1 MB). Never fails.
pub(crate) fn log(message: impl AsRef<str>) {
    static LOCK: Mutex<()> = Mutex::new(());
    let _guard = LOCK.lock();
    let path = app_folder().join("monitorctl.log");
    _ = std::fs::create_dir_all(app_folder());
    if std::fs::metadata(&path).is_ok_and(|metadata| metadata.len() > 1 << 20) {
        _ = std::fs::rename(&path, path.with_extension("log.old"));
    }
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        _ = writeln!(file, "{} [{:?}] {}", now(), std::thread::current().id(), message.as_ref());
    }
}

pub(crate) trait Logged {
    fn logged(self, context: &str) -> Self;
}

impl<Success> Logged for Result<Success, String> {
    fn logged(self, context: &str) -> Self {
        if let Err(error) = &self {
            log(format!("ERRO {context}: {error}"));
        }
        self
    }
}
