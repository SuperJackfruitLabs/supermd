use std::sync::Arc;

use gpui::{rgb, App, Global, Hsla, SharedString, WindowAppearance};

/// Declares a group of colour fields once: the struct itself, a `map`
/// used by `Theme::map_colors`, and a `fields()` accessor the test
/// walks. A field can only be declared here — there is nowhere else
/// for one to hide from `map_colors`, so flux warming cannot miss it.
/// Modelled on the `surfaces!` macro in `menus.rs`, which removed the
/// same hand-maintained-second-list defect from `Surface::ALL`.
macro_rules! theme_colors {
    ($name:ident { $($field:ident),+ $(,)? }) => {
        #[derive(Debug, Clone, Copy, PartialEq)]
        pub struct $name {
            $(pub $field: Hsla,)+
        }

        impl $name {
            fn map(&self, f: &impl Fn(Hsla) -> Hsla) -> Self {
                Self { $($field: f(self.$field),)+ }
            }

            pub fn fields(&self) -> Vec<(&'static str, Hsla)> {
                vec![$((stringify!($field), self.$field),)+]
            }
        }
    };
}

// Colors for tree-sitter highlight captures.
theme_colors!(SyntaxColors {
    keyword,
    function,
    kind, // types
    string,
    comment,
    constant,
    property,
    operator,
    tag,
    attribute,
});

theme_colors!(ThemeColors {
    // Document surface
    bg,
    fg,
    fg_strong,
    fg_muted,
    accent,
    link,
    code_bg,
    code_fg,
    border,
    page_bg,
    border_subtle,
    shadow,

    // Chrome: sidebar, tab bar, panels
    panel_bg,
    hover_bg,
    selected_bg,
    find_match_bg,
    find_active_bg,

    // Diff view washes
    diff_added_bg,
    diff_added_fg,
    diff_deleted_bg,
    diff_deleted_fg,
});

/// Visual constants for the whole app. One place to tune the look.
pub struct Theme {
    pub is_dark: bool,

    pub colors: ThemeColors,
    pub syntax: SyntaxColors,

    pub body_family: SharedString,
    pub mono_family: SharedString,

    pub body_size: f32,
    pub body_line_height: f32,
    pub code_size: f32,
    pub ui_size: f32,
}

/// Existing call sites read and write colours as direct fields
/// (`t.bg`, `theme.accent = ...`) all over `workspace.rs`, `view.rs`
/// and `editor/`. `Deref`/`DerefMut` to `ThemeColors` keeps every one
/// of those compiling unchanged instead of renaming them to `t.colors.bg`.
impl std::ops::Deref for Theme {
    type Target = ThemeColors;
    fn deref(&self) -> &ThemeColors {
        &self.colors
    }
}

impl std::ops::DerefMut for Theme {
    fn deref_mut(&mut self) -> &mut ThemeColors {
        &mut self.colors
    }
}

impl Theme {
    /// A copy with every color passed through `f`; fonts, sizes, and
    /// flags unchanged. Colors are declared once via `theme_colors!`,
    /// so this cannot miss one — flux warming relies on full coverage.
    pub fn map_colors(&self, f: impl Fn(Hsla) -> Hsla) -> Self {
        Self {
            is_dark: self.is_dark,
            colors: self.colors.map(&f),
            syntax: self.syntax.map(&f),
            body_family: self.body_family.clone(),
            mono_family: self.mono_family.clone(),
            body_size: self.body_size,
            body_line_height: self.body_line_height,
            code_size: self.code_size,
            ui_size: self.ui_size,
        }
    }

    /// Every colour field on the theme (document/chrome/diff, then
    /// syntax), name and current value. Used only by tests to walk
    /// the full set `map_colors` is guaranteed to cover.
    pub fn color_fields(&self) -> Vec<(&'static str, Hsla)> {
        let mut v = self.colors.fields();
        v.extend(self.syntax.fields());
        v
    }

