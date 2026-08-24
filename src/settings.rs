//! Persistent app settings (~/.supermd/settings.toml). Deliberately
//! tiny: theme choices only, for now.

use std::path::{Path, PathBuf};

#[derive(serde::Serialize, serde::Deserialize, Clone, PartialEq, Debug)]
#[serde(default)]
pub struct Settings {
    pub light_theme: String,
    pub dark_theme: String,
    /// Reopen the most recent workspace when launched without a path.
    pub reopen_last: bool,
    /// Absolute workspace paths, most recent first, max 8.
    pub recent_workspaces: Vec<String>,
    /// Run the first formatter plugin before every save (default off).
    pub format_on_save: bool,
    /// Per-plugin capability grants ("workspace-read") or refusals
    /// ("denied:workspace-read").
    pub plugin_grants: std::collections::BTreeMap<String, Vec<String>>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            light_theme: "Jackfruit Light".into(),
            dark_theme: "Jackfruit Dark".into(),
            reopen_last: true,
            recent_workspaces: Vec::new(),
            format_on_save: false,
            plugin_grants: Default::default(),
        }
    }
}

impl Settings {
    /// Record a just-opened workspace: dedupe, push front, cap at 8.
    pub fn note_workspace(&mut self, path: &Path) {
        let p = path.to_string_lossy().into_owned();
        self.recent_workspaces.retain(|x| *x != p);
        self.recent_workspaces.insert(0, p);
        self.recent_workspaces.truncate(8);
    }
}

/// ~/.supermd on every OS (HOME, else USERPROFILE on Windows).
pub fn config_dir() -> PathBuf {
    crate::platform::home_dir().join(".supermd")
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
        assert_eq!(Settings::default().light_theme, "Jackfruit Light");
        assert_eq!(Settings::default().dark_theme, "Jackfruit Dark");
    }

    #[test]
    fn save_load_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let s = Settings {
            light_theme: "Solarized Light".into(),
            dark_theme: "Nord".into(),
            ..Settings::default()
        };
        save(dir.path(), &s).unwrap();
        assert_eq!(load(dir.path()), s);
    }

    #[test]
    fn format_on_save_defaults_off() {
        assert!(!Settings::default().format_on_save);
    }

    #[test]
    fn plugin_grants_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = Settings::default();
        s.plugin_grants.insert("reader".into(), vec!["workspace-read".into()]);
        save(dir.path(), &s).unwrap();
        assert_eq!(load(dir.path()), s);
    }

    #[test]
    fn net_domain_grants_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = Settings::default();
        s.plugin_grants.insert(
            "url-title".into(),
            vec!["net:en.wikipedia.org".into(), "denied:net:evil.com".into()],
        );
        save(dir.path(), &s).unwrap();
        let s2 = load(dir.path());
        assert_eq!(
            s2.plugin_grants["url-title"],
            ["net:en.wikipedia.org", "denied:net:evil.com"]
        );
    }

    #[test]
    fn note_workspace_dedupes_and_caps() {
        let mut s = Settings::default();
        assert!(s.reopen_last);
        for i in 0..10 {
            s.note_workspace(Path::new(&format!("/w/{i}")));
        }
        assert_eq!(s.recent_workspaces.len(), 8);
        assert_eq!(s.recent_workspaces[0], "/w/9");
        s.note_workspace(Path::new("/w/5"));
        assert_eq!(s.recent_workspaces[0], "/w/5");
        assert_eq!(s.recent_workspaces.iter().filter(|p| *p == "/w/5").count(), 1);
    }

    #[test]
    fn old_settings_files_still_parse() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("settings.toml"), "light_theme = \"Paper\"\n").unwrap();
        let s = load(dir.path());
        assert_eq!(s.light_theme, "Paper");
        assert!(s.reopen_last);
        assert!(s.recent_workspaces.is_empty());
    }

    #[test]
    fn config_dirs_are_rooted_under_home() {
        // Pure path construction: nothing is read from or written to disk.
        let cfg = config_dir();
        assert!(cfg.ends_with(".supermd"), "got {cfg:?}");
        assert!(cfg.starts_with(crate::platform::home_dir()));
        let themes = themes_dir();
        assert_eq!(themes, cfg.join("themes"));
        assert!(themes.ends_with(".supermd/themes"), "got {themes:?}");
    }

    #[test]
    fn partial_file_fills_defaults() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("settings.toml"), "dark_theme = \"Nord\"\n").unwrap();
        let s = load(dir.path());
        assert_eq!(s.light_theme, "Jackfruit Light");
        assert_eq!(s.dark_theme, "Nord");
    }
}
