//! Tests for the GENERATED `seti` module (`src/seti.rs`). They live in
//! this hand-written file so regenerating the vendor drop never
//! destroys them.

use crate::seti::icon_for;

#[test]
fn extension_rules() {
    assert_eq!(icon_for("main.rs").0, "rust");
    assert_eq!(icon_for("app.ts").0, "typescript");
    assert_eq!(icon_for("app.tsx").0, "react"); // Seti's actual mapping
    assert_eq!(icon_for("notes.md").0, "markdown");
    assert_eq!(icon_for("x.unknownext").0, "default");
}

#[test]
fn exact_and_substring_rules() {
    assert_eq!(icon_for("Dockerfile").0, "docker");
    assert_eq!(icon_for("webpack.config.js").0, "webpack");
}

#[test]
fn specific_names_beat_bare_extensions() {
    assert_ne!(icon_for("tsconfig.json").0, icon_for("other.json").0);
}

#[test]
fn case_insensitive() {
    assert_eq!(icon_for("MAIN.RS").0, "rust");
}

#[test]
fn every_referenced_icon_is_embedded() {
    use std::collections::HashSet;
    let embedded: HashSet<&str> = crate::seti::ICONS.iter().map(|(n, _)| *n).collect();
    for (_, icon, _) in crate::seti::ALL_RULES_FOR_TEST {
        assert!(embedded.contains(icon), "missing svg for {icon}");
    }
    assert!(embedded.contains("default"));
    assert!(embedded.contains("folder"));
}
