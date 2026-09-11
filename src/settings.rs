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
    /// Security-scoped bookmark blobs for `recent_workspaces`, keyed by
    /// path. Only the sandboxed macOS build writes these; every other
    /// build leaves the map empty and reopens by path.
    pub workspace_bookmarks: std::collections::BTreeMap<String, String>,
    /// Run the first formatter plugin before every save (default off).
    pub format_on_save: bool,
    /// Per-plugin capability grants ("workspace-read") or refusals
    /// ("denied:workspace-read").
    pub plugin_grants: std::collections::BTreeMap<String, Vec<String>>,
    /// Time-of-day theme adaptation (off unless enabled).
    pub flux: FluxSettings,
    /// Whether the user has already been offered (and answered, yes or
    /// no) "make SuperMD the default Markdown app". A refusal is
    /// remembered forever -- a prompt that comes back is worse than no
    /// prompt.
    pub default_handler_asked: bool,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, PartialEq, Debug)]
#[serde(default)]
pub struct FluxSettings {
    pub enabled: bool,
    /// Coordinates for sunrise/sunset; without them a fixed 7:00–19:00
    /// day window applies.
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    /// Crossfade to the dark theme at night.
    pub auto_dark: bool,
    /// Drift colors toward `night_kelvin` as night falls.
    pub warm_shift: bool,
    pub night_kelvin: f64,
    pub transition_minutes: f64,
}

impl Default for FluxSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            latitude: None,
            longitude: None,
            auto_dark: true,
            warm_shift: true,
            night_kelvin: 3400.0,
            transition_minutes: 40.0,
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            light_theme: "Jackfruit Light".into(),
            dark_theme: "Jackfruit Dark".into(),
            reopen_last: true,
            recent_workspaces: Vec::new(),
            workspace_bookmarks: Default::default(),
            format_on_save: false,
            plugin_grants: Default::default(),
            flux: FluxSettings::default(),
            default_handler_asked: false,
        }
    }
}

impl Settings {
    /// Record a just-opened workspace: dedupe, push front, cap at 8.
    /// `bookmark` is the hex-encoded scoped grant, when this build
    /// captures one.
    pub fn note_workspace(&mut self, path: &Path, bookmark: Option<String>) {
        let p = path.to_string_lossy().into_owned();
        self.recent_workspaces.retain(|x| *x != p);
        self.recent_workspaces.insert(0, p.clone());
        self.recent_workspaces.truncate(8);
        if let Some(blob) = bookmark {
            self.workspace_bookmarks.insert(p, blob);
        }
        crate::bookmarks::prune(&mut self.workspace_bookmarks, &self.recent_workspaces);
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
    load_reporting(dir).0
}

/// Like `load`, but also reports when an on-disk file existed and
/// could not be used -- the case a torn write produces (including
/// invalid UTF-8 from a write cut off mid-character, which
/// `read_to_string` rejects the same way it rejects a missing file).
/// A missing file is the ordinary first run and stays silent; anything
/// else preserves the original bytes beside the new one and returns a
/// message describing what happened and where they went, so a caller
/// that can reach the user (workspace startup) can say so before the
/// next save silently overwrites the preserved copy with defaults.
pub fn load_reporting(dir: &Path) -> (Settings, Option<String>) {
    let path = dir.join("settings.toml");
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            // No file is the ordinary first-run case, not a problem.
            return (Settings::default(), None);
        }
        Err(err) => {
            return (Settings::default(), Some(preserve_corrupt(dir, &path, &err.to_string())));
        }
    };
    match toml::from_str(&String::from_utf8_lossy(&bytes)) {
        Ok(settings) => (settings, None),
        Err(err) => (Settings::default(), Some(preserve_corrupt(dir, &path, &err.to_string()))),
    }
}

/// Defaulting is right for a missing file and wrong for a corrupt one:
/// the user's themes, recents, bookmarks and every plugin permission
/// grant live here. Keep the bytes so they can be recovered, and
/// return a message describing why and where.
fn preserve_corrupt(dir: &Path, path: &Path, reason: &str) -> String {
    let kept = dir.join("settings.toml.corrupt");
    // The source is already fully written by the time we get here (we
    // are reading it after the fact, not racing a writer), so a plain
    // copy keeps the exact original bytes -- unlike `save`, there is no
    // concurrent writer for this to tear.
    let _ = std::fs::copy(path, &kept);
    let msg = format!(
        "{} could not be read ({reason}); the previous file was kept at {}",
        path.display(),
        kept.display()
    );
    eprintln!("supermd: {msg}");
    msg
}

