use std::sync::Arc;

use gpui::{rgb, App, Global, Hsla, SharedString, WindowAppearance};

/// Colors for tree-sitter highlight captures.
pub struct SyntaxColors {
    pub keyword: Hsla,
    pub function: Hsla,
    pub kind: Hsla, // types
    pub string: Hsla,
    pub comment: Hsla,
    pub constant: Hsla,
    pub property: Hsla,
    pub operator: Hsla,
    pub tag: Hsla,
    pub attribute: Hsla,
}

/// Visual constants for the whole app. One place to tune the look.
pub struct Theme {
    pub is_dark: bool,

    // Document surface
    pub bg: Hsla,
    pub fg: Hsla,
    pub fg_strong: Hsla,
    pub fg_muted: Hsla,
    pub accent: Hsla,
    pub link: Hsla,
    pub code_bg: Hsla,
    pub code_fg: Hsla,
    pub border: Hsla,

    // Chrome: sidebar, tab bar, panels
    pub panel_bg: Hsla,
    pub hover_bg: Hsla,
    pub selected_bg: Hsla,
    pub find_match_bg: Hsla,
    pub find_active_bg: Hsla,

    // Diff view washes
    pub diff_added_bg: Hsla,
    pub diff_added_fg: Hsla,
    pub diff_deleted_bg: Hsla,
    pub diff_deleted_fg: Hsla,

    pub syntax: SyntaxColors,

    pub body_family: SharedString,
    pub mono_family: SharedString,

    pub body_size: f32,
    pub body_line_height: f32,
    pub code_size: f32,
    pub ui_size: f32,
}

impl Theme {
    pub fn light() -> Self {
        Self {
            is_dark: false,

            bg: rgb(0xfdfbf6).into(),
            fg: rgb(0x33302a).into(),
            fg_strong: rgb(0x211f1a).into(),
            fg_muted: rgb(0x918b7d).into(),
            accent: rgb(0xc9821c).into(),
            link: rgb(0xc9821c).into(),
            code_bg: rgb(0xf6f2e9).into(),
            code_fg: rgb(0x4a463d).into(),
            border: rgb(0xeae5d8).into(),

            panel_bg: rgb(0xf8f5ec).into(),
            hover_bg: rgb(0xf0ebdf).into(),
            selected_bg: rgb(0xe8e1d0).into(),
            find_match_bg: rgb(0xf6e3a8).into(),
            find_active_bg: rgb(0xecc153).into(),

            diff_added_bg: rgb(0xe6f0dc).into(),
            diff_added_fg: rgb(0x3d6b2f).into(),
            diff_deleted_bg: rgb(0xf7e3e0).into(),
            diff_deleted_fg: rgb(0xa04b3d).into(),

            syntax: SyntaxColors {
                keyword: rgb(0xa626a4).into(),
                function: rgb(0x4078f2).into(),
                kind: rgb(0xc18401).into(),
                string: rgb(0x50a14f).into(),
                comment: rgb(0xa2a3a7).into(),
                constant: rgb(0x986801).into(),
                property: rgb(0xe45649).into(),
                operator: rgb(0x707277).into(),
                tag: rgb(0xe45649).into(),
                attribute: rgb(0x986801).into(),
            },

            body_family: crate::platform::body_font().into(),
            mono_family: crate::platform::mono_font().into(),

            body_size: 16.0,
            body_line_height: 1.65,
            code_size: 13.0,
            ui_size: 13.0,
        }
    }

    pub fn dark() -> Self {
        Self {
            is_dark: true,

            bg: rgb(0x211f1a).into(),
            fg: rgb(0xd9d4c8).into(),
            fg_strong: rgb(0xf2ede2).into(),
            fg_muted: rgb(0x8f897a).into(),
            accent: rgb(0xe5a63b).into(),
            link: rgb(0xe5a63b).into(),
            code_bg: rgb(0x2b2822).into(),
            code_fg: rgb(0xcfc9ba).into(),
            border: rgb(0x383428).into(),

            panel_bg: rgb(0x262420).into(),
            hover_bg: rgb(0x2f2c25).into(),
            selected_bg: rgb(0x3a362c).into(),
            find_match_bg: rgb(0x574a1c).into(),
            find_active_bg: rgb(0x7d6a24).into(),

            diff_added_bg: rgb(0x2c3a26).into(),
            diff_added_fg: rgb(0xa8c897).into(),
            diff_deleted_bg: rgb(0x3d2723).into(),
            diff_deleted_fg: rgb(0xd18b7f).into(),

            syntax: SyntaxColors {
                keyword: rgb(0xc678dd).into(),
                function: rgb(0x61afef).into(),
                kind: rgb(0xe5c07b).into(),
                string: rgb(0x98c379).into(),
                comment: rgb(0x6b7280).into(),
                constant: rgb(0xd19a66).into(),
                property: rgb(0xe06c75).into(),
                operator: rgb(0x8a919c).into(),
                tag: rgb(0xe06c75).into(),
                attribute: rgb(0xd19a66).into(),
            },

            body_family: crate::platform::body_font().into(),
            mono_family: crate::platform::mono_font().into(),

            body_size: 16.0,
            body_line_height: 1.65,
            code_size: 13.0,
            ui_size: 13.0,
        }
    }