    pub fn light() -> Self {
        Self {
            is_dark: false,

            colors: ThemeColors {
                bg: rgb(0xfdfbf6).into(),
                // Ink shares the ground's warm hue rather than sitting
                // neutral-to-black on it -- a warm palette reads as
                // dated the moment the ink stops matching the paper's
                // temperature.
                fg: rgb(0x353027).into(),
                fg_strong: rgb(0x221f19).into(),
                fg_muted: rgb(0x8c8373).into(),
                accent: rgb(0xc9821c).into(),
                link: rgb(0xc9821c).into(),
                code_bg: rgb(0xf6f2e9).into(),
                code_fg: rgb(0x4a463d).into(),
                border: rgb(0xeae5d8).into(),
                // Pure white: the only direction left to brighten from an
                // already near-white ground, and the biggest legible step.
                page_bg: rgb(0xffffff).into(),
                border_subtle: Hsla { a: 0.55, ..rgb(0xeae5d8).into() },
                shadow: Hsla { h: 0.095, s: 0.30, l: 0.18, a: 0.11 },

                panel_bg: rgb(0xf8f5ec).into(),
                hover_bg: rgb(0xf0ebdf).into(),
                selected_bg: rgb(0xe8e1d0).into(),
                find_match_bg: rgb(0xf6e3a8).into(),
                find_active_bg: rgb(0xecc153).into(),

                diff_added_bg: rgb(0xe6f0dc).into(),
                diff_added_fg: rgb(0x3d6b2f).into(),
                diff_deleted_bg: rgb(0xf7e3e0).into(),
                diff_deleted_fg: rgb(0xa04b3d).into(),
            },

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

            colors: ThemeColors {
                bg: rgb(0x211f1a).into(),
                // Same warm hue family as the light theme's ink, so
                // light and dark read as one app rather than a warm
                // theme and a cool one.
                fg: rgb(0xdad3c8).into(),
                fg_strong: rgb(0xf2ece3).into(),
                fg_muted: rgb(0x968973).into(),
                accent: rgb(0xe5a63b).into(),
                link: rgb(0xe5a63b).into(),
                code_bg: rgb(0x2b2822).into(),
                code_fg: rgb(0xcfc9ba).into(),
                border: rgb(0x383428).into(),
                // One step brighter than ground: paper still catches
                // the light even at night.
                page_bg: rgb(0x2b2822).into(),
                border_subtle: Hsla { a: 0.55, ..rgb(0x383428).into() },
                shadow: Hsla { h: 0., s: 0., l: 0., a: 0.34 },

                panel_bg: rgb(0x262420).into(),
                hover_bg: rgb(0x2f2c25).into(),
                selected_bg: rgb(0x3a362c).into(),
                find_match_bg: rgb(0x574a1c).into(),
                find_active_bg: rgb(0x7d6a24).into(),

                diff_added_bg: rgb(0x2c3a26).into(),
                diff_added_fg: rgb(0xa8c897).into(),
                diff_deleted_bg: rgb(0x3d2723).into(),
                diff_deleted_fg: rgb(0xd18b7f).into(),
            },

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

    /// The page is one step from the ground: brighter whenever there is
    /// room to get brighter, and nudged the other way only when the
    /// ground is already at maximum lightness (a pure-white "paper"
    /// theme has nowhere higher to go, but page and ground still need to
    /// read as two distinct surfaces rather than collapsing into one).
    pub fn derive_page_bg(bg: Hsla, is_dark: bool) -> Hsla {
        let step = if is_dark { 0.035 } else { 0.030 };
        let raised = (bg.l + step).min(1.0);
        let l = if raised - bg.l > f32::EPSILON {
            raised
        } else {
            (bg.l - step).max(0.0)
        };
        Hsla { l, ..bg }
    }

    /// One shadow colour. Warm-shifted in light themes so the page
    /// does not cast a cold grey shadow onto a warm ground.
    pub fn derive_shadow(is_dark: bool) -> Hsla {
        if is_dark {
            Hsla { h: 0., s: 0., l: 0., a: 0.34 }
        } else {
            Hsla { h: 0.095, s: 0.30, l: 0.18, a: 0.11 }
        }
    }

    /// WCAG relative-luminance contrast ratio, ranging 1.0 (identical)
    /// to 21.0 (black on white). Alpha is ignored: every colour this
    /// compares is composited opaque in practice.
    pub fn contrast(a: Hsla, b: Hsla) -> f32 {
        fn luminance(c: Hsla) -> f32 {
            let rgba = gpui::Rgba::from(c);
            let f = |v: f32| if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
            0.2126 * f(rgba.r) + 0.7152 * f(rgba.g) + 0.0722 * f(rgba.b)
        }
        let (x, y) = (luminance(a), luminance(b));
        let (hi, lo) = if x > y { (x, y) } else { (y, x) };
        (hi + 0.05) / (lo + 0.05)
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
    #[serde(default)]
    page_bg: Option<String>,
    #[serde(default)]
    border_subtle: Option<String>,
    #[serde(default)]
    shadow: Option<String>,
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
        theme.page_bg = match &c.page_bg {
            Some(hex) => parse_hex(hex)?,
            None => Theme::derive_page_bg(theme.bg, is_dark),
        };
        theme.border_subtle = match &c.border_subtle {
            Some(hex) => parse_hex(hex)?,
            None => Hsla { a: theme.border.a * 0.55, ..theme.border },
        };
        theme.shadow = match &c.shadow {
            Some(hex) => parse_hex(hex)?,
            None => Theme::derive_shadow(is_dark),
        };
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

    /// Warm ground, warm ink. A cream background with neutral-grey or
    /// pure-black text is the temperature mismatch that makes a warm
    /// palette read as dated rather than deliberate.
    #[test]
    fn the_default_themes_share_one_temperature() {
        let light = Theme::light();
        assert!(light.fg.s > 0.02, "light ink is warm-shifted, not neutral grey");
        assert!(
            (light.fg.h - light.bg.h).abs() < 0.15,
            "ink hue {} should sit near the ground's {}",
            light.fg.h,
            light.bg.h
        );
        let dark = Theme::dark();
        assert!(dark.bg.s > 0.01, "dark ground is warm-neutral, not blue-grey");
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

    /// Every colour on the theme is warmed by flux. The macro is what
    /// guarantees it: a field declared in `theme_colors!` is mapped,
    /// and a field cannot be declared anywhere else.
    #[test]
    fn map_colors_touches_every_colour_field() {
        let t = Theme::light();
        let black = Hsla { h: 0., s: 0., l: 0., a: 1. };
        let mapped = t.map_colors(|_| black);
        for (name, c) in mapped.color_fields() {
            assert_eq!(c, black, "{name} was not mapped");
        }
        assert!(
            mapped.color_fields().len() >= 20,
            "colour_fields looks truncated: {}",
            mapped.color_fields().len()
        );
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

    /// Every shipped theme (built-in + `assets/themes/*.toml`), loaded the
    /// way the app loads them. The list is checked against `ls
    /// assets/themes/` at the time this was written -- update it if a
    /// theme file is added, renamed, or removed.
    fn shipped_themes() -> Vec<(String, Arc<Theme>)> {
        let mut v: Vec<(String, Arc<Theme>)> = vec![
            ("built-in light".to_string(), Arc::new(Theme::light())),
            ("built-in dark".to_string(), Arc::new(Theme::dark())),
        ];
        for (name, src) in [
            ("graphite", include_str!("../assets/themes/graphite.toml")),
            ("gruvbox-dark", include_str!("../assets/themes/gruvbox-dark.toml")),
            ("jackfruit-dark", include_str!("../assets/themes/jackfruit-dark.toml")),
            ("jackfruit-light", include_str!("../assets/themes/jackfruit-light.toml")),
            ("nord", include_str!("../assets/themes/nord.toml")),
            ("paper", include_str!("../assets/themes/paper.toml")),
            ("solarized-dark", include_str!("../assets/themes/solarized-dark.toml")),
            ("solarized-light", include_str!("../assets/themes/solarized-light.toml")),
        ] {
            v.push((name.to_string(), LoadedTheme::from_toml(src).expect(name).theme));
        }
        v
    }

    /// A theme that predates these tokens still loads, and gets a page
    /// surface derived from its ground rather than a hole in the UI.
    #[test]
    fn page_bg_is_derived_when_a_theme_omits_it() {
        let light = Theme::light();
        assert!(
            light.page_bg.l > light.bg.l,
            "light: page {} must be brighter than ground {}",
            light.page_bg.l,
            light.bg.l
        );
        let dark = Theme::dark();
        assert!(
            dark.page_bg.l > dark.bg.l,
            "dark: page {} must still be a step up from ground {}",
            dark.page_bg.l,
            dark.bg.l
        );
    }

    /// The page must be visible against the ground without being a
    /// jarring jump -- the convention is one adjacent step. Covers every
    /// shipped theme (not just the built-ins): a theme that omits
    /// `page_bg` goes through `derive_page_bg`, and that derivation
    /// needs to hold the band for real themes, not just the two we
    /// hand-tuned.
    #[test]
    fn page_and_ground_are_one_step_apart() {
        for (name, t) in shipped_themes() {
            let delta = (t.page_bg.l - t.bg.l).abs();
            assert!(
                (0.012..=0.075).contains(&delta),
                "{name}: page/ground delta {delta} is not one adjacent step"
            );
        }
    }

    /// Body text has to stay readable on the new surface, in every
    /// theme we ship -- a derived value that looks wrong in nord is
    /// caught here rather than by squinting at a screenshot.
    ///
    /// Some shipped themes already fail one of these floors on their
    /// *existing* fg/bg pairing, independent of anything this task
    /// derives -- e.g. `nord`'s muted text was never 3:1 against its
    /// ground, and `solarized-light`'s body text tops out well under
    /// 4.5:1 against page white, so no page-surface choice can lift it
    /// over the floor. Each is recorded below as a bounded *band*
    /// (recorded ratio, floor) rather than a hole in the assertion: a
    /// further regression still fails the test, and so does a fix that
    /// clears the real floor -- at that point the entry is stale and
    /// must be deleted, which is what makes Task 8's work (hand-tuning
    /// every shipped theme) show up here instead of nowhere.
    const KNOWN_BODY_GAPS: &[(&str, f32, f32)] = &[
        ("solarized-dark", 3.9, 4.5),
        ("solarized-light", 4.2, 4.5),
    ];
    const KNOWN_MUTED_GAPS: &[(&str, f32, f32)] = &[
        ("nord", 2.3, 3.0),
        ("solarized-dark", 2.7, 3.0),
        ("solarized-light", 2.4, 3.0),
    ];

    #[test]
    fn every_shipped_theme_keeps_text_readable_on_the_page() {
        for (name, theme) in shipped_themes() {
            let body = Theme::contrast(theme.fg, theme.page_bg);
            match KNOWN_BODY_GAPS.iter().find(|(n, _, _)| *n == name) {
                Some((_, floor, ceiling)) => assert!(
                    body >= *floor && body < *ceiling,
                    "{name}: body text on page drifted to {body:.2}:1, expected [{floor}, {ceiling}) --                      if it cleared {ceiling}, delete this exception instead of widening it"
                ),
                None => assert!(body >= 4.5, "{name}: body text on page is {body:.2}:1"),
            }
            let muted = Theme::contrast(theme.fg_muted, theme.bg);
            match KNOWN_MUTED_GAPS.iter().find(|(n, _, _)| *n == name) {
                Some((_, floor, ceiling)) => assert!(
                    muted >= *floor && muted < *ceiling,
                    "{name}: muted text on ground drifted to {muted:.2}:1, expected [{floor}, {ceiling}) --                      if it cleared {ceiling}, delete this exception instead of widening it"
                ),
                None => assert!(muted >= 3.0, "{name}: muted text on ground is {muted:.2}:1"),
            }
            let surfaces = Theme::contrast(theme.page_bg, theme.bg);
            assert!(
                surfaces >= 1.03,
                "{name}: page and ground are indistinguishable ({surfaces:.3}:1)"
            );
        }
    }

    /// A theme that omits `shadow` gets the appearance-appropriate
    /// derived shadow (warm-shifted in light, opaque black in dark) --
    /// not a leftover value from whichever appearance `Theme::light()`/
    /// `Theme::dark()` started from before the file's colours were
    /// applied.
    #[test]
    fn shadow_is_derived_per_appearance_when_absent() {
        let light = LoadedTheme::from_toml(builtin_theme_sources()[0]).unwrap(); // Jackfruit Light
        assert!(!builtin_theme_sources()[0].contains("shadow"));
        assert_eq!(light.theme.shadow, Theme::derive_shadow(false));

        let dark = LoadedTheme::from_toml(builtin_theme_sources()[3]).unwrap(); // Jackfruit Dark
        assert!(!builtin_theme_sources()[3].contains("shadow"));
        assert_eq!(dark.theme.shadow, Theme::derive_shadow(true));

        assert_ne!(light.theme.shadow, dark.theme.shadow);
    }

    /// A theme that omits `border_subtle` gets its own `border` colour
    /// at reduced alpha -- same hue/saturation/lightness, a dimmer
    /// hairline -- not full-strength and not invisible.
    #[test]
    fn border_subtle_is_derived_from_border_alpha_when_absent() {
        let loaded = LoadedTheme::from_toml(builtin_theme_sources()[3]).unwrap(); // Jackfruit Dark
        assert!(!builtin_theme_sources()[3].contains("border_subtle"));
        let (border, subtle) = (loaded.theme.border, loaded.theme.border_subtle);
        assert_eq!(subtle.h, border.h);
        assert_eq!(subtle.s, border.s);
        assert_eq!(subtle.l, border.l);
        assert_eq!(subtle.a, border.a * 0.55);
        assert!(subtle.a < border.a, "the hairline must be dimmer, not equal");
    }

    /// A theme written before these tokens existed loads unchanged.
    #[test]
    fn a_theme_without_the_new_keys_still_loads() {
        let src = include_str!("../assets/themes/nord.toml");
        assert!(!src.contains("page_bg"), "fixture assumption: nord predates page_bg");
        let t = LoadedTheme::from_toml(src).expect("nord loads");
        assert!(t.theme.page_bg.l > 0., "derived rather than defaulted to nothing");
    }

    /// Optional keys, when present, override the derivation entirely.
    #[test]
    fn explicit_page_border_and_shadow_keys_override_derivation() {
        let toml_src = builtin_theme_sources()[3].replace(
            "[syntax]",
            "page_bg = \"#123456\"\nborder_subtle = \"#654321\"\nshadow = \"#0f0f0f\"\n[syntax]",
        );
        let loaded = LoadedTheme::from_toml(&toml_src).unwrap();
        assert_eq!(loaded.theme.page_bg, gpui::rgb(0x123456).into());
        assert_eq!(loaded.theme.border_subtle, gpui::rgb(0x654321).into());
        assert_eq!(loaded.theme.shadow, gpui::rgb(0x0f0f0f).into());
    }

    /// WCAG contrast is symmetric and bottoms out at 1.0 for identical
    /// colours, independent of argument order.
    #[test]
    fn contrast_is_symmetric_and_bounded() {
        let black = Hsla { h: 0., s: 0., l: 0., a: 1. };
        let white = Hsla { h: 0., s: 0., l: 1., a: 1. };
        let ratio = Theme::contrast(black, white);
        assert!((ratio - 21.0).abs() < 0.01, "black/white should be ~21:1, got {ratio}");
        assert_eq!(Theme::contrast(black, white), Theme::contrast(white, black));
        assert_eq!(Theme::contrast(black, black), 1.0);
    }
}

/// All known themes + the user's choices + current system appearance.
pub struct ThemeState {
    pub themes: Vec<LoadedTheme>,
    pub settings: crate::settings::Settings,
    pub system_dark: bool,
    /// Flux day↔night factor (0 day, 1 night); ignored while flux is
    /// disabled. Kept current by the minute timer in main.rs.
    pub flux_blend: f32,
}

impl Global for ThemeState {}

impl ThemeState {
    pub fn resolve(&self) -> Arc<Theme> {
        let flux = &self.settings.flux;
        // Flux night forces the dark theme; day still follows the
        // system appearance.
        let dark = if flux.enabled && flux.auto_dark && self.flux_blend >= 0.5 {
            true
        } else {
            self.system_dark
        };
        let want = if dark {
            &self.settings.dark_theme
        } else {
            &self.settings.light_theme
        };
        let picked = self
            .themes
            .iter()
            .find(|t| &t.name == want && t.theme.is_dark == dark)
            .or_else(|| self.themes.iter().find(|t| t.theme.is_dark == dark))
            .map(|t| t.theme.clone())
            .unwrap_or_else(|| Arc::new(if dark { Theme::dark() } else { Theme::light() }));
        if flux.enabled && flux.warm_shift && self.flux_blend > 0.0 {
            Arc::new(crate::flux::warm_theme(
                &picked,
                self.flux_blend,
                flux.night_kelvin,
            ))
        } else {
            picked
        }
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
            flux_blend: 0.0,
        }
    }

    #[test]
    fn flux_night_forces_dark_and_warms() {
        let mut s = state("Paper", "Nord", false);
        s.settings.flux.enabled = true;
        s.flux_blend = 1.0;
        let resolved = s.resolve();
        assert!(resolved.is_dark, "night overrides a light system appearance");
        // Warm shift trimmed blue against the plain dark theme.
        let plain = state("Paper", "Nord", true).resolve();
        let plain_bg = gpui::Rgba::from(plain.fg);
        let warmed_bg = gpui::Rgba::from(resolved.fg);
        assert!(warmed_bg.b < plain_bg.b, "{} < {}", warmed_bg.b, plain_bg.b);

        // auto_dark off: theme follows the system, warmth still applies.
        s.settings.flux.auto_dark = false;
        assert!(!s.resolve().is_dark);
    }

    #[test]
    fn flux_disabled_or_daytime_changes_nothing() {
        let mut s = state("Paper", "Nord", false);
        s.flux_blend = 1.0; // stale blend with flux off: ignored
        let off = s.resolve();
        assert!(!off.is_dark);
        assert_eq!(off.bg, state("Paper", "Nord", false).resolve().bg);

        s.settings.flux.enabled = true;
        s.flux_blend = 0.0; // enabled but daytime: byte-identical
        assert_eq!(s.resolve().bg, off.bg);
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
