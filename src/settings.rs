//! Persistent app settings (~/.supermd/settings.toml). Deliberately
//! tiny: theme choices only, for now.

use std::path::{Path, PathBuf};

#[derive(serde::Serialize, serde::Deserialize, Clone, PartialEq, Debug)]
#[serde(default)]
pub struct Settings {
    pub light_theme: String,
    pub dark_theme: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            light_theme: "Paper".into(),
            dark_theme: "Graphite".into(),
        }
    }
}

/// ~/.supermd (shared with the backups directory's parent).
pub fn config_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".supermd")
}

pub fn themes_dir() -> PathBuf {
    config_dir().join("themes")
}

pub fn load(dir: &Path) -> Settings {
    std::fs::read_to_string(dir.join("settings.toml"))
        .ok()
        .and_then(|s| toml::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save(dir: &Path, settings: &Settings) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let body = toml::to_string_pretty(settings)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    std::fs::write(dir.join("settings.toml"), body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_when_missing_or_invalid() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load(dir.path()), Settings::default());
        std::fs::write(dir.path().join("settings.toml"), "not [valid").unwrap();
        assert_eq!(load(dir.path()), Settings::default());
        assert_eq!(Settings::default().light_theme, "Paper");
        assert_eq!(Settings::default().dark_theme, "Graphite");
    }

    #[test]
    fn save_load_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let s = Settings { light_theme: "Solarized Light".into(), dark_theme: "Nord".into() };
        save(dir.path(), &s).unwrap();
        assert_eq!(load(dir.path()), s);
    }

    #[test]
    fn partial_file_fills_defaults() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("settings.toml"), "dark_theme = \"Nord\"\n").unwrap();
        let s = load(dir.path());
        assert_eq!(s.light_theme, "Paper");
        assert_eq!(s.dark_theme, "Nord");
    }
}