    /// Type scale for headings, level 1..=6.
    pub fn heading_size(&self, level: u8) -> f32 {
        match level {
            1 => 28.0,
            2 => 23.0,
            3 => 19.0,
            4 => 17.0,
            _ => self.body_size,
        }
    }
}

pub struct ActiveTheme(pub Arc<Theme>);

impl Global for ActiveTheme {}

/// The current theme. Cheap to clone (Arc).
pub fn theme(cx: &App) -> Arc<Theme> {
    cx.global::<ActiveTheme>().0.clone()
}

// ── theme files ─────────────────────────────────────────────────────────

#[derive(serde::Deserialize)]
struct ThemeFileColors {
    bg: String,
    fg: String,
    fg_strong: String,
    fg_muted: String,
    accent: String,
    link: String,
    code_bg: String,
    code_fg: String,
    border: String,
    panel_bg: String,
    hover_bg: String,
    selected_bg: String,
    find_match_bg: String,
    find_active_bg: String,
    diff_added_bg: Option<String>,
    diff_added_fg: Option<String>,
    diff_deleted_bg: Option<String>,
    diff_deleted_fg: Option<String>,
}

#[derive(serde::Deserialize)]
struct ThemeFileSyntax {
    keyword: String,
    function: String,
    #[serde(rename = "type")]
    kind: String,
    string: String,
    comment: String,
    constant: String,
    property: String,
    operator: String,
    tag: String,
    attribute: String,
}

#[derive(serde::Deserialize)]
struct ThemeFile {
    name: String,
    appearance: String,
    colors: ThemeFileColors,
    syntax: ThemeFileSyntax,
}

pub struct LoadedTheme {
    pub name: String,
    pub theme: Arc<Theme>,
}

pub fn parse_hex(s: &str) -> Result<Hsla, String> {
    let hex = s.strip_prefix('#').unwrap_or(s);
    if hex.len() != 6 {
        return Err(format!("bad hex color {s:?}"));
    }
    let value = u32::from_str_radix(hex, 16).map_err(|e| format!("bad hex color {s:?}: {e}"))?;
    Ok(rgb(value).into())
}

impl LoadedTheme {
    pub fn from_toml(source: &str) -> Result<Self, String> {
        let file: ThemeFile = toml::from_str(source).map_err(|e| e.to_string())?;
        let is_dark = match file.appearance.as_str() {
            "dark" => true,
            "light" => false,
            other => return Err(format!("bad appearance {other:?}")),
        };
        let mut theme = if is_dark { Theme::dark() } else { Theme::light() };
        let c = &file.colors;
        theme.bg = parse_hex(&c.bg)?;
        theme.fg = parse_hex(&c.fg)?;
        theme.fg_strong = parse_hex(&c.fg_strong)?;
        theme.fg_muted = parse_hex(&c.fg_muted)?;
        theme.accent = parse_hex(&c.accent)?;
        theme.link = parse_hex(&c.link)?;
        theme.code_bg = parse_hex(&c.code_bg)?;
        theme.code_fg = parse_hex(&c.code_fg)?;
        theme.border = parse_hex(&c.border)?;
        theme.panel_bg = parse_hex(&c.panel_bg)?;
        theme.hover_bg = parse_hex(&c.hover_bg)?;
        theme.selected_bg = parse_hex(&c.selected_bg)?;
        theme.find_match_bg = parse_hex(&c.find_match_bg)?;
        theme.find_active_bg = parse_hex(&c.find_active_bg)?;
        // Optional keys keep their appearance defaults when absent.
        if let Some(v) = &c.diff_added_bg {
            theme.diff_added_bg = parse_hex(v)?;
        }
        if let Some(v) = &c.diff_added_fg {
            theme.diff_added_fg = parse_hex(v)?;
        }
        if let Some(v) = &c.diff_deleted_bg {
            theme.diff_deleted_bg = parse_hex(v)?;
        }
        if let Some(v) = &c.diff_deleted_fg {
            theme.diff_deleted_fg = parse_hex(v)?;
        }
        let s = &file.syntax;
        theme.syntax = SyntaxColors {
            keyword: parse_hex(&s.keyword)?,
            function: parse_hex(&s.function)?,
            kind: parse_hex(&s.kind)?,
            string: parse_hex(&s.string)?,
            comment: parse_hex(&s.comment)?,
            constant: parse_hex(&s.constant)?,
            property: parse_hex(&s.property)?,
            operator: parse_hex(&s.operator)?,
            tag: parse_hex(&s.tag)?,
            attribute: parse_hex(&s.attribute)?,
        };
        Ok(LoadedTheme { name: file.name, theme: Arc::new(theme) })
    }
}

