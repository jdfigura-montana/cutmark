//! The discovery files under `%APPDATA%\Cutmark\remote\`: `<app>.json` while the app
//! runs and `apps.json`, which persists so the plugin can launch an app that is not
//! running (spec: Remote Control "Discovery files").

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::protocol::PROTOCOL;

/// The Cutmark config folder for this OS: the same rules as ui-kit's appearance file
/// (the folder function there is private, so the logic lives here too).
fn config_folder() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        std::env::var_os("APPDATA").map(|base| PathBuf::from(base).join("Cutmark"))
    }
    #[cfg(target_os = "macos")]
    {
        std::env::var_os("HOME").map(|home| PathBuf::from(home).join("Library").join("Application Support").join("Cutmark"))
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        if let Some(base) = std::env::var_os("XDG_CONFIG_HOME") {
            let base = PathBuf::from(base);
            if base.is_absolute() {
                return Some(base.join("cutmark"));
            }
        }
        std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config").join("cutmark"))
    }
}

/// The discovery folder: the configured dir when set, else `<Cutmark folder>\remote`.
pub(crate) fn folder(configured: Option<&Path>) -> Option<PathBuf> {
    if let Some(dir) = configured {
        return Some(dir.to_path_buf());
    }
    config_folder().map(|folder| folder.join("remote"))
}

/// The pid recorded in `<app>.json`, if that file exists and holds a number.
pub(crate) fn existing_pid(app_file: &Path) -> Option<u32> {
    let text = fs::read_to_string(app_file).ok()?;
    serde_json::from_str::<Value>(&text).ok()?.get("pid")?.as_u64().map(|pid| pid as u32)
}

/// Writes `<app>.json` atomically: the port and token a plugin needs to connect.
pub(crate) fn write_app_file(dir: &Path, app: &str, pid: u32, port: u16, token: &str, version: &str) -> std::io::Result<()> {
    let file = json!({
        "app": app,
        "pid": pid,
        "port": port,
        "token": token,
        "protocol": PROTOCOL,
        "version": version,
    });
    write_atomic(&dir.join(format!("{app}.json")), &file)
}

/// Removes `<app>.json`; a missing file is fine.
pub(crate) fn remove_app_file(dir: &Path, app: &str) {
    let _ = fs::remove_file(dir.join(format!("{app}.json")));
}

/// Adds this app's exe and version to `apps.json`, keeping every other app's entry.
/// A corrupt file is replaced by a fresh object.
pub(crate) fn update_apps_file(dir: &Path, app: &str, exe: &Path, version: &str) -> std::io::Result<()> {
    let path = dir.join("apps.json");
    let mut apps = fs::read_to_string(&path)
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default();
    apps.insert(app.to_string(), json!({ "exe": exe.to_string_lossy(), "version": version }));
    write_atomic(&path, &Value::Object(apps))
}

/// Writes JSON to a temp file in the same folder and renames it over `path`.
fn write_atomic(path: &Path, value: &Value) -> std::io::Result<()> {
    let text =
        serde_json::to_string_pretty(value).map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "the discovery state could not be serialized"))?;
    let temp = path.with_extension(format!("json.tmp{}", std::process::id()));
    fs::write(&temp, text)?;
    fs::rename(&temp, path)
}
