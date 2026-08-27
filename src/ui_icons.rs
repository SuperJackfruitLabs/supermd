//! Hand-authored UI icons, served under `icons/ui/`.
//!
//! Deliberately separate from `seti.rs`, which is GENERATED from the
//! Seti UI file-type set and must not be edited: regenerating that file
//! would otherwise destroy anything added here. Same convention as
//! `seti_tests.rs`.

/// `(name, svg bytes)` for every icon the chrome can draw.
pub const ICONS: &[(&str, &[u8])] = &[
    ("sidebar", include_bytes!("../assets/icons/ui/sidebar.svg")),
    ("outline", include_bytes!("../assets/icons/ui/outline.svg")),
    ("knowledge", include_bytes!("../assets/icons/ui/knowledge.svg")),
    ("changes", include_bytes!("../assets/icons/ui/changes.svg")),
    ("plus", include_bytes!("../assets/icons/ui/plus.svg")),
    ("sun", include_bytes!("../assets/icons/ui/sun.svg")),
    ("graph", include_bytes!("../assets/icons/ui/graph.svg")),
];

/// Asset path for an icon, as `gpui::svg().path(..)` wants it.
pub fn path(name: &str) -> String {
    format!("icons/ui/{name}.svg")
}

/// The bytes for `name`, if it exists.
pub fn bytes(name: &str) -> Option<&'static [u8]> {
    ICONS.iter().find(|(n, _)| *n == name).map(|(_, b)| *b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_is_wellformed_svg() {
        for (name, bytes) in ICONS {
            let svg = std::str::from_utf8(bytes).expect("{name} is utf-8");
            assert!(svg.starts_with("<svg"), "{name} starts with an svg root");
            assert!(svg.contains("viewBox=\"0 0 16 16\""), "{name} is a 16px icon");
            assert!(
                svg.contains("currentColor"),
                "{name} must inherit its colour so themes tint it"
            );
        }
    }

    #[test]
    fn lookup_finds_icons_by_name_and_builds_asset_paths() {
        assert!(bytes("sidebar").is_some());
        assert!(bytes("nope").is_none());
        assert_eq!(path("sun"), "icons/ui/sun.svg");
    }

    #[test]
    fn icon_names_are_unique() {
        let mut names: Vec<&str> = ICONS.iter().map(|(n, _)| *n).collect();
        names.sort_unstable();
        let before = names.len();
        names.dedup();
        assert_eq!(before, names.len(), "duplicate icon name");
    }
}
