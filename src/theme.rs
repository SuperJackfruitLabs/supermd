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

            bg: rgb(0xffffff).into(),
            fg: rgb(0x37352f).into(),
            fg_strong: rgb(0x1d1c19).into(),
            fg_muted: rgb(0x8f8d87).into(),
            accent: rgb(0xdd4c4f).into(),
            link: rgb(0xdd4c4f).into(),
            code_bg: rgb(0xf6f5f3).into(),
            code_fg: rgb(0x484744).into(),
            border: rgb(0xe9e8e5).into(),

            panel_bg: rgb(0xf7f6f4).into(),
            hover_bg: rgb(0xedecea).into(),
            selected_bg: rgb(0xe6e4e0).into(),
            find_match_bg: rgb(0xffe9a3).into(),
            find_active_bg: rgb(0xffc94d).into(),

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

            body_family: ".SystemUIFont".into(),
            mono_family: "Menlo".into(),

            body_size: 16.0,
            body_line_height: 1.65,
            code_size: 13.0,
            ui_size: 13.0,
        }
    }

    pub fn dark() -> Self {
        Self {
            is_dark: true,

            bg: rgb(0x1f1f1e).into(),
            fg: rgb(0xd6d4cf).into(),
            fg_strong: rgb(0xf1efec).into(),
            fg_muted: rgb(0x8b8984).into(),
            accent: rgb(0xe25d5f).into(),
            link: rgb(0xe25d5f).into(),
            code_bg: rgb(0x2a2a28).into(),
            code_fg: rgb(0xccc9c2).into(),
            border: rgb(0x343432).into(),

            panel_bg: rgb(0x262624).into(),
            hover_bg: rgb(0x30302e).into(),
            selected_bg: rgb(0x3a3a37).into(),
            find_match_bg: rgb(0x51431a).into(),
            find_active_bg: rgb(0x7a6220).into(),

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

            body_family: ".SystemUIFont".into(),
            mono_family: "Menlo".into(),

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

pub fn apply_system_appearance(appearance: WindowAppearance, cx: &mut App) {
    let dark = matches!(
        appearance,
        WindowAppearance::Dark | WindowAppearance::VibrantDark
    );
    if cx.global::<ActiveTheme>().0.is_dark != dark {
        cx.set_global(ActiveTheme(Arc::new(if dark {
            Theme::dark()
        } else {
            Theme::light()
        })));
    }
}
