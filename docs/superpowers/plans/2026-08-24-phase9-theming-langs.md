# Phase 9: Theming + Language Coverage Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:executing-plans.

**Spec:** `docs/superpowers/specs/2026-08-24-phase9-theming-langs-design.md`

### Task 1: Mapping expansion (TDD)
- [ ] RED (new tests in highlight.rs):
```rust
#[test]
fn maps_filenames_and_new_extensions() {
    use std::path::Path;
    let f = |p: &str| language_for_file(Path::new(p));
    assert_eq!(f("Dockerfile"), Some("dockerfile"));
    assert_eq!(f("Makefile"), Some("make"));
    assert_eq!(f("meson.build"), Some("meson"));
    assert_eq!(f("App.kt"), Some("kotlin"));
    assert_eq!(f("sim.jl"), Some("julia"));
    assert_eq!(f("core.clj"), Some("clojure"));
    assert_eq!(f("theme.scss"), Some("scss"));
    assert_eq!(f("main.tf"), Some("hcl"));
    assert_eq!(f("shader.wgsl"), Some("wgsl"));
    assert_eq!(f("view.m"), Some("objc"));
    assert_eq!(f("x.fs"), None); // F# vs Forth ambiguity
    assert_eq!(f("noext"), None);
}
```
- [ ] GREEN: `pub fn language_for_file(path: &Path) -> Option<&'static str>` in highlight.rs — exact-filename match first, then the full extension table (all current mappings + inkjet coverage). `reader::language_for_path` becomes a thin delegate (or callers move to the new fn); editor Provider detection uses it.
- [ ] Full suite; commit.

### Task 2: Extras registry — XML + GraphQL (smoke TDD)
- [ ] `cargo add tree-sitter-xml@0.7 tree-sitter-graphql@0.2`; vendor Helix `runtime/queries/graphql/highlights.scm` → `assets/queries/graphql-highlights.scm` + MPL attribution README.
- [ ] RED: smoke cases `("xml", "<a b=\"c\">x</a>\n")`, `("graphql", "type Query { a: Int }\n")` in the grammar matrix.
- [ ] GREEN: check ts-highlight 0.23 `HighlightConfiguration::new` signature in the vendored registry source; build extras in `Languages::new()` (now holding `extras: Vec<(&str, HighlightConfiguration)>`), configured with inkjet HIGHLIGHT_NAMES; `highlight()` consults extras first. Restore xml/gql aliases + mapping entries.
- [ ] Full suite; commit.

### Task 3: Theme files + loader + settings (TDD)
- [ ] `cargo add serde --features derive` + `toml`.
- [ ] RED (theme.rs/settings.rs tests): hex "#dd4c4f" parses; bad hex errors; `ThemeFile` TOML round-trip; `Theme::from_file` maps every field; every builtin in `builtin_themes()` parses with correct `is_dark`; settings default (Paper/Graphite), save→load round-trip in a temp dir (dir passed as param).
- [ ] GREEN: `ThemeFile` (serde) with all color fields as strings; `parse_hex`; `builtin_themes()` from `include_str!` of `assets/themes/{paper,graphite,solarized-light,solarized-dark,nord,gruvbox-dark}.toml`; `load_custom_themes(dir)`; `settings.rs` with `Settings { light_theme, dark_theme }`, `load(dir)/save(dir)`.
- [ ] Write the six TOML files (Paper/Graphite = current palettes; Solarized/Nord/Gruvbox from their canonical palettes, syntax slots hand-assigned).
- [ ] Full suite; commit.

### Task 4: Registry + picker + wiring (shell)
- [ ] `ThemeState` global: `themes: Vec<(name, is_dark, Arc<Theme>)>` (builtins + custom), `settings`, `system_dark: bool`. `apply_system_appearance` resolves via settings names (fallback Paper/Graphite). main.rs loads state at startup.
- [ ] Picker overlay (⌘T + View menu): grouped Light/Dark rows, arrow keys move + apply live (`ActiveTheme` swap + notify), Enter writes the picked theme into its appearance slot and saves settings, Escape restores the pre-open ActiveTheme + settings. `"ThemePicker"` key context (up/down/enter/escape bindings).
- [ ] Shortcuts dialog gains the ⌘T row.
- [ ] Manual verify incl. custom theme file drop-in; commit.

### Task 5: Docs + finish + push
- [ ] README (themes section, language count), WELCOME roadmap; finishing skill; push.

## Self-review
Mapping tests cover filename/ext/unmapped; extras unified capture table stated; builtin-parse test prevents shipping broken TOML; settings dir injection keeps tests hermetic; picker revert semantics stated. Types: `language_for_file` T1↔T2 callers; `ThemeFile`/`builtin_themes`/`Settings` T3↔T4. ✓
