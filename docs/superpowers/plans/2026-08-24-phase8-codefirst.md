# Phase 8: Code-First Editing Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:executing-plans.

**Spec:** `docs/superpowers/specs/2026-08-24-phase8-codefirst-design.md`

### Task 1: Finder fixes (shell)
- [ ] Result rows `.w_full()` so selection bg spans the pane.
- [ ] `UniformListScrollHandle` on Finder (check exact gpui 0.2.2 API: `uniform_list(...).track_scroll(...)`, `handle.scroll_to_item(ix, strategy)`); call on up/down/refilter.
- [ ] Manual verify; commit.

### Task 2: inkjet migration (guarded by existing smoke tests)
- [ ] `cargo add inkjet` with `all_languages` (or every `language-*` feature); remove the 30 tree-sitter grammar crates, tree-sitter-highlight, tree-sitter-language; delete assets/queries.
- [ ] Rewrite `highlight.rs`: CAPTURE_NAMES = inkjet's HIGHLIGHT_NAMES; `highlight(lang, src)` via `inkjet::Language::from_token(lang)` + raw events → spans. Keep alias fallback.
- [ ] `capture_color`/`syntax_color` prefix maps extended for Helix names (`keyword.control`, `function.method`, `type.builtin`, `variable.other.member`, `string.special`, `comment.line/block`, `constant.numeric`, `markup.*` ignored).
- [ ] RED expectation: existing smoke tests run against the new engine; fix snippets/mappings until green. Add smoke cases: dockerfile, julia, perl. Record binary/compile deltas in the commit.
- [ ] Full suite; commit.

### Task 3: Code mode layout + gutter
- [ ] TDD `gutter_cols(line_count)` (1→1, 10→2, 9999→4) in editor (pure fn).
- [ ] `is_code_mode()`; list closure: code mode rows = full-width container (px 16 pad, no max_w/centering) with `[gutter | LineElement]`; gutter shows `line_ix + 1`, mono `code_size - 1`, `fg_muted` at 0.7 alpha, right-aligned, width `gutter_cols * ~8px + 12`. Markdown path byte-identical to today.
- [ ] Manual verify on main.rs + WELCOME.md side by side; commit.

### Task 4: Auto-indent (TDD)
- [ ] RED in core.rs:
```rust
#[test]
fn newline_copies_leading_whitespace() {
    let mut ed = EditorCore::new("    let x = 1;");
    ed.set_cursor(14);
    ed.insert_newline_auto_indent(t0());
    assert_eq!(ed.buffer.text(), "    let x = 1;\n    ");
}
#[test]
fn newline_mid_line_indents_and_carries_tail() {
    let mut ed = EditorCore::new("\tfoo(bar)");
    ed.set_cursor(5); // after "foo("... within line
    ed.insert_newline_auto_indent(t0());
    assert_eq!(ed.buffer.text(), "\tfoo\n\t(bar)");
}
#[test]
fn newline_no_indent_is_plain() {
    let mut ed = EditorCore::new("plain");
    ed.set_cursor(5);
    ed.insert_newline_auto_indent(t0());
    assert_eq!(ed.buffer.text(), "plain\n");
}
```
- [ ] GREEN: leading whitespace of the cursor's line, `insert("\n" + indent)`.
- [ ] Editor Newline action: code mode → auto-indent variant; markdown unchanged. Manual verify; commit.

### Task 5: Docs, finish, push
- [ ] README language count updated (measure actual enabled inkjet languages); WELCOME roadmap row; finishing-a-development-branch; push.

## Self-review
Spec↔tasks 1:1; capture-name remap risk held by smoke tests; gutter outside shaped text keeps offset math intact (stated in T3); auto-indent markdown exclusion stated (T4). ✓