/// Builtin theme TOML sources, lights first.
pub fn builtin_theme_sources() -> [&'static str; 8] {
    [
        include_str!("../assets/themes/jackfruit-light.toml"),
        include_str!("../assets/themes/paper.toml"),
        include_str!("../assets/themes/solarized-light.toml"),
        include_str!("../assets/themes/jackfruit-dark.toml"),
        include_str!("../assets/themes/graphite.toml"),
        include_str!("../assets/themes/solarized-dark.toml"),
        include_str!("../assets/themes/nord.toml"),
        include_str!("../assets/themes/gruvbox-dark.toml"),
    ]
}

pub fn builtin_themes() -> Vec<LoadedTheme> {
    builtin_theme_sources()
        .iter()
        .map(|src| LoadedTheme::from_toml(src).expect("builtin theme must parse"))
        .collect()
}

/// Custom themes from a directory; malformed files are skipped loudly.
pub fn load_custom_themes(dir: &std::path::Path) -> Vec<LoadedTheme> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("toml") {
            continue;
        }
        match std::fs::read_to_string(&path).map_err(|e| e.to_string()).and_then(|s| {
            LoadedTheme::from_toml(&s)
        }) {
            Ok(theme) => out.push(theme),
            Err(err) => eprintln!("supermd: skipping theme {}: {err}", path.display()),
        }
    }
    out
}

#[cfg(test)]
mod theme_file_tests {
    use super::*;

    #[test]
    fn parse_hex_roundtrips_and_rejects_garbage() {
        assert!(parse_hex("#dd4c4f").is_ok());
        assert!(parse_hex("dd4c4f").is_ok()); // leading # optional
        assert!(parse_hex("#xyz").is_err());
        assert!(parse_hex("#dd4c").is_err());
    }