pub fn save(dir: &Path, settings: &Settings) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let body = toml::to_string_pretty(settings)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    // Temp file plus rename, as documents already do: an interrupted
    // write leaves the old file intact rather than a truncated one.
    crate::editor::autosave::atomic_write(&dir.join("settings.toml"), &body)
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

    /// A refusal is remembered forever. A prompt that comes back is
    /// worse than no prompt.
    #[test]
    fn the_default_handler_answer_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = Settings::default();
        assert!(!s.default_handler_asked, "not asked on a fresh install");
        s.default_handler_asked = true;
        save(dir.path(), &s).unwrap();
        assert!(load(dir.path()).default_handler_asked);
    }

    #[test]
    fn flux_defaults_off_and_parses_partial_tables() {
        let d = FluxSettings::default();
        assert!(!d.enabled && d.auto_dark && d.warm_shift);
        assert_eq!(d.night_kelvin, 3400.0);
        assert_eq!(d.transition_minutes, 40.0);
        assert_eq!(d.latitude, None);

        // A partial [flux] table keeps defaults for absent keys, and
        // pre-flux settings files still parse.
        let s: Settings =
            toml::from_str("[flux]\nenabled = true\nlatitude = 12.97\nlongitude = 77.59\n")
                .unwrap();
        assert!(s.flux.enabled && s.flux.auto_dark);
        assert_eq!(s.flux.latitude, Some(12.97));
        let old: Settings = toml::from_str("light_theme = \"Paper\"\n").unwrap();
        assert_eq!(old.flux, FluxSettings::default());
    }

    #[test]
    fn flux_survives_a_save_load_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = Settings::default();
        s.flux.enabled = true;
        s.flux.latitude = Some(51.5);
        s.flux.night_kelvin = 2700.0;
        save(dir.path(), &s).unwrap();
        assert_eq!(load(dir.path()), s);
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
            s.note_workspace(Path::new(&format!("/w/{i}")), None);
        }
        assert_eq!(s.recent_workspaces.len(), 8);
        assert_eq!(s.recent_workspaces[0], "/w/9");
        s.note_workspace(Path::new("/w/5"), None);
        assert_eq!(s.recent_workspaces[0], "/w/5");
        assert_eq!(s.recent_workspaces.iter().filter(|p| *p == "/w/5").count(), 1);
    }

    #[test]
    fn note_workspace_stores_and_prunes_bookmarks() {
        let mut s = Settings::default();
        for i in 0..9 {
            s.note_workspace(Path::new(&format!("/w/{i}")), Some(format!("{i:02x}")));
        }
        // Eight recents cap, and the bookmark map never outgrows them.
        assert_eq!(s.recent_workspaces.len(), 8);
        assert_eq!(s.workspace_bookmarks.len(), 8);
        assert!(!s.workspace_bookmarks.contains_key("/w/0"));
        assert_eq!(s.workspace_bookmarks.get("/w/8"), Some(&"08".to_string()));
    }

    #[test]
    fn note_workspace_without_a_bookmark_leaves_the_map_alone() {
        let mut s = Settings::default();
        s.note_workspace(Path::new("/w/a"), None);
        assert_eq!(s.recent_workspaces, vec!["/w/a".to_string()]);
        assert!(s.workspace_bookmarks.is_empty());
    }

    #[test]
    fn settings_without_bookmarks_still_parse() {
        // Forward/backward compatibility: an old settings.toml has no
        // workspace_bookmarks key.
        let s: Settings = toml::from_str("reopen_last = true\n").unwrap();
        assert!(s.workspace_bookmarks.is_empty());
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

    /// A torn write must not cost the user their settings. `save` was a
    /// plain `fs::write`, so an interrupted one truncated the file and
    /// `load` silently returned defaults -- discarding themes, recents,
    /// bookmarks and every plugin permission grant with no message.
    #[test]
    fn a_corrupt_settings_file_is_preserved_not_discarded() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("settings.toml"), "this is not = valid toml [[[").unwrap();

        let loaded = load(dir.path());
        assert_eq!(loaded, Settings::default(), "unreadable settings fall back");
        assert!(
            dir.path().join("settings.toml.corrupt").exists(),
            "and the unreadable file is kept, not thrown away"
        );
        assert!(
            std::fs::read_to_string(dir.path().join("settings.toml.corrupt"))
                .unwrap()
                .contains("not = valid"),
            "the preserved copy is the original bytes"
        );
    }

    /// A missing file is not a corrupt one: first run must not leave a
    /// `.corrupt` file lying beside the settings.
    #[test]
    fn a_missing_settings_file_leaves_no_corrupt_copy() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load(dir.path()), Settings::default());
        assert!(!dir.path().join("settings.toml.corrupt").exists());
    }

    /// A write torn mid-character leaves invalid UTF-8 -- and
    /// `read_to_string` rejects that exactly the way it rejects a
    /// missing file, so treating every read error as "no file" misses
    /// the realistic torn-write case whenever the settings held any
    /// non-ASCII content (a theme name, a grant string, a path): no
    /// `.corrupt` copy, no message, and the bytes gone for good the
    /// moment anything next saves.
    #[test]
    fn invalid_utf8_from_a_torn_write_is_preserved_not_discarded() {
        let dir = tempfile::tempdir().unwrap();
        // A truncated 4-byte UTF-8 sequence, as a write cut off
        // mid-character would leave behind.
        let original: &[u8] = b"theme = \"dark\xF0\x9F\x92";
        std::fs::write(dir.path().join("settings.toml"), original).unwrap();

        let loaded = load(dir.path());
        assert_eq!(loaded, Settings::default(), "unreadable settings fall back");
        let kept = dir.path().join("settings.toml.corrupt");
        assert!(kept.exists(), "invalid UTF-8 must take the preservation path, not the missing-file one");
        assert_eq!(
            std::fs::read(&kept).unwrap(),
            original,
            "the preserved copy keeps the exact original bytes, not a lossy re-encoding"
        );
    }

    /// The corruption is reported, not just fixed silently -- every
    /// call site does load-mutate-save, so a caller that never learns
    /// about the fallback will overwrite the preserved copy with
    /// defaults on its very next save.
    #[test]
    fn load_reporting_surfaces_the_corruption_message() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("settings.toml"), "this is not = valid toml [[[").unwrap();

        let (settings, corrupt) = load_reporting(dir.path());
        assert_eq!(settings, Settings::default());
        let msg = corrupt.expect("a parse failure must be reported, not swallowed");
        assert!(
            msg.contains("settings.toml.corrupt"),
            "the message must point at the recovery file: {msg}"
        );
    }

    #[test]
    fn load_reporting_is_silent_for_a_missing_or_valid_file() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load_reporting(dir.path()).1, None, "first run reports nothing");
        save(dir.path(), &Settings::default()).unwrap();
        assert_eq!(load_reporting(dir.path()).1, None, "a valid file reports nothing");
    }

    /// Writes go through a temp file and a rename, so a reader never
    /// observes a half-written file. A regression to a plain
    /// `fs::write` would still leave "one file, no scratch" for a
    /// clean directory, so a stale scratch file from a previous crash
    /// is the case that actually distinguishes the two: `save` must
    /// clear it, where an in-place `fs::write` would leave it sitting
    /// untouched.
    #[test]
    fn save_cleans_up_a_stale_scratch_file_from_a_previous_crash() {
        let dir = tempfile::tempdir().unwrap();
        // Mirrors atomic_write's naming: settings.toml -> settings.supermd-tmp.
        std::fs::write(dir.path().join("settings.supermd-tmp"), "leftover from a crashed write").unwrap();

        let mut s = Settings::default();
        s.format_on_save = true;
        save(dir.path(), &s).unwrap();

        let names: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["settings.toml".to_string()], "stale scratch must not survive: {names:?}");
        assert!(load(dir.path()).format_on_save, "and it round-trips");
    }

    /// Same claim, checked a second way: a rename always produces a
    /// fresh inode, where an in-place `fs::write` truncation reuses the
    /// old one. This is the most direct evidence that `save` goes
    /// through rename rather than truncation.
    #[cfg(unix)]
    #[test]
    fn save_replaces_the_file_via_rename_not_in_place_truncation() {
        use std::os::unix::fs::MetadataExt;

        let dir = tempfile::tempdir().unwrap();
        save(dir.path(), &Settings::default()).unwrap();
        let before = std::fs::metadata(dir.path().join("settings.toml")).unwrap().ino();

        let mut s = Settings::default();
        s.format_on_save = true;
        save(dir.path(), &s).unwrap();
        let after = std::fs::metadata(dir.path().join("settings.toml")).unwrap().ino();

        assert_ne!(before, after, "save must rename a new file over the old one, not truncate it in place");
        assert!(load(dir.path()).format_on_save);
    }
}
