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
                // The ground is the cream one step below the page --
                // the desk the sheet rests on, not the sheet.
                bg: rgb(0xf8f4ea).into(),
                // Ink shares the ground's warm hue rather than sitting
                // neutral-to-black on it -- a warm palette reads as
                // dated the moment the ink stops matching the paper's
                // temperature.
                fg: rgb(0x353027).into(),
                fg_strong: rgb(0x221f19).into(),
                fg_muted: rgb(0x827969).into(),
                accent: rgb(0xc9821c).into(),
                link: rgb(0xc9821c).into(),
                code_bg: rgb(0xf6f2e9).into(),
                code_fg: rgb(0x4a463d).into(),
                border: rgb(0xeae5d8).into(),
                // The warm cream itself. It is the identity of the
                // theme, and the page is where the reading happens, so
                // the page is what gets it.
                page_bg: rgb(0xfdfbf6).into(),
                border_subtle: Hsla { a: 0.55, ..rgb(0xeae5d8).into() },
                shadow: Hsla { h: 0.095, s: 0.30, l: 0.18, a: 0.18 },

                panel_bg: rgb(0xf3efe2).into(),
                hover_bg: rgb(0xebe6d8).into(),
                selected_bg: rgb(0xe3dcc9).into(),
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
                bg: rgb(0x171612).into(),
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
                // The warm charcoal the theme is named for. It used to
                // be `bg`, and it used to equal `code_bg` -- which made
                // a fenced block invisible on the page it sat on.
                page_bg: rgb(0x211f1a).into(),
                border_subtle: Hsla { a: 0.55, ..rgb(0x383428).into() },
                shadow: Hsla { h: 0., s: 0., l: 0., a: 0.34 },

                panel_bg: rgb(0x1c1b18).into(),
                hover_bg: rgb(0x25231d).into(),
                selected_bg: rgb(0x302d24).into(),
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

    /// The strongest a theme's `shadow` may be, enforced at load.
    ///
    /// Above this it is not a falloff, it is a slab: the page's outer
    /// shadow layer carries 0.6 of the theme alpha, so an opaque black
    /// paints a hard 60%-black band around the document. Six digits of
    /// hex parse opaque, and `shadow = "#000000"` is the obvious thing
    /// to write, so the format makes that mistake easy to make and the
    /// user's own theme file is the one place no test can reach. A
    /// documentation line asks; this enforces.
    ///
    /// It clamps rather than rejecting. A shadow that is too strong is
    /// cosmetic, and a theme should not fail to load -- taking the
    /// whole colour scheme away -- over one token.
    pub const MAX_SHADOW_ALPHA: f32 = 0.5;

    /// One shadow colour. Warm-shifted in light themes so the page
    /// does not cast a cold grey shadow onto a warm ground.
    ///
    /// The light alpha is 0.18, not the 0.11 it started at. Measured on
    /// screen at the shipped 10px inset, 0.11 dropped the ground by
    /// about 15/255 at the page edge and faded over roughly six points
    /// -- present in a pixel sample, not visible as a lift. The page
    /// was not short of room, it was short of contrast.
    pub fn derive_shadow(is_dark: bool) -> Hsla {
        if is_dark {
            Hsla { h: 0., s: 0., l: 0., a: 0.34 }
        } else {
            Hsla { h: 0.095, s: 0.30, l: 0.18, a: 0.18 }
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

/// `#rrggbb`, or `#rrggbbaa` when a colour needs to be translucent.
///
/// The alpha form exists because `shadow` is a *tint plus a strength*,
/// and six digits can only say the tint. A theme that wrote
/// `shadow = "#000000"` before this would get a fully opaque black --
/// `elevation::shadows` scales the theme alpha, so an opaque shadow
/// paints the page's drop shadow as a solid slab rather than a falloff.
/// The derived shadows have always carried alpha; the file format just
/// had no way to spell it.
pub fn parse_hex(s: &str) -> Result<Hsla, String> {
    let hex = s.strip_prefix('#').unwrap_or(s);
    let value = match hex.len() {
        6 | 8 => u32::from_str_radix(hex, 16).map_err(|e| format!("bad hex color {s:?}: {e}"))?,
        _ => return Err(format!("bad hex color {s:?}")),
    };
    Ok(if hex.len() == 8 { gpui::rgba(value).into() } else { rgb(value).into() })
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
            Some(hex) => {
                let c = parse_hex(hex)?;
                Hsla { a: c.a.min(Theme::MAX_SHADOW_ALPHA), ..c }
            }
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

    /// A theme file with none of the surface keys -- the shape a user
    /// theme written before they existed still has. The shipped themes
    /// all declare `page_bg` and `shadow` now, so the derivations they
    /// exist for can only be exercised against a fixture like this one.
    const THEME_WITHOUT_SURFACE_KEYS: &str = r##"
name = "No Surfaces"
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
    /// One shipped theme cannot meet the body floor, and the exception
    /// below is permanent rather than a to-do.
    ///
    /// Solarized's light body ink is base00 `#657b83`. Against base3
    /// `#fdf6e3` it is 4.13:1 and against pure white -- the brightest
    /// page any theme could have -- it is 4.4546:1. There is no page
    /// colour that lifts it to 4.5:1, so the only fix is to change
    /// base00, and base00 is not a detail of the theme: Solarized is a
    /// published sixteen-colour palette built on measured *relative*
    /// lightness relationships, and its author chose those numbers
    /// deliberately. A Solarized theme with different ink is a theme
    /// wearing Solarized's name. The page here is set as bright as it
    /// can go while still reading as Solarized, which is the most that
    /// can be done from this side.
    ///
    /// Everything else that used to sit in this table is gone, fixed in
    /// the theme rather than recorded here: `solarized-dark`'s body
    /// (the page now carries base03, which is what base0 was designed
    /// against) and the muted text of `nord`, `solarized-dark` and
    /// `solarized-light`. Secondary chrome text is not a published
    /// Solarized or Nord role, so tuning it to the floor costs those
    /// themes nothing they had. `solarized-dark`'s muted entry came
    /// back once the assertion started reading the surface that binds
    /// (see `worst_muted_contrast`); moving `fg_muted` from base01 to
    /// base00 bought it the page and the sidebar but not a selected
    /// row, and it is now a permanent muted exception below, for the
    /// same published-ink reason as this one.
    ///
    /// The same fact has a third symptom that is recorded here and not
    /// guarded: body ink on `selected_bg` is 3.38:1 in solarized-dark
    /// and 3.28:1 in solarized-light (7.2-9.6:1 in the other eight).
    /// That is the search overlay's matched line and an open file's
    /// name in the sidebar. It is the same root cause and has the same
    /// non-fix, so widening this assertion to `selected_bg` would add
    /// two more entries to this table and change no pixel; the number
    /// is written down here instead, where the next person measuring
    /// will find it.
    ///
    /// The entry is a *band*, not a hole: drift in either direction
    /// fails. If it ever clears 4.5, delete it instead of widening it.
    const KNOWN_BODY_GAPS: &[(&str, f32, f32)] = &[("solarized-light", 4.2, 4.5)];
    /// Two permanent muted exceptions, and they are the *same fact* as
    /// the body exception above rather than a second problem.
    ///
    /// Solarized's ink is published low-contrast by design: base0 on
    /// base03 is 4.75:1 and base00 on base3 is 4.34:1, the lowest body
    /// contrast of anything we ship, and everything else has to fit
    /// underneath that. Muted ink must also clear 3:1 on `selected_bg`,
    /// the far end of the surface ramp. Solving for the ink that does
    /// gives 4.29:1 on the page in solarized-dark and 4.22:1 in
    /// solarized-light -- 90% and 97% of the body ink's own contrast.
    /// Contrast is a function of luminance alone, so there is no hue or
    /// saturation that buys this back: the only `fg_muted` that clears
    /// the floor is one that reads as body text, which is not a muted
    /// token, it is a second body token. For comparison, the themes
    /// that pass sit at 37-56% of their body ink.
    ///
    /// The surface side was checked before accepting this, and it is
    /// worse. Lowering solarized-dark's `selected_bg` until base00
    /// clears leaves `selected_bg`/`hover_bg` at 1.025:1 -- below the
    /// floor in
    /// `every_background_token_is_visible_on_what_it_is_painted_on`, so
    /// a readable hint would cost a visible selection. In
    /// solarized-light the ink fails at 2.84:1 even on `hover_bg`, so
    /// no selection colour lighter than hover -- which would invert the
    /// ladder -- reaches 3:1 either.
    ///
    /// Both are *bands*, not holes: drift down fails, and so does
    /// drift up past the floor. If either ever clears 3.0, delete it
    /// rather than widening it.
    const KNOWN_MUTED_GAPS: &[(&str, f32, f32)] =
        &[("solarized-dark", 2.3, 3.0), ("solarized-light", 2.45, 3.0)];

    /// Every surface muted text is actually painted on -- all five --
    /// reduced to the worst one.
    ///
    /// This began as `contrast(fg_muted, bg)` alone, and that was the
    /// wrong reference: `bg` is the *desk*, and almost nothing writes
    /// muted text on the desk. The five real surfaces are
    ///
    /// - `bg`, the desk;
    /// - `page_bg` -- the horizontal rule's label, strikethrough,
    ///   projector captions (`editor/mod.rs`, `view.rs`,
    ///   `editor/projector.rs`);
    /// - `panel_bg` -- the sidebar, the tab strip's inactive labels,
    ///   and every floating overlay's ground;
    /// - `hover_bg` -- the sidebar chevron and folder icon are
    ///   unconditionally `fg_muted` (`workspace.rs`), so hovering any
    ///   directory row paints them on it, as does hovering an inactive
    ///   tab;
    /// - `selected_bg` -- the finder's directory hint, the palette's
    ///   plugin name, the search results' line numbers, the install
    ///   list's descriptions and the `[[` popup's path hint are all
    ///   `fg_muted` on a *selected* row.
    ///
    /// The ramp runs `bg` -> `selected_bg` and `fg_muted` is one ink
    /// for all of it, so `selected_bg` -- the far end -- is the binding
    /// surface in every shipped theme. Each time this set grew, the
    /// narrower version had been flattering every theme at once: `bg`
    /// hid 2.79:1 muted text in solarized-dark, and the three-surface
    /// version hid 2.67:1 in paper. Taking the minimum means a theme
    /// can only pass by being readable everywhere it writes, and moving
    /// a surface can never fix it -- only the ink can.
    fn worst_muted_contrast(t: &Theme) -> f32 {
        [t.bg, t.page_bg, t.panel_bg, t.hover_bg, t.selected_bg]
            .into_iter()
            .map(|surface| Theme::contrast(t.fg_muted, surface))
            .fold(f32::INFINITY, f32::min)
    }

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
            let muted = worst_muted_contrast(&theme);
            match KNOWN_MUTED_GAPS.iter().find(|(n, _, _)| *n == name) {
                Some((_, floor, ceiling)) => assert!(
                    muted >= *floor && muted < *ceiling,
                    "{name}: muted text on its worst surface drifted to {muted:.2}:1, expected [{floor}, {ceiling}) --                      if it cleared {ceiling}, delete this exception instead of widening it; if the assertion stopped reading a surface, put it back"
                ),
                None => assert!(
                    muted >= 3.0,
                    "{name}: muted text is {muted:.2}:1 on the worst of ground, page, panel, hover and selection"
                ),
            }
            let surfaces = Theme::contrast(theme.page_bg, theme.bg);
            assert!(
                surfaces >= 1.03,
                "{name}: page and ground are indistinguishable ({surfaces:.3}:1)"
            );
        }
    }

    /// A code fence must stay visible against the page it now sits on.
    ///
    /// `code_bg` was tuned against the window background. Once the
    /// document moved onto its own brighter surface, several themes had
    /// a fence within a hair of the page under it -- `jackfruit-dark`
    /// and the built-in dark were at 1.000:1, literally the same
    /// colour. Giving each page the theme's own background restores the
    /// separation the theme author drew, which is why this passes with
    /// room now rather than by a nudge.
    #[test]
    fn code_fences_stay_visible_on_the_page() {
        for (name, theme) in shipped_themes() {
            let separation = Theme::contrast(theme.code_bg, theme.page_bg);
            assert!(
                separation >= 1.04,
                "{name}: code_bg is invisible on the page ({separation:.3}:1)"
            );
            let text = Theme::contrast(theme.code_fg, theme.code_bg);
            assert!(text >= 4.5, "{name}: code text is {text:.2}:1");
        }
    }

    /// A background token is only a signal if it differs from what it
    /// is painted on. `code_bg` was the first of these found collapsed
    /// onto the page; it was not the only one.
    ///
    /// `hover_bg` and `panel_bg` land ON THE PAGE, not only in chrome.
    /// The table projector paints its header row with `panel_bg` and
    /// hovers a body row with `hover_bg` (`editor/mod.rs`), and the tab
    /// strip hovers an inactive tab to `hover_bg` while the active tab
    /// carries `page_bg` (`workspace.rs`) -- so a collision there both
    /// kills the hover feedback and makes a hovered tab read as the
    /// active one. `hover_bg` and `selected_bg` also land on `panel_bg`
    /// (sidebar rows, the finder, the palette, the `[[` completion
    /// popup), and the sidebar puts them directly side by side:
    /// keyboard-selected is `hover_bg`, active is `selected_bg`.
    ///
    /// `selected_bg` reaches the page too, in one place that a grep for
    /// it next to `page_bg` will not show: a tab's close button hovers
    /// to `selected_bg`, and the tab around it is `page_bg` when active
    /// and `bg` when not (`workspace.rs`). Both pairs are listed.
    ///
    /// The floor is the fence's, for the fence's reason -- below it the
    /// difference is a couple of levels out of 255 and nothing appears
    /// to happen when the pointer moves.
    #[test]
    fn every_background_token_is_visible_on_what_it_is_painted_on() {
        for (name, t) in shipped_themes() {
            for (what, ink, surface) in [
                ("a hovered table row, and a hovered tab", t.hover_bg, t.page_bg),
                ("a table header", t.panel_bg, t.page_bg),
                ("a hovered sidebar / finder / popup row", t.hover_bg, t.panel_bg),
                ("a selected sidebar / finder / popup row", t.selected_bg, t.panel_bg),
                ("selection against hover, side by side", t.selected_bg, t.hover_bg),
                ("a tab's close button, hovered on the active tab", t.selected_bg, t.page_bg),
                ("a tab's close button, hovered on an inactive tab", t.selected_bg, t.bg),
            ] {
                let separation = Theme::contrast(ink, surface);
                assert!(
                    separation >= 1.04,
                    "{name}: {what} is invisible against what it sits on ({separation:.3}:1)"
                );
            }
        }
    }

    /// A theme that omits `shadow` gets the appearance-appropriate
    /// derived shadow (warm-shifted in light, opaque black in dark) --
    /// not a leftover value from whichever appearance `Theme::light()`/
    /// `Theme::dark()` started from before the file's colours were
    /// applied.
    #[test]
    fn shadow_is_derived_per_appearance_when_absent() {
        assert!(!THEME_WITHOUT_SURFACE_KEYS.contains("shadow"));
        let dark = LoadedTheme::from_toml(THEME_WITHOUT_SURFACE_KEYS).unwrap();
        assert_eq!(dark.theme.shadow, Theme::derive_shadow(true));

        let light_src = THEME_WITHOUT_SURFACE_KEYS.replace("\"dark\"", "\"light\"");
        let light = LoadedTheme::from_toml(&light_src).unwrap();
        assert_eq!(light.theme.shadow, Theme::derive_shadow(false));

        assert_ne!(light.theme.shadow, dark.theme.shadow);
    }

    /// A theme that omits `border_subtle` gets its own `border` colour
    /// at reduced alpha -- same hue/saturation/lightness, a dimmer
    /// hairline -- not full-strength and not invisible.
    #[test]
    fn border_subtle_is_derived_from_border_alpha_when_absent() {
        assert!(!THEME_WITHOUT_SURFACE_KEYS.contains("border_subtle"));
        let loaded = LoadedTheme::from_toml(THEME_WITHOUT_SURFACE_KEYS).unwrap();
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
        assert!(!THEME_WITHOUT_SURFACE_KEYS.contains("page_bg"));
        let light_src = THEME_WITHOUT_SURFACE_KEYS.replace("\"dark\"", "\"light\"");
        for (appearance, src) in
            [("dark", THEME_WITHOUT_SURFACE_KEYS.to_string()), ("light", light_src)]
        {
            let t = LoadedTheme::from_toml(&src).expect("loads").theme;
            // The same band and floor the shipped themes are held to.
            // Every one of those now declares `page_bg` outright, so
            // without this the derivation could quietly become a no-op
            // and only a *user* theme -- the thing it exists for --
            // would show the damage.
            let delta = (t.page_bg.l - t.bg.l).abs();
            assert!(
                (0.012..=0.075).contains(&delta),
                "{appearance}: derived page/ground delta {delta} is not one adjacent step"
            );
            let surfaces = Theme::contrast(t.page_bg, t.bg);
            assert!(
                surfaces >= 1.03,
                "{appearance}: derived page and ground are indistinguishable ({surfaces:.3}:1)"
            );
        }
    }

    /// Optional keys, when present, override the derivation entirely.
    #[test]
    fn explicit_page_border_and_shadow_keys_override_derivation() {
        let toml_src = THEME_WITHOUT_SURFACE_KEYS.replace(
            "[syntax]",
            "page_bg = \"#123456\"\nborder_subtle = \"#654321\"\nshadow = \"#0f0f0f57\"\n[syntax]",
        );
        let loaded = LoadedTheme::from_toml(&toml_src).unwrap();
        assert_eq!(loaded.theme.page_bg, gpui::rgb(0x123456).into());
        assert_eq!(loaded.theme.border_subtle, gpui::rgb(0x654321).into());
        assert_eq!(loaded.theme.shadow, parse_hex("#0f0f0f57").unwrap());
    }

    /// A six-digit colour is opaque; an eight-digit one carries its own
    /// alpha. `shadow` needs the second form: `elevation::shadows`
    /// scales the theme's alpha per layer, so a shadow parsed opaque
    /// paints the page's falloff as a solid slab. Before eight-digit
    /// hex, a theme file simply could not say "black at a third".
    #[test]
    fn eight_digit_hex_carries_alpha_and_six_digit_stays_opaque() {
        assert_eq!(parse_hex("#0f0f0f").unwrap().a, 1.0);
        let translucent = parse_hex("#0f0f0f57").unwrap();
        assert!(
            (translucent.a - 87. / 255.).abs() < 1e-6,
            "alpha byte should survive: got {}",
            translucent.a
        );
        let opaque = parse_hex("#0f0f0f").unwrap();
        assert_eq!((translucent.h, translucent.s, translucent.l), (opaque.h, opaque.s, opaque.l));
        assert!(parse_hex("#0f0f0f5").is_err(), "seven digits is not a colour");
        assert!(parse_hex("#0f0f0f577").is_err(), "nine digits is not a colour");
    }

    /// Every shipped theme's `shadow` must be translucent. An opaque
    /// one is not a shadow -- `elevation::shadows` multiplies it by the
    /// per-layer alpha, so at a: 1.0 the page's outer layer lands at
    /// 0.6 of solid colour and reads as a painted border.
    ///
    /// The bound is 0.4, not `MAX_SHADOW_ALPHA`: the load-time clamp
    /// already guarantees 0.5, so asserting 0.5 here would be asserting
    /// the clamp rather than the themes. The shipped values are 0.18
    /// (light) and 0.34 (dark), so 0.4 leaves headroom and still bites.
    #[test]
    fn no_shipped_theme_casts_an_opaque_shadow() {
        for (name, theme) in shipped_themes() {
            assert!(
                theme.shadow.a > 0.0 && theme.shadow.a < 0.4,
                "{name}: shadow alpha {} is not a shadow",
                theme.shadow.a
            );
        }
    }

    /// A theme file cannot cast a slab. Six-digit hex parses opaque,
    /// which is what a theme author writing `shadow = "#000000"` gets,
    /// and no test can reach a user's own theme directory -- so the
    /// load clamps it. The tint survives; only the strength is capped.
    #[test]
    fn an_opaque_shadow_in_a_theme_file_is_clamped_not_honoured() {
        let with = |hex: &str| {
            let src = THEME_WITHOUT_SURFACE_KEYS
                .replace("[syntax]", &format!("shadow = \"{hex}\"\n[syntax]"));
            LoadedTheme::from_toml(&src).expect("loads").theme.shadow
        };
        let opaque = with("#000000");
        assert_eq!(opaque.a, Theme::MAX_SHADOW_ALPHA, "an opaque shadow must be capped");
        let tinted = with("#12100e");
        let raw = parse_hex("#12100e").unwrap();
        assert_eq!(
            (tinted.h, tinted.s, tinted.l),
            (raw.h, raw.s, raw.l),
            "the clamp caps strength, it does not repaint the tint"
        );
        let honest = with("#12100e57");
        assert_eq!(honest, parse_hex("#12100e57").unwrap(), "a shadow under the cap is untouched");
        assert!(honest.a < Theme::MAX_SHADOW_ALPHA);
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
        // Precedence: an explicit appearance beats both flux's night
        // override and the system. Under System, flux night still
        // forces dark and day still follows the system appearance --
        // unchanged from before this setting existed.
        let dark = match self.settings.appearance {
            crate::settings::Appearance::Light => false,
            crate::settings::Appearance::Dark => true,
            crate::settings::Appearance::System => {
                if flux.enabled && flux.auto_dark && self.flux_blend >= 0.5 {
                    true
                } else {
                    self.system_dark
                }
            }
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

    /// An explicit appearance beats both the system and flux. A
    /// setting that says "always Light" and then goes dark at night is
    /// a setting that lies.
    #[test]
    fn an_explicit_appearance_overrides_system_and_flux() {
        use crate::settings::Appearance;
        let mut st = state("Paper", "Nord", true);

        st.settings.appearance = Appearance::Light;
        assert!(!st.resolve().is_dark, "explicit Light beats a dark system");

        st.settings.appearance = Appearance::Dark;
        st.system_dark = false;
        assert!(st.resolve().is_dark, "explicit Dark beats a light system");

        // Flux night would force dark; an explicit Light still wins.
        st.settings.appearance = Appearance::Light;
        st.settings.flux.enabled = true;
        st.settings.flux.auto_dark = true;
        st.flux_blend = 1.0;
        assert!(!st.resolve().is_dark, "explicit Light beats flux night");
    }

    /// System is the default and keeps today's behaviour exactly,
    /// including flux's night override.
    #[test]
    fn system_appearance_keeps_todays_behaviour() {
        use crate::settings::Appearance;
        let mut st = state("Paper", "Nord", true);
        st.settings.appearance = Appearance::System;

        st.system_dark = true;
        assert!(st.resolve().is_dark);
        st.system_dark = false;
        assert!(!st.resolve().is_dark);

        st.settings.flux.enabled = true;
        st.settings.flux.auto_dark = true;
        st.flux_blend = 1.0;
        assert!(st.resolve().is_dark, "flux night still forces dark under System");
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