    #[test]
    fn builtins_parse_with_declared_appearance() {
        let themes = builtin_themes();
        let names: Vec<&str> = themes.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "Jackfruit Light",
                "Paper",
                "Solarized Light",
                "Jackfruit Dark",
                "Graphite",
                "Solarized Dark",
                "Nord",
                "Gruvbox Dark"
            ]
        );
        assert!(!themes[0].theme.is_dark);
        assert!(themes[3].theme.is_dark);
        assert_eq!(themes.iter().filter(|t| t.theme.is_dark).count(), 5);
    }

    #[test]
    fn theme_file_toml_maps_every_field() {
        let toml_src = r##"
name = "Test"
appearance = "dark"
[colors]
bg = "#111111"
fg = "#dddddd"
fg_strong = "#ffffff"
fg_muted = "#888888"
accent = "#ff0000"
link = "#ff0001"
code_bg = "#222222"
code_fg = "#cccccc"
border = "#333333"
panel_bg = "#191919"
hover_bg = "#252525"
selected_bg = "#303030"
find_match_bg = "#554400"
find_active_bg = "#776600"
[syntax]
keyword = "#c678dd"
function = "#61afef"
type = "#e5c07b"
string = "#98c379"
comment = "#5c6370"
constant = "#d19a66"
property = "#e06c75"
operator = "#8a919c"
tag = "#e06c75"
attribute = "#d19a66"
"##;
        let loaded = LoadedTheme::from_toml(toml_src).unwrap();
        assert_eq!(loaded.name, "Test");
        assert!(loaded.theme.is_dark);
        assert_eq!(loaded.theme.bg, gpui::rgb(0x111111).into());
        assert_eq!(loaded.theme.syntax.kind, gpui::rgb(0xe5c07b).into());
        assert_eq!(loaded.theme.find_active_bg, gpui::rgb(0x776600).into());
    }

    #[test]
    fn builtin_appearances_have_distinct_diff_colors() {
        assert_ne!(Theme::light().diff_added_bg, Theme::dark().diff_added_bg);
        assert_ne!(Theme::light().diff_deleted_bg, Theme::dark().diff_deleted_bg);
        assert_ne!(Theme::light().diff_added_fg, Theme::light().diff_deleted_fg);
    }

    #[test]
    fn theme_file_diff_keys_optional_and_parsed() {
        let toml_src = r##"
name = "T"
appearance = "dark"
[colors]
bg = "#111111"
fg = "#dddddd"
fg_strong = "#ffffff"
fg_muted = "#888888"
accent = "#ff0000"
link = "#ff0001"
code_bg = "#222222"
code_fg = "#cccccc"
border = "#333333"
panel_bg = "#191919"
hover_bg = "#252525"
selected_bg = "#303030"
find_match_bg = "#554400"
find_active_bg = "#776600"
diff_added_bg = "#112233"
[syntax]
keyword = "#c678dd"
function = "#61afef"
type = "#e5c07b"
string = "#98c379"
comment = "#5c6370"
constant = "#d19a66"
property = "#e06c75"
operator = "#8a919c"
tag = "#e06c75"
attribute = "#d19a66"
"##;
        let loaded = LoadedTheme::from_toml(toml_src).unwrap();
        assert_eq!(loaded.theme.diff_added_bg, gpui::rgb(0x112233).into());
        // unspecified keys fall back to appearance defaults
        assert_eq!(loaded.theme.diff_deleted_bg, Theme::dark().diff_deleted_bg);
        assert_eq!(loaded.theme.diff_added_fg, Theme::dark().diff_added_fg);
    }

    #[test]
    fn custom_dir_loads_valid_and_skips_invalid() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("broken.toml"),
            "name = \"Broken\"\nappearance = \"light\"\n",
        )
        .unwrap();
        let good = builtin_theme_sources()[1].replace("Paper", "My Paper");
        std::fs::write(dir.path().join("good.toml"), good).unwrap();
        let themes = load_custom_themes(dir.path());
        assert_eq!(themes.len(), 1);
        assert_eq!(themes[0].name, "My Paper");
    }

    #[test]
    fn custom_dir_ignores_non_toml_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("notes.txt"), "not a theme").unwrap();
        std::fs::write(dir.path().join("README.md"), "# themes").unwrap();
        std::fs::write(dir.path().join("noext"), "").unwrap();
        assert!(load_custom_themes(dir.path()).is_empty());
        // A valid .toml alongside them still loads.
        std::fs::write(dir.path().join("ok.toml"), builtin_theme_sources()[0]).unwrap();
        assert_eq!(load_custom_themes(dir.path()).len(), 1);
    }

    #[test]
    fn heading_size_scale_descends_to_body_size() {
        let t = Theme::light();
        assert_eq!(t.heading_size(1), 28.0);
        assert_eq!(t.heading_size(2), 23.0);
        assert_eq!(t.heading_size(3), 19.0);
        assert_eq!(t.heading_size(4), 17.0);
        assert_eq!(t.heading_size(5), t.body_size);
        assert_eq!(t.heading_size(6), t.body_size);
        // Sizes strictly decrease from h1 to h4 and never go below body.
        assert!(t.heading_size(1) > t.heading_size(2));
        assert!(t.heading_size(4) >= t.body_size);
    }

    #[test]
    fn bad_appearance_is_rejected() {
        let toml_src = builtin_theme_sources()[0].replacen("light", "purple", 1);
        let Err(err) = LoadedTheme::from_toml(&toml_src) else { panic!("expected error") };
        assert!(err.contains("purple"), "unexpected error: {err}");
    }

    #[test]
    fn all_diff_keys_override_defaults() {
        // Inject all four diff keys at the end of [colors] in Jackfruit Dark.
        let toml_src = builtin_theme_sources()[3].replace(
            "[syntax]",
            "diff_added_bg = \"#0a1a0a\"\ndiff_added_fg = \"#aaffaa\"\ndiff_deleted_bg = \"#1a0a0a\"\ndiff_deleted_fg = \"#ffaaaa\"\n[syntax]",
        );
        let loaded = LoadedTheme::from_toml(&toml_src).unwrap();
        assert_eq!(loaded.theme.diff_added_bg, gpui::rgb(0x0a1a0a).into());
        assert_eq!(loaded.theme.diff_added_fg, gpui::rgb(0xaaffaa).into());
        assert_eq!(loaded.theme.diff_deleted_bg, gpui::rgb(0x1a0a0a).into());
        assert_eq!(loaded.theme.diff_deleted_fg, gpui::rgb(0xffaaaa).into());
    }
}

