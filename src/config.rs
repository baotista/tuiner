//! TOML load/save at the XDG config path — Input Device, Input Channel, Tuning, Mode and
//! Reference Pitch, the state issue #12 persists across a restart.
//!
//! A missing file means first run (`load` returns `Ok(None)`), not an error. A file that exists
//! but won't parse — hand-edited into something broken — is reported via `Err` rather than
//! treated as fatal; the caller falls back to the picker and defaults exactly as it would on
//! first run. The missing-*device* fallback (the remembered device having vanished) is not this
//! module's concern: it needs the live Input Device list, which only `main` has.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::tuning::Mode;

/// Everything persisted across a restart.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Config {
    pub device_name: String,
    pub channel: usize,
    pub tuning: String,
    pub mode: Mode,
    pub reference_pitch: f32,
}

/// The real XDG config file path: `$XDG_CONFIG_HOME/tuiner/config.toml`, falling back to
/// `$HOME/.config/tuiner/config.toml` when `XDG_CONFIG_HOME` isn't set.
pub fn default_path() -> PathBuf {
    resolve_path(
        std::env::var_os("XDG_CONFIG_HOME"),
        std::env::var_os("HOME"),
    )
}

/// The path-resolution logic on its own, independent of the real environment, so it's testable
/// without mutating process-global env vars.
fn resolve_path(xdg_config_home: Option<OsString>, home: Option<OsString>) -> PathBuf {
    let base = xdg_config_home
        .map(PathBuf::from)
        .or_else(|| home.map(|h| PathBuf::from(h).join(".config")))
        .expect("neither XDG_CONFIG_HOME nor HOME is set");
    base.join("tuiner").join("config.toml")
}

/// Loads the config at `path`. `Ok(None)` means no file exists yet — first run. `Err` carries a
/// message to report when the file exists but isn't valid TOML for a `Config` at all; the caller
/// must still start the app rather than treat this as fatal.
pub fn load(path: &Path) -> Result<Option<Config>, String> {
    let contents = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("could not read config at {}: {e}", path.display())),
    };
    toml::from_str(&contents)
        .map(Some)
        .map_err(|e| format!("config at {} is invalid: {e}", path.display()))
}

/// Writes `config` to `path` as TOML, creating the parent directory first if it doesn't exist.
pub fn save(path: &Path, config: &Config) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("could not create {}: {e}", parent.display()))?;
    }
    let contents =
        toml::to_string_pretty(config).map_err(|e| format!("could not serialize config: {e}"))?;
    fs::write(path, contents)
        .map_err(|e| format!("could not write config at {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Config {
        Config {
            device_name: "USB Audio Interface".into(),
            channel: 1,
            tuning: "DADGAD".into(),
            mode: Mode::Guided,
            reference_pitch: 442.0,
        }
    }

    #[test]
    fn xdg_config_home_wins_when_set() {
        let path = resolve_path(Some("/xdg-base".into()), Some("/home/player".into()));
        assert_eq!(path, PathBuf::from("/xdg-base/tuiner/config.toml"));
    }

    #[test]
    fn falls_back_to_home_dot_config_without_xdg_config_home() {
        let path = resolve_path(None, Some("/home/player".into()));
        assert_eq!(
            path,
            PathBuf::from("/home/player/.config/tuiner/config.toml")
        );
    }

    #[test]
    fn loading_a_missing_file_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("does-not-exist.toml");
        assert_eq!(load(&path), Ok(None));
    }

    #[test]
    fn loading_malformed_content_reports_an_error_rather_than_panicking() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, "this is not valid TOML for a Config {{{").unwrap();
        assert!(load(&path).is_err());
    }

    #[test]
    fn saving_then_loading_round_trips_the_config() {
        let dir = tempfile::tempdir().unwrap();
        // Nested, non-existent parent — exercises the create_dir_all path too.
        let path = dir.path().join("nested").join("config.toml");
        let config = sample();
        save(&path, &config).unwrap();
        assert_eq!(load(&path), Ok(Some(config)));
    }

    #[test]
    fn saved_config_is_human_readable_toml() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        save(&path, &sample()).unwrap();
        let contents = fs::read_to_string(&path).unwrap();
        assert!(contents.contains("device_name"));
        assert!(contents.contains("DADGAD"));
    }
}
