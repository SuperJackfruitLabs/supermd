# Phase 6: File Types Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Full Seti icon pack in the sidebar, 8 more grammars, image viewer tabs.

**Spec:** `docs/superpowers/specs/2026-08-23-phase6-filetypes-design.md`

## Global Constraints

- TDD for `seti::icon_for`, `is_image_path`, grammar smoke tests.
- Vendored assets committed with Seti's MIT license file; generator script committed; builds offline.
- Suite green per commit; repo trailers.

### Task 1: Vendor Seti + generated mapping (TDD on icon_for)

- [ ] `scripts/vendor_seti.py`: download 169 SVGs + LICENSE from jesseweed/seti-ui@master into `assets/icons/seti/`; parse `mapping.less`; emit `src/seti.rs` (embedded bytes table via `include_bytes!`, rule tables, `icon_for`, `SetiColor`). Run it.
- [ ] RED: hand-written tests in `src/seti_test_support` — actually inline `#[cfg(test)]` appended to generated file is regenerated-over; instead tests live in `src/seti_tests.rs` (`mod seti_tests;` under cfg(test)) so regeneration never destroys them:

```rust
#[test] fn extension_rules() {
    assert_eq!(icon_for("main.rs").0, "rust");
    assert_eq!(icon_for("app.tsx").0, "typescript");
    assert_eq!(icon_for("x.unknownext").0, "default");
}
#[test] fn exact_and_substring_rules() {
    assert_eq!(icon_for("Dockerfile").0, "docker");
    assert_eq!(icon_for("webpack.config.js").0, "webpack");
}
#[test] fn precedence_exact_over_extension() {
    // mapping.less gives specific names priority over bare extensions
    assert_ne!(icon_for("tsconfig.json").0, icon_for("other.json").0);
}
```

- [ ] GREEN via generator output; full suite; commit (assets + script + generated code + license).

### Task 2: AssetSource + sidebar icons (shell)

- [ ] Verify `svg()` tint behavior in vendored gpui source; implement `Assets: AssetSource` over the embedded map; `Application::new().with_assets(Assets)` in main.
- [ ] Sidebar rows: tinted `svg()` 14px, folder/open-folder for dirs; Seti color → theme tint tables (light/dark).
- [ ] Manual verify both themes; commit.

### Task 3: New grammars (TDD smoke tests)

- [ ] Check latest crate versions on the index; add compatible ones (yaml, toml-ng, ruby, java, php, cpp, swift, kotlin candidates); RED: one `code_spans_highlight_<lang>` smoke test per added grammar; GREEN: register in `Languages::new`, extend `language_for_path` + aliases. Drop incompatible crates, note in commit.
- [ ] Full suite; commit.

### Task 4: Image viewer tab

- [ ] RED: `is_image_path` test (png/jpg/jpeg/gif/webp/svg/bmp/ico, case-insensitive; negative for .md/.rs).
- [ ] GREEN + shell: `Tab::Image { path, title }`; `open_path` routes images first; render centered `img` in reading column; tab title/path helpers; outline returns None.
- [ ] Manual verify: click banner.png in sidebar → image tab. Commit.

### Task 5: Docs + finish

- [ ] WELCOME.md roadmap row; suite/build clean; commit; finishing-a-development-branch.

## Self-review

Coverage: spec items 1:1 with tasks; generated-code-vs-tests separation handled (tests in non-generated file); fallback documented for svg tint and incompatible grammars. Types: `SetiColor`/`icon_for` (T1↔T2), `Tab::Image` (T4). No placeholders. ✓