/// All known themes + the user's choices + current system appearance.
pub struct ThemeState {
    pub themes: Vec<LoadedTheme>,
    pub settings: crate::settings::Settings,
    pub system_dark: bool,
}

impl Global for ThemeState {}

impl ThemeState {
    pub fn resolve(&self) -> Arc<Theme> {
        let want = if self.system_dark {
            &self.settings.dark_theme
        } else {
            &self.settings.light_theme
        };
        self.themes
            .iter()
            .find(|t| &t.name == want && t.theme.is_dark == self.system_dark)
            .or_else(|| {
                self.themes
                    .iter()
                    .find(|t| t.theme.is_dark == self.system_dark)
            })
            .map(|t| t.theme.clone())
            .unwrap_or_else(|| {
                Arc::new(if self.system_dark { Theme::dark() } else { Theme::light() })
            })
    }
}

#[cfg(test)]
mod theme_state_tests {
    use super::*;

    fn state(light: &str, dark: &str, system_dark: bool) -> ThemeState {
        ThemeState {
            themes: builtin_themes(),
            settings: crate::settings::Settings {
                light_theme: light.into(),
                dark_theme: dark.into(),
                ..crate::settings::Settings::default()
            },
            system_dark,
        }
    }

    #[test]
    fn resolve_picks_named_theme_for_appearance() {
        let s = state("Paper", "Nord", true);
        let resolved = s.resolve();
        assert!(resolved.is_dark);
        let nord = s.themes.iter().find(|t| t.name == "Nord").unwrap();
        assert_eq!(resolved.bg, nord.theme.bg);

        let s = state("Solarized Light", "Nord", false);
        let resolved = s.resolve();
        assert!(!resolved.is_dark);
        let sol = s.themes.iter().find(|t| t.name == "Solarized Light").unwrap();
        assert_eq!(resolved.bg, sol.theme.bg);
    }

    #[test]
    fn resolve_falls_back_to_first_theme_of_appearance() {
        // Unknown name: fall back to the first dark builtin (Jackfruit Dark).
        let s = state("Paper", "No Such Theme", true);
        let resolved = s.resolve();
        assert!(resolved.is_dark);
        assert_eq!(resolved.bg, Theme::dark().bg);

        // Name exists but has the wrong appearance: same fallback applies.
        let s = state("Nord", "Paper", false);
        let resolved = s.resolve();
        assert!(!resolved.is_dark);
        assert_eq!(resolved.bg, Theme::light().bg); // Jackfruit Light
    }

    #[test]
    fn resolve_with_no_themes_uses_hardcoded_defaults() {
        let mut s = state("x", "y", true);
        s.themes.clear();
        let resolved = s.resolve();
        assert!(resolved.is_dark);
        assert_eq!(resolved.bg, Theme::dark().bg);

        s.system_dark = false;
        let resolved = s.resolve();
        assert!(!resolved.is_dark);
        assert_eq!(resolved.bg, Theme::light().bg);
    }
}

/// Re-resolve the active theme from state (after settings or appearance
/// changes).
pub fn refresh_active_theme(cx: &mut App) {
    let theme = cx.global::<ThemeState>().resolve();
    cx.set_global(ActiveTheme(theme));
}

pub fn apply_system_appearance(appearance: WindowAppearance, cx: &mut App) {
    let dark = matches!(
        appearance,
        WindowAppearance::Dark | WindowAppearance::VibrantDark
    );
    cx.global_mut::<ThemeState>().system_dark = dark;
    refresh_active_theme(cx);
}
