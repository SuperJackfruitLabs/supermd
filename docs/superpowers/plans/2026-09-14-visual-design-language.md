# 0.0.17 Visual Design Language Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give SuperMD its own visual identity — the document is a page resting on a continuous ground — and clear the filed issues that live in the same code.

**Architecture:** Four surface roles (ground, page, floating, modal) replace today's single flat plane. Value stepping does the separating that hairlines do now; shadows are reserved for the page and for things that genuinely float. Two new theme colours, both derived when a theme omits them, so every existing theme keeps loading. Shadow geometry and radius live in code, not TOML.

**Tech Stack:** Rust, GPUI 0.2.2 (vendored at `vendor/gpui`), ropey, pulldown-cmark 0.13.4, toml/serde, `ignore` crate.

**Spec:** `docs/superpowers/specs/2026-09-14-visual-design-language-design.md`

## Global Constraints

- **Editing logic is pure Rust under test; the GPUI shell stays thin.** New logic goes in a pure module with the shell driving it.
- **Byte offsets into the ropey rope** are the universal currency for every position, span and selection.
- **`src/editor/display.rs` is the only place** the "buffer offset == rendered offset" invariant may break.
- **CI enforces a 90% line coverage floor.** Run `bash scripts/build_plugins.sh --fixtures` before `cargo llvm-cov`, or `extensions.rs` reads ~47% and the total looks like a failure.
- **Both suites must pass:** `cargo test` and `cargo test --no-default-features --features mas`.
- **`cargo build --bin supermd` must succeed.** It is NOT covered by the suites — a test-only GPUI API (`WindowHandle::root`, gated `#[cfg(any(test, feature = "test-support"))]`) reached production code in 0.0.16, compiled under `cargo test`, and broke `cargo build`. Run it explicitly.
- **Tests must compile on macOS, Linux and Windows.** Anything using `std::os::unix` needs `#[cfg(unix)]`.
- **Any test touching `HOME` must go through `workspace::tests::temp_home()`**, which holds `HOME_LOCK`. A bare `std::env::set_var("HOME", …)` races other tests and passes only in isolation.
- **A new `KeyBinding` is declared in `commands.rs`**, never by hand in `main.rs`. Then `UPDATE_DOCS=1 cargo test shortcut_docs_match_the_table` and `cargo run --example build_docs`.
- **New theme colours must be threaded through `Theme::map_colors`** or flux warming misses them. Task 1 makes this structural.
- **SuperMD never writes to the user's git repository.**
- **The plain-text Markdown file is the source of truth.**
- **Existing user themes must keep loading unedited.** `ThemeFileColors` (`src/theme.rs:224`) has all-required `String` fields and no serde defaults, so every new key is `Option<String>`.
- **Known flake, do not chase:** `editor::tests::scrollbar_drag_scrubs_through_the_document` fails occasionally under heavy CPU load, passes in isolation.
- **`tests/fixtures/plugins` is gitignored.** Without it the sandbox tests SKIP silently while still counting as passes — run `grep -c SKIP` on test output.
- **Disk is tight on the dev machine.** `df -h /` around builds; below ~1.5Gi run `rm -rf target/debug/incremental`. Never a bare `cargo clean`.

## File Structure

**Created:**
- `src/elevation.rs` — pure: the four surface roles, their shadow geometry, the radius scale. No GPUI element construction, only values the shell applies.

**Modified:**
- `src/theme.rs` — colour-field macro, three new tokens, derivation for themes that omit them, contrast helpers.
- `src/workspace.rs` — root layout (page inset), tab strip, sidebar, outline, status bar, `seti_tint`, table render, overlay shadows, right-click on widget tables, folder-picker shortfall.
- `src/view.rs` — reading-view page surface, table borders, `rule`.
- `src/editor/mod.rs` — editor page background, `render_table`, delete-row caret, refusal feedback.
- `src/editor/display.rs` — thematic-break rendering.
- `src/editor/spans.rs` — frontmatter span kind.
- `src/markdown.rs` — frontmatter block, HTML block passthrough.
- `src/reader.rs` — outline excludes frontmatter; reading-view checkbox toggling.
- `src/files.rs` — sidebar/index predicate split.
- `src/preview.rs`, `src/finder.rs`, `src/install_ui.rs` — overlay shadows re-pointed.
- `assets/themes/*.toml` (8 files) — hand-tuned for the page surface.

---

### Task 1: Declare theme colours once

**Files:**
- Modify: `src/theme.rs:20-101` (the `Theme` struct and `map_colors`)
- Test: inline in `src/theme.rs`

**Interfaces:**
- Produces: `theme_colors!` macro generating the `Theme` colour fields and `Theme::map_colors`. No public signature changes — `map_colors(&self, f: impl Fn(Hsla) -> Hsla) -> Self` stays.

`Theme::map_colors` (`src/theme.rs:62`) enumerates all 42 fields by hand. CLAUDE.md warns that a colour missing from it is skipped by flux warming. This is the same hand-maintained-second-list defect `Surface::ALL` had before `menus.rs`'s `surfaces!` macro removed it. Later tasks add colours; make forgetting one a compile error first.

- [ ] **Step 1: Write the failing test**

```rust
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
```

- [ ] **Step 2: Run it and watch it fail**

```sh
cargo test --bin supermd map_colors_touches_every_colour_field
```
Expected: FAIL — `no method named color_fields`.

- [ ] **Step 3: Add the macro**

Declare the colour fields once. The macro generates the struct fields, `map_colors`, and a `color_fields()` accessor used only by the test.

```rust
macro_rules! theme_colors {
    ($($field:ident),+ $(,)?) => {
        /// Colour fields of the theme. Declared once so `map_colors`
        /// cannot miss one -- flux warming runs through it, and a
        /// colour it skips silently stops adapting to time of day.
        #[derive(Debug, Clone, Copy, PartialEq)]
        pub struct ThemeColors {
            $(pub $field: Hsla,)+
        }

        impl ThemeColors {
            fn map(&self, f: &impl Fn(Hsla) -> Hsla) -> Self {
                Self { $($field: f(self.$field),)+ }
            }

            pub fn fields(&self) -> Vec<(&'static str, Hsla)> {
                vec![$((stringify!($field), self.$field),)+]
            }
        }
    };
}

theme_colors!(
    bg, fg, fg_strong, fg_muted, accent, link, code_bg, code_fg, border,
    panel_bg, hover_bg, selected_bg, find_match_bg, find_active_bg,
    diff_added_bg, diff_added_fg, diff_deleted_bg, diff_deleted_fg,
);
```

Keep `Theme`'s existing field names reachable. Add `Deref`-style accessors or flatten `ThemeColors` into `Theme` — whichever keeps every existing `t.bg` / `t.fg_muted` call site compiling unchanged. **Do not rename any existing field**: `t.bg`, `t.fg`, `t.accent` and the rest appear across `workspace.rs`, `view.rs` and `editor/`, and renaming them is out of scope for this task.

Rewrite `map_colors` to delegate:

```rust
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

    pub fn color_fields(&self) -> Vec<(&'static str, Hsla)> {
        let mut v = self.colors.fields();
        v.extend(self.syntax.fields());
        v
    }
```

Apply the same treatment to `SyntaxColors` (`src/theme.rs:6-17`) so syntax colours are covered by the same guarantee.

- [ ] **Step 4: Run the test and the suites**

```sh
cargo test --bin supermd map_colors_touches_every_colour_field
cargo test --bin supermd
cargo build --bin supermd
```
Expected: PASS, suite green at its current count, build clean.

- [ ] **Step 5: Prove the guard is real**

Temporarily delete one field from the `theme_colors!` invocation. Expected: **compile error** at every call site using it, not a silent pass. Restore it.

- [ ] **Step 6: Commit**

```bash
git add src/theme.rs
git commit -m "refactor: declare theme colours once so flux cannot miss one

map_colors enumerated 42 fields by hand, and CLAUDE.md warns that a
colour missing from it stops adapting to time of day. Same shape as the
Surface::ALL list the surfaces! macro removed.

Declared once via theme_colors!, which generates the fields, the map,
and the accessor the test walks. Forgetting a colour is now a compile
error rather than something a reviewer has to notice."
```

---

### Task 2: The three new tokens, derived when absent

**Files:**
- Modify: `src/theme.rs` (`theme_colors!` invocation, `ThemeFileColors:224`, `from_toml:283`, `light():104`, `dark():152`)
- Test: inline in `src/theme.rs`

**Interfaces:**
- Consumes: `theme_colors!` from Task 1.
- Produces: `t.page_bg`, `t.border_subtle`, `t.shadow` on `Theme`; `Theme::derive_page_bg(bg: Hsla, is_dark: bool) -> Hsla`; `Theme::contrast(a: Hsla, b: Hsla) -> f32` (WCAG relative-luminance ratio, ≥1.0).

`page_bg` is the document surface, brighter than the ground. `border_subtle` is the lighter hairline for the few places a border survives. `shadow` is the single shadow colour; its *geometry* is Task 3 and lives in code.

- [ ] **Step 1: Write the failing tests**

```rust
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
    /// jarring jump -- the convention is one adjacent step.
    #[test]
    fn page_and_ground_are_one_step_apart() {
        for t in [Theme::light(), Theme::dark()] {
            let delta = (t.page_bg.l - t.bg.l).abs();
            assert!(
                (0.012..=0.075).contains(&delta),
                "page/ground delta {delta} is not one adjacent step"
            );
        }
    }

    /// Body text has to stay readable on the new surface, in every
    /// theme we ship -- a derived value that looks wrong in nord is
    /// caught here rather than by squinting at a screenshot.
    #[test]
    fn every_shipped_theme_keeps_text_readable_on_the_page() {
        for (name, theme) in shipped_themes() {
            let body = Theme::contrast(theme.fg, theme.page_bg);
            assert!(body >= 4.5, "{name}: body text on page is {body:.2}:1");
            let muted = Theme::contrast(theme.fg_muted, theme.bg);
            assert!(muted >= 3.0, "{name}: muted text on ground is {muted:.2}:1");
            let surfaces = Theme::contrast(theme.page_bg, theme.bg);
            assert!(
                surfaces >= 1.03,
                "{name}: page and ground are indistinguishable ({surfaces:.3}:1)"
            );
        }
    }
```

`shipped_themes()` is a test helper that loads every `assets/themes/*.toml` through `LoadedTheme::from_toml` plus `Theme::light()` and `Theme::dark()`, returning `Vec<(String, Theme)>`. Write it in the same test module using `include_str!` for each file so the test has no filesystem dependency:

```rust
    fn shipped_themes() -> Vec<(String, Theme)> {
        let mut v = vec![
            ("built-in light".to_string(), Theme::light()),
            ("built-in dark".to_string(), Theme::dark()),
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
            v.push((name.to_string(), LoadedTheme::from_toml(src).expect(name)));
        }
        v
    }
```

**Verify the file list against `ls assets/themes/` before writing it** — if a theme has been added or renamed, fix the list rather than the test.

- [ ] **Step 2: Run and watch them fail**

```sh
cargo test --bin supermd page_bg_ every_shipped_theme_keeps_text
```
Expected: FAIL — `no field page_bg`.

- [ ] **Step 3: Add the tokens and the derivation**

Add `page_bg, border_subtle, shadow` to the `theme_colors!` invocation. Add the three keys to `ThemeFileColors` as **optional**:

```rust
struct ThemeFileColors {
    // ... existing required fields unchanged ...
    #[serde(default)]
    page_bg: Option<String>,
    #[serde(default)]
    border_subtle: Option<String>,
    #[serde(default)]
    shadow: Option<String>,
}
```

In `from_toml`, after `theme.bg` and `theme.border` are set:

```rust
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
```

```rust
    /// The page is one adjacent step from the ground: brighter in
    /// light, and still brighter in dark -- a page is paper, and paper
    /// catches the light in both.
    pub fn derive_page_bg(bg: Hsla, is_dark: bool) -> Hsla {
        let step = if is_dark { 0.035 } else { 0.030 };
        Hsla { l: (bg.l + step).clamp(0., 1.), ..bg }
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
```

Set explicit values in `Theme::light()` and `Theme::dark()` rather than relying on derivation for the built-ins.

Add the contrast helper:

```rust
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
```

- [ ] **Step 4: Run the tests**

```sh
cargo test --bin supermd page_bg_ page_and_ground every_shipped_theme_keeps_text
cargo test --bin supermd
cargo build --bin supermd
```
Expected: PASS. **If a shipped theme fails the contrast floor, do not weaken the threshold** — record which theme and carry it into Task 8, which hand-tunes them. Mark such a theme with `#[ignore]`-style exclusion only if it blocks; prefer fixing the derivation constants.

- [ ] **Step 5: Prove an old theme still loads**

```rust
    /// A theme written before these tokens existed loads unchanged.
    #[test]
    fn a_theme_without_the_new_keys_still_loads() {
        let src = include_str!("../assets/themes/nord.toml");
        assert!(!src.contains("page_bg"), "fixture assumption: nord predates page_bg");
        let t = LoadedTheme::from_toml(src).expect("nord loads");
        assert!(t.page_bg.l > 0., "derived rather than defaulted to nothing");
    }
```

- [ ] **Step 6: Commit**

```bash
git add src/theme.rs
git commit -m "feat: page, subtle border and shadow tokens, derived when absent

Three new colours. Every key is optional in the theme file --
ThemeFileColors has no serde defaults and all-required fields, so a
required key would have broken every existing theme.

A theme that omits them gets a page one adjacent step from its ground,
a hairline at reduced alpha, and a shadow warm-shifted to match a warm
ground. The contrast test walks all eight shipped themes and asserts
body text stays readable on the new surface."
```

---

### Task 3: The elevation module

**Files:**
- Create: `src/elevation.rs`
- Modify: `src/main.rs` (add `mod elevation;`)
- Test: inline in `src/elevation.rs`

**Interfaces:**
- Consumes: `t.shadow` from Task 2.
- Produces:
  - `pub enum Surface { Ground, Page, Floating, Modal }`
  - `pub fn shadows(surface: Surface, shadow: Hsla) -> Vec<gpui::BoxShadow>`
  - `pub fn radius(surface: Surface) -> gpui::Pixels`
  - `pub fn inner_radius(outer: gpui::Pixels, padding: gpui::Pixels) -> gpui::Pixels`

Geometry lives here, not in TOML. Zed does the same: its `ElevationIndex` gives persistent surfaces zero shadow, popovers a two-layer shadow at 2–3px blur and 3–12% opacity, modals four layers up to 12px.

- [ ] **Step 1: Write the failing tests**

```rust
    /// The ground never lifts. A shadow on a static panel tells the
    /// user it can be picked up, and the sidebar cannot.
    #[test]
    fn only_the_page_and_floating_things_cast_shadows() {
        let s = Hsla { h: 0., s: 0., l: 0., a: 0.3 };
        assert!(shadows(Surface::Ground, s).is_empty(), "ground must not lift");
        assert!(!shadows(Surface::Page, s).is_empty(), "the page rests on the ground");
        assert!(!shadows(Surface::Floating, s).is_empty());
        assert!(!shadows(Surface::Modal, s).is_empty());
    }

    /// Depth increases with the tier: a dialog reads as further from
    /// the page than a popover does.
    #[test]
    fn depth_increases_with_the_tier() {
        let s = Hsla { h: 0., s: 0., l: 0., a: 0.3 };
        let blur = |sf| {
            shadows(sf, s).iter().map(|b| f32::from(b.blur_radius)).fold(0., f32::max)
        };
        assert!(blur(Surface::Page) < blur(Surface::Floating));
        assert!(blur(Surface::Floating) < blur(Surface::Modal));
        assert!(shadows(Surface::Modal, s).len() >= shadows(Surface::Page, s).len());
    }

    /// Apple's concentric rule: a rounded thing inside a rounded thing
    /// shares its centre of curvature. inner = outer - padding.
    #[test]
    fn inner_radius_is_concentric_and_never_negative() {
        assert_eq!(inner_radius(px(12.), px(4.)), px(8.));
        assert_eq!(inner_radius(px(4.), px(9.)), px(0.), "clamped, not negative");
    }
```

- [ ] **Step 2: Run and watch them fail**

```sh
cargo test --bin supermd elevation::
```
Expected: FAIL — module does not exist.

- [ ] **Step 3: Write the module**

```rust
//! Surface roles and their depth. Geometry lives here rather than in a
//! theme file: exposing twelve numbers to theme authors invites themes
//! that look broken, and shadow geometry is design, not palette.

use gpui::{px, BoxShadow, Hsla, Pixels};

/// Where a surface sits. Every pixel in the app belongs to exactly one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    /// The window itself -- sidebar, tab strip, outline, status bar.
    /// Never lifts.
    Ground,
    /// The document, and only the document. The one thing that rests
    /// on the ground at rest.
    Page,
    /// Hover previews, the finder, the palette, context menus.
    Floating,
    /// Dialogs, the install flow, confirmations.
    Modal,
}

pub fn shadows(surface: Surface, shadow: Hsla) -> Vec<BoxShadow> {
    let layer = |y: f32, blur: f32, alpha: f32| BoxShadow {
        color: Hsla { a: shadow.a * alpha, ..shadow },
        offset: gpui::point(px(0.), px(y)),
        blur_radius: px(blur),
        spread_radius: px(0.),
    };
    match surface {
        Surface::Ground => vec![],
        Surface::Page => vec![layer(1., 3., 0.9), layer(6., 14., 0.6)],
        Surface::Floating => vec![layer(1., 3., 1.0), layer(4., 12., 0.8)],
        Surface::Modal => vec![
            layer(1., 2., 1.0),
            layer(4., 10., 0.9),
            layer(10., 24., 0.7),
            layer(18., 44., 0.5),
        ],
    }
}

pub fn radius(surface: Surface) -> Pixels {
    match surface {
        Surface::Ground => px(0.),
        Surface::Page => px(8.),
        Surface::Floating => px(8.),
        Surface::Modal => px(12.),
    }
}

/// Apple's concentric rule: a rounded rect inside another shares its
/// centre of curvature, so the inner radius is the outer minus the gap
/// between them. Clamped at zero -- a negative radius is a square.
pub fn inner_radius(outer: Pixels, padding: Pixels) -> Pixels {
    let v = f32::from(outer) - f32::from(padding);
    px(v.max(0.))
}
```

- [ ] **Step 4: Run the tests and both suites**

```sh
cargo test --bin supermd elevation::
cargo test --bin supermd && cargo test --bin supermd --no-default-features --features mas
cargo build --bin supermd
```

- [ ] **Step 5: Commit**

```bash
git add src/elevation.rs src/main.rs
git commit -m "feat: surface roles and their depth

Four roles -- ground, page, floating, modal -- and the shadow geometry
each gets. The ground never lifts: a shadow on a static panel says it
can be picked up, and a sidebar cannot.

Geometry is code rather than theme keys, the way Zed keeps it. Also
Apple's concentric rule for nested radii, which is a relationship
rather than a constant: inner = outer - padding."
```

---

### Task 4: The document becomes a page

**Files:**
- Modify: `src/workspace.rs:5435-5480` (root layout; the `content` match and the wrapping `div`)
- Modify: `src/editor/mod.rs` (`impl Render for Editor` — its root background)
- Modify: `src/reader.rs:312-320` (`impl Render for Reader` — its root background)
- Test: inline in `src/workspace.rs`

**Interfaces:**
- Consumes: `elevation::{Surface, shadows, radius}` (Task 3), `t.page_bg` (Task 2).
- Produces: `Workspace::page_inset() -> gpui::Pixels` — the gap between the page and the window edge, so later tasks and the projectors agree on one number.

This is the heaviest task in the release. The editor virtualizes one list item per logical line and the projectors compute widget widths against the available measure, so an inset is not a paint-only change.

- [ ] **Step 1: Write the failing test**

```rust
    /// The document is the one thing that rests on the ground. It gets
    /// the page surface; the window keeps the ground.
    #[gpui::test]
    fn the_document_pane_is_a_page_not_the_window_background(cx: &mut TestAppContext) {
        let _home = temp_home();
        let fx = tempfile::tempdir().unwrap();
        std::fs::write(fx.path().join("n.md"), "# Note\n").unwrap();
        let (ws, cx) = open_workspace(cx, fx.path());
        cx.run_until_parked();
        cx.update(|_, app| {
            let t = crate::theme::theme_for(app);
            assert_ne!(
                t.page_bg, t.bg,
                "the page must be its own surface, not the window background"
            );
            assert!(
                f32::from(ws.read(app).page_inset()) > 0.,
                "the page needs a margin, or it cannot read as resting on anything"
            );
        });
    }
```

Check the real helper names before writing this: `open_workspace` and `temp_home` exist in `workspace::tests`; the theme accessor may be `theme(cx)` rather than `theme_for(app)` — grep `src/workspace.rs` for how existing tests read the theme and match it.

- [ ] **Step 2: Run and watch it fail**

```sh
cargo test --bin supermd the_document_pane_is_a_page
```
Expected: FAIL — `no method named page_inset`.

- [ ] **Step 3: Wrap the content in a page**

Add the inset accessor and wrap the matched `content` element:

```rust
    /// The gap between the page and the window edge. One number, so
    /// the projectors and the layout agree on the measure.
    pub(crate) fn page_inset(&self) -> gpui::Pixels {
        px(10.)
    }
```

In `render`, after the `content` match (`src/workspace.rs:5439-5468`), wrap it:

```rust
        let inset = self.page_inset();
        let page = div()
            .flex_1()
            .min_w_0()
            .my(inset)
            .mr(inset)
            .bg(t.page_bg)
            .rounded(crate::elevation::radius(crate::elevation::Surface::Page))
            .shadow(crate::elevation::shadows(
                crate::elevation::Surface::Page,
                t.shadow,
            ))
            .overflow_hidden()
            .child(content);
```

and use `page` where `content` was used before.

Change the editor's and reader's own root backgrounds from `t.bg` to `t.page_bg` so the surface is continuous through the pane (`src/reader.rs:318`, and the corresponding `bg(...)` in `impl Render for Editor`). The image-viewer arms at `src/workspace.rs:5450` and `:5459` keep `t.bg` — an image is not a document, and a bright page behind a transparent PNG would be wrong.

**Watch the measure.** After this change run the editor's existing layout tests; if wrapped-line geometry or a projector widget width regressed, fix it here rather than deferring — that is the risk this task exists to absorb.

- [ ] **Step 4: Run everything**

```sh
cargo test --bin supermd the_document_pane_is_a_page
cargo test --bin supermd
cargo test --bin supermd --no-default-features --features mas
cargo build --bin supermd
```

- [ ] **Step 5: Look at it**

```sh
cargo run -- examples/vault
```
The document should read as a sheet on a ground. If the inset looks wrong at this stage, say so in the report rather than tuning it silently — Task 8 tunes values with the whole picture visible.

- [ ] **Step 6: Commit**

```bash
git add src/workspace.rs src/editor/mod.rs src/reader.rs
git commit -m "feat: the document is a page resting on the ground

The editor's background stopped being the window's. The document pane
takes page_bg, a margin, a radius and the resting shadow; the window
keeps the ground.

The inset changes the available measure -- the editor virtualizes one
item per logical line and the projectors size widgets against it -- so
this is not a paint-only change. Image tabs keep the ground: an image
is not a document."
```

---

### Task 5: The active tab belongs to the page

**Files:**
- Modify: `src/workspace.rs` (tab rendering, near `seti_tint` use at `:4229`)
- Test: inline in `src/workspace.rs`

**Interfaces:**
- Consumes: `t.page_bg` (Task 2), `Workspace::page_inset()` (Task 4).

Tabs sit on the ground. The active tab takes `page_bg` and meets the page with no seam, so it reads as attached to the document. This single move does most of the work of making the page look like a sheet.

**Ruling (recorded during Task 4).** The page is inset on **left, right and bottom only** — the top edge is deliberately flush, because that is where the active tab joins it. Tab and page together form the sheet. An inset on all four sides would leave a gap the tab cannot cross, which contradicts this task's own requirement; an inset on neither left nor top left the page butting flush against the sidebar, reading as not resting on anything. Left/right/bottom plus a flush top resolves both.

- [ ] **Step 1: Write the failing test**

```rust
    /// The active tab is part of the page, not part of the chrome.
    /// Inactive tabs stay on the ground.
    #[gpui::test]
    fn the_active_tab_takes_the_page_surface(cx: &mut TestAppContext) {
        let _home = temp_home();
        let fx = tempfile::tempdir().unwrap();
        std::fs::write(fx.path().join("a.md"), "# A\n").unwrap();
        std::fs::write(fx.path().join("b.md"), "# B\n").unwrap();
        let (ws, cx) = open_workspace(cx, fx.path());
        cx.run_until_parked();
        cx.update(|_, app| {
            let t = crate::theme::theme_for(app);
            let w = ws.read(app);
            assert_eq!(w.tab_background(0, &t), t.page_bg, "active tab is the page");
            assert_eq!(w.tab_background(1, &t), t.bg, "inactive tabs are ground");
        });
    }
```

- [ ] **Step 2: Run and watch it fail**

```sh
cargo test --bin supermd the_active_tab_takes_the_page_surface
```
Expected: FAIL — `no method named tab_background`.

- [ ] **Step 3: Extract the rule, then use it**

The rule is pure, so it is testable without driving a render:

```rust
    /// Which surface a tab sits on. The active tab is the page's own
    /// edge; every other tab is chrome.
    pub(crate) fn tab_background(&self, ix: usize, t: &Theme) -> gpui::Hsla {
        if ix == self.active { t.page_bg } else { t.bg }
    }
```

Apply it at the tab render site, give the active tab a top radius matching `elevation::radius(Surface::Page)` with square bottom corners, and remove any bottom border on the active tab so it meets the page cleanly.

- [ ] **Step 4: Run the tests**

```sh
cargo test --bin supermd tab_ && cargo test --bin supermd && cargo build --bin supermd
```

- [ ] **Step 5: Commit**

```bash
git add src/workspace.rs
git commit -m "feat: the active tab is the page's own edge

Tabs sit on the ground; the active one takes page_bg and meets the
document with no seam. The rule is a pure function so it is tested
without driving a render."
```

---

### Task 6: One continuous ground

**Files:**
- Modify: `src/workspace.rs` — `render_sidebar:3724`, `render_outline:5278`, `render_status_bar:4071`, `render_titlebar:4218`
- Test: inline in `src/workspace.rs`

Sidebar, tab strip, outline and status bar share one background with no dividers between them. The hairlines separating them today are what make the app read as panes rather than a ground.

- [ ] **Step 1: Write the failing test**

```rust
    /// The chrome is one surface. Dividers between its parts are what
    /// made the app read as three panes at the same value.
    #[gpui::test]
    fn the_chrome_is_one_continuous_ground(cx: &mut TestAppContext) {
        let _home = temp_home();
        let fx = tempfile::tempdir().unwrap();
        std::fs::write(fx.path().join("n.md"), "# N\n").unwrap();
        let (ws, cx) = open_workspace(cx, fx.path());
        cx.run_until_parked();
        cx.update(|_, app| {
            let t = crate::theme::theme_for(app);
            let w = ws.read(app);
            for part in [ChromePart::Sidebar, ChromePart::Outline, ChromePart::StatusBar] {
                assert_eq!(
                    w.chrome_background(part, &t),
                    t.bg,
                    "{part:?} must share the ground"
                );
            }
            assert!(
                !w.chrome_has_divider(ChromePart::Sidebar),
                "the ground is continuous; no divider inside it"
            );
        });
    }
```

- [ ] **Step 2: Run and watch it fail**

```sh
cargo test --bin supermd the_chrome_is_one_continuous_ground
```
Expected: FAIL — `ChromePart` does not exist.

- [ ] **Step 3: Name the parts and unify them**

```rust
/// The parts of the chrome. They all share the ground; naming them
/// keeps "which surface is this?" answerable in a test rather than by
/// reading four render functions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChromePart {
    Sidebar,
    Outline,
    StatusBar,
    TitleBar,
}

impl Workspace {
    pub(crate) fn chrome_background(&self, _part: ChromePart, t: &Theme) -> gpui::Hsla {
        t.bg
    }

    /// Nothing inside the ground draws a divider. The page's own edge
    /// is what separates chrome from document.
    pub(crate) fn chrome_has_divider(&self, _part: ChromePart) -> bool {
        false
    }
}
```

Then in each render function, replace `t.panel_bg` / `t.bg` backgrounds with `self.chrome_background(part, &t)` and **delete the borders between chrome parts** — the sidebar's right border, the outline's left border, the status bar's top border. Keep `panel_bg` as a token (themes still set it, and hover/selected states derive from it); it simply stops being the sidebar's background.

- [ ] **Step 4: Sidebar row states**

With no dividers and one ground, the active row is the only thing marking where you are. Write the test first:

```rust
    /// The open file is marked by an accent bar on the leading edge,
    /// not by a background alone -- on a continuous ground a tinted
    /// row alone is easy to miss.
    #[test]
    fn the_active_sidebar_row_carries_an_accent_bar() {
        let t = Theme::light();
        let active = sidebar_row_style(RowState::Active, &t);
        assert_eq!(active.leading_bar, Some(t.accent));
        assert_ne!(active.background, t.bg, "and a tint behind it");

        let hovered = sidebar_row_style(RowState::Hovered, &t);
        assert_eq!(hovered.leading_bar, None, "hover is a value step, not a mark");
        assert_ne!(hovered.background, t.bg);

        let resting = sidebar_row_style(RowState::Resting, &t);
        assert_eq!(resting.background, t.bg, "a resting row is the ground");
        assert_eq!(resting.leading_bar, None);
    }
```

```rust
/// How a sidebar row paints. Pure, so "which row is the open one?" is
/// answerable in a test rather than by reading a render function.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RowState {
    Resting,
    Hovered,
    Active,
}

pub(crate) struct RowStyle {
    pub background: gpui::Hsla,
    pub leading_bar: Option<gpui::Hsla>,
}

pub(crate) fn sidebar_row_style(state: RowState, t: &Theme) -> RowStyle {
    match state {
        RowState::Resting => RowStyle { background: t.bg, leading_bar: None },
        RowState::Hovered => RowStyle { background: t.hover_bg, leading_bar: None },
        RowState::Active => RowStyle { background: t.selected_bg, leading_bar: Some(t.accent) },
    }
}
```

Apply it at the sidebar row render site. This is separate from `sidebar_row_color` (`workspace.rs:478`), which decides *text* colour and carries the gitignored dimming — leave that function alone; it is already tested and #36 depends on it.

- [ ] **Step 5: Group the toolbar**

The four glyphs float in the top-right with no grouping. Apple's guidance is at most three logical groups. Give them consistent hit targets and group them by what they do — view toggles together, document actions together — separated by spacing rather than dividers. They stay on the ground: a container would imply elevation, and nothing in the chrome lifts.

No test asserts visual grouping; assert the grouping *rule* instead if you introduce one (e.g. a function returning which group a command belongs to), and otherwise verify by eye in Step 6.

- [ ] **Step 6: Run and look**

```sh
cargo test --bin supermd chrome_ && cargo test --bin supermd && cargo build --bin supermd
cargo run -- examples/vault
```

- [ ] **Step 7: Commit**

```bash
git add src/workspace.rs
git commit -m "feat: sidebar, outline and status bar are one ground

They shared a value and were separated by hairlines, which is what made
the app read as three panes rather than a document on a desk. Now they
share the background and the dividers are gone; the page's own edge
does the separating.

With no dividers the open file needs marking, so the active row gains
an accent bar on its leading edge and the row states become a pure
function. The toolbar glyphs are grouped by what they do, spaced rather
than boxed -- a container would imply elevation, and nothing in the
chrome lifts."
```

---

### Task 7: One temperature

**Files:**
- Modify: `src/theme.rs` (`light():104`, `dark():152` — ink and ground values)
- Modify: `src/workspace.rs:488` (`seti_tint`) — add the muted chrome variant
- Test: inline in both

**Interfaces:**
- Produces: `seti_tint_muted(color: SetiColor, t: &Theme, active: bool) -> gpui::Hsla`

Icons are already themed: `seti_tint` maps Seti's palette onto the theme, and blue resolves to `syntax.function`. The clash is that **chrome is coloured with a code-syntax palette** — saturated and cool because it is tuned for code legibility. The syntax palette is not touched.

- [ ] **Step 1: Write the failing tests**

```rust
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
```

```rust
    /// Chrome icons go quiet so the file list stops competing with the
    /// document; the open file's icon takes the accent, which is how
    /// you can see at a glance which one it is.
    #[test]
    fn chrome_icons_are_muted_and_the_active_one_takes_the_accent() {
        let t = crate::theme::Theme::light();
        assert_eq!(seti_tint_muted(SetiColor::Blue, &t, false), t.fg_muted);
        assert_eq!(seti_tint_muted(SetiColor::Purple, &t, false), t.fg_muted);
        assert_eq!(seti_tint_muted(SetiColor::Blue, &t, true), t.accent);
        // The full-colour mapping survives for the finder, where telling
        // file types apart quickly is the actual task.
        assert_eq!(seti_tint(SetiColor::Blue, &t), t.syntax.function);
    }
```

- [ ] **Step 2: Run and watch them fail**

```sh
cargo test --bin supermd one_temperature chrome_icons_are_muted
```

- [ ] **Step 3: Implement**

```rust
/// Chrome icons, quiet. The sidebar and tab strip use the theme's
/// neutrals rather than its syntax palette: syntax colours are tuned
/// for code legibility -- saturated, cool -- and reading them against
/// warm chrome is what made the file list clash.
pub(crate) fn seti_tint_muted(color: SetiColor, t: &Theme, active: bool) -> gpui::Hsla {
    let _ = color;
    if active { t.accent } else { t.fg_muted }
}
```

Use it at the sidebar row (`src/workspace.rs:3661`) and the tab (`:4229`), passing whether that row/tab is the active one. Leave `seti_tint` in place for `search_ui.rs:334` and the finder.

Warm-shift `Theme::light()` and `Theme::dark()`: move `fg`, `fg_strong`, `fg_muted` off neutral onto a low-saturation warm hue near the ground's, and give the dark ground a small warm saturation instead of a blue-grey cast.

- [ ] **Step 4: Run, including the contrast floor from Task 2**

```sh
cargo test --bin supermd one_temperature chrome_icons every_shipped_theme_keeps_text
cargo test --bin supermd && cargo build --bin supermd
```
The contrast test is the guard here: warm-shifting ink must not cost readability.

- [ ] **Step 5: Commit**

```bash
git add src/theme.rs src/workspace.rs
git commit -m "feat: one temperature across ground, ink and chrome icons

Warm ground with neutral ink is what made the cream read as dated. The
ink is warm-shifted to sit near the ground's hue, and the dark ground
gains a little warmth so light and dark read as the same app.

Chrome icons drop the syntax palette. They were already themed --
seti_tint maps Seti's colours onto the theme, so blue was
syntax.function -- but syntax colours are tuned for code legibility and
clash against warm chrome. Sidebar and tabs now use the neutrals, the
active file takes the accent, and the finder keeps full colour where
telling file types apart is the point."
```

---

### Task 8: Hand-tune the eight shipped themes

**Files:**
- Modify: `assets/themes/graphite.toml`, `gruvbox-dark.toml`, `jackfruit-dark.toml`, `jackfruit-light.toml`, `nord.toml`, `paper.toml`, `solarized-dark.toml`, `solarized-light.toml`
- Test: the existing contrast test from Task 2

These themes were designed for a flat world. Derived defaults keep *user* themes loading; they do not spare us from tuning ours. This is craft work — look at each one.

- [ ] **Step 1: Confirm the file list**

```sh
ls assets/themes/
```
If it differs from the eight above, update both this task and `shipped_themes()` in `src/theme.rs`.

- [ ] **Step 2: Set explicit tokens per theme**

For each, add `page_bg`, `border_subtle` and `shadow` under `[colors]` rather than relying on derivation. Judge each by eye against its own palette:

- `nord` and `solarized-*` are cold by design. Do not warm them — they are themes, and a theme is allowed its own temperature. Give them a page that reads correctly in their own terms.
- `gruvbox-dark` has a strong opinion about background value; its derived page may be too close or too far.
- `paper` is the one most likely to need a brighter page, being the warmest.

- [ ] **Step 3: Check the code fence against the page**

`code_bg` was designed to sit on the window background and now sits on `page_bg`, which is brighter. A fence that read correctly on cream can disappear on a page. Extend the contrast test rather than eyeballing it:

```rust
    /// A code fence must stay visible against the page it now sits on.
    /// code_bg was tuned against the window background, which was
    /// darker.
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
```

- [ ] **Step 4: Run the contrast floor after each edit**

```sh
cargo test --bin supermd every_shipped_theme_keeps_text_readable_on_the_page code_fences_stay_visible
```
Never weaken the thresholds to make a theme pass. If a theme cannot meet them, change the theme.

- [ ] **Step 5: Look at all eight**

```sh
cargo run -- examples/vault
```
Cycle themes (⌘K ⌘T or the theme picker) and look at each in both a document and the graph.

- [ ] **Step 6: Commit**

```bash
git add assets/themes
git commit -m "feat: tune the shipped themes for the page surface

Every theme gets an explicit page, subtle border and shadow rather than
the derived defaults, which exist so user themes keep loading, not to
spare us the work.

nord and solarized stay cold on purpose: a theme is allowed its own
temperature, and the defaults are what carry the identity."
```

---

### Task 9: Tables stop being spreadsheets

**Files:**
- Modify: `src/view.rs:451` (`table`) — the reading view
- Modify: `src/editor/mod.rs` (`render_table`, near `:3607`) — the editor widget
- Test: inline in both

Outer border and header rule keep weight; rows separate with a hairline; **interior vertical rules go away**. Notion ships that as the default; Obsidian gives outer, header and interior borders their own tokens.

- [ ] **Step 1: Write the failing test**

```rust
    /// A table is a document element, not a spreadsheet. Vertical
    /// rules between every cell are what made it read as one.
    #[test]
    fn table_borders_are_outer_and_horizontal_only() {
        let t = Theme::light();
        let b = table_borders(&t);
        assert_eq!(b.outer, t.border, "the outer boundary keeps weight");
        assert_eq!(b.header, t.border, "so does the rule under the header");
        assert_eq!(b.row, t.border_subtle, "rows separate with a hairline");
        assert!(b.column.is_none(), "no interior vertical rules");
    }
```

- [ ] **Step 2: Run and watch it fail**

```sh
cargo test --bin supermd table_borders_are_outer_and_horizontal_only
```

- [ ] **Step 3: Extract the rule, apply it in both renderers**

Put the pure rule where both can reach it — `src/view.rs` is shared by the reading path, so define it there and have the editor call it:

```rust
/// Which borders a table draws. Outer and header keep weight, rows get
/// a hairline, and interior verticals are gone -- they are what made a
/// table read as a spreadsheet rather than part of the document.
pub struct TableBorders {
    pub outer: Hsla,
    pub header: Hsla,
    pub row: Hsla,
    pub column: Option<Hsla>,
}

pub fn table_borders(t: &Theme) -> TableBorders {
    TableBorders {
        outer: t.border,
        header: t.border,
        row: t.border_subtle,
        column: None,
    }
}
```

Apply in `view.rs:451`'s `table` and in `editor/mod.rs`'s `render_table`. Keep the header's background step and per-column alignment exactly as they are — alignment is read from the delimiter row's colons (`src/editor/blocks.rs:152-173`) and must not regress.

- [ ] **Step 4: Run**

```sh
cargo test --bin supermd table_ && cargo test --bin supermd && cargo build --bin supermd
```
Run the existing table tests too: alignment, cell navigation and the row-click-to-source behaviour must all still pass.

- [ ] **Step 5: Commit**

```bash
git add src/view.rs src/editor/mod.rs
git commit -m "feat: tables lose their interior vertical rules

Outer border and header rule keep weight, rows separate with a
hairline, verticals are gone. One pure rule shared by the reading view
and the editor widget so they cannot drift.

Column alignment still comes from the delimiter row's colons and is
unchanged."
```

---

### Task 10: One shadow vocabulary

**Files:**
- Modify: `src/finder.rs:371`, `src/install_ui.rs:210`, `src/palette.rs:212`, `src/search_ui.rs:437`, `src/preview.rs:749`
- Modify: `src/workspace.rs:3183, 3332, 3470, 3563, 5862, 5969, 6026`
- Test: inline in `src/elevation.rs`

Twelve call sites each reach for `shadow_lg()` independently. Re-point them at the Floating and Modal tiers so there is one vocabulary.

- [ ] **Step 1: Write the failing test**

```rust
    /// Overlays share one vocabulary. A popover and a dialog are
    /// different depths on purpose, and neither invents its own.
    #[test]
    fn floating_and_modal_are_distinguishable() {
        let s = Hsla { h: 0., s: 0., l: 0., a: 0.3 };
        let f = shadows(Surface::Floating, s);
        let m = shadows(Surface::Modal, s);
        assert_ne!(f.len(), m.len(), "a dialog is not a popover");
        let deepest = |v: &Vec<gpui::BoxShadow>| {
            v.iter().map(|b| f32::from(b.blur_radius)).fold(0., f32::max)
        };
        assert!(deepest(&m) > deepest(&f) * 1.5, "a modal reads as further away");
    }
```

- [ ] **Step 2: Run it**

```sh
cargo test --bin supermd floating_and_modal_are_distinguishable
```

- [ ] **Step 3: Re-point every call site**

Replace each `.shadow_lg()` with the matching tier:

- **Floating** — `finder.rs`, `palette.rs`, `search_ui.rs`, `preview.rs` (the hover popover), and the context menus in `workspace.rs`.
- **Modal** — `install_ui.rs`, and any confirmation dialog among the `workspace.rs` sites.

```rust
.shadow(crate::elevation::shadows(crate::elevation::Surface::Floating, t.shadow))
.rounded(crate::elevation::radius(crate::elevation::Surface::Floating))
```

Decide each `workspace.rs` site by reading what it renders — do not guess from the line number. Record in the report which site got which tier.

- [ ] **Step 4: Verify none were missed**

```sh
grep -rn "shadow_lg()" src/ || echo "no ad-hoc shadows remain"
cargo test --bin supermd && cargo build --bin supermd
```

- [ ] **Step 5: Commit**

```bash
git add src
git commit -m "refactor: overlays share one shadow vocabulary

Twelve call sites each reached for shadow_lg independently. They now
name a tier -- floating for popovers, menus and the finder; modal for
dialogs -- so depth is a decision rather than a default."
```

---

### Task 11: Thematic breaks are drawn, not spelled (#40)

**Files:**
- Modify: `src/editor/display.rs:140-251` (`collect_directives`)
- Test: inline in `src/editor/display.rs`

`spans.rs:105` emits `StyleKind::Rule`, but `collect_directives` has no arm for it and falls to the no-op `_ => {}`, so `---` stays on screen merely faded (`mod.rs:2929`). The reading view already draws a real divider (`view.rs:509`). Hiding the source and drawing a rule, revealed on caret contact, is the rule the editor already applies to heading hashes and list bullets.

- [ ] **Step 1: Write the failing test**

```rust
    /// A thematic break is a line, not three hyphens. The source comes
    /// back the moment the caret touches it, like every other marker.
    #[gpui::test]
    fn a_thematic_break_hides_its_source_until_touched() {
        let src = "before\n\n---\n\nafter\n";
        let spans = crate::editor::spans::markdown_spans(src, &[]);
        let away = display_lines(src, &spans, 0..0);
        let rule_line = away.iter().find(|l| l.source_range.contains(&9)).unwrap();
        assert!(
            !rule_line.text.contains("---"),
            "the hyphens are drawn as a rule, not spelled out: {:?}",
            rule_line.text
        );
        let touching = display_lines(src, &spans, 10..10);
        let revealed = touching.iter().find(|l| l.source_range.contains(&9)).unwrap();
        assert!(
            revealed.text.contains("---"),
            "the caret on the line reveals the source"
        );
    }
```

Match the real helper names: grep `src/editor/display.rs` for how existing tests build display lines (`heading_hashes_hide_when_cursor_off_line` at `:648` is the model) and use the same entry point and struct field names.

- [ ] **Step 2: Run and watch it fail**

```sh
cargo test --bin supermd a_thematic_break_hides_its_source
```
Expected: FAIL — the text still contains `---`.

- [ ] **Step 3: Give `StyleKind::Rule` an arm**

In `collect_directives`, add an arm alongside the heading and list-marker cases:

```rust
        StyleKind::Rule => {
            // The whole line is the marker. Hide it and let the view
            // draw a rule in its place; the reveal rule brings the
            // source back when the caret lands on the line.
            push(Action::Hide(Bias::Whole));
        }
```

If `Bias` has no `Whole` variant, hide the span's full byte range using whichever variant the existing `Hide` arms use for a full-span hide — grep the enum rather than inventing a variant.

Then in `editor/mod.rs`'s line rendering, when a line's only content is a hidden `Rule`, draw a 1px divider in `t.border` at the line's vertical centre instead of an empty line. Reuse `view.rs:509`'s `rule` treatment so the editor and the reading view agree.

- [ ] **Step 4: Run**

```sh
cargo test --bin supermd a_thematic_break display:: && cargo test --bin supermd && cargo build --bin supermd
```
Watch that no existing display test regressed: `Rule` previously produced no directives, so anything asserting on unmodified `---` lines needs updating to the new truth, not the test weakened.

- [ ] **Step 5: Commit**

```bash
git add src/editor/display.rs src/editor/mod.rs
git commit -m "fix: thematic breaks draw as a rule in the editor

spans.rs has always emitted StyleKind::Rule; display.rs had no arm for
it, so --- sat on screen faded while the reading view drew a real
divider. Now it hides and draws, and the caret reveals the source like
every other marker.

Closes #40"
```

---

### Task 12: Frontmatter stops being a giant heading (#38)

**Files:**
- Modify: `src/markdown.rs` (new `frontmatter_range`, parse, `Block`)
- Modify: `src/editor/spans.rs` (skip the frontmatter range when styling)
- Modify: `src/reader.rs:129-141` (outline excludes it)
- Modify: `src/view.rs` (render the block quietly)
- Test: inline in each

A `---` block at the start of a file is not recognised anywhere. CommonMark's Setext rule then makes the metadata lines plus the closing `---` into **one H2 whose range swallows the closing delimiter** — verified against the vendored pulldown-cmark 0.13.4. The result is five lines of large bold text and a phantom outline entry.

This task stops the misparse. It does **not** add frontmatter support — no tag or alias reading, no property editing. That is #52 in 0.0.18.

- [ ] **Step 1: Write the failing tests**

```rust
    /// The delimited block at the top of a file is metadata, not a
    /// heading. CommonMark's setext rule turns it into one, which is
    /// why a note's title line used to render as the loudest thing on
    /// the page.
    #[test]
    fn frontmatter_is_its_own_block_not_a_heading() {
        let src = "---\ntitle: Weekly Review\ntags: [planning]\n---\n\n# Real Heading\n\nBody.\n";
        let doc = parse(src);
        assert!(
            matches!(doc.blocks.first(), Some(Block::FrontMatter(_))),
            "first block is frontmatter, got {:?}",
            doc.blocks.first()
        );
        let headings: Vec<_> = doc
            .blocks
            .iter()
            .filter_map(|b| match b {
                Block::Heading { level, content } => Some((*level, content.text.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(headings.len(), 1, "only the real heading: {headings:?}");
        assert_eq!(headings[0].0, 1);
    }

    /// Only at the very start, and only when it closes. A --- further
    /// down is a thematic break and must stay one.
    #[test]
    fn frontmatter_is_recognised_only_at_the_top_and_only_when_closed() {
        assert!(frontmatter_range("---\na: 1\n---\nbody\n").is_some());
        assert!(frontmatter_range("\n---\na: 1\n---\n").is_none(), "not at the top");
        assert!(frontmatter_range("---\na: 1\nnever closes\n").is_none(), "unclosed");
        assert!(frontmatter_range("body\n\n---\n\nmore\n").is_none(), "a real rule");
        assert!(frontmatter_range("").is_none());
    }
```

```rust
    /// The outline lists the document's headings. A metadata block is
    /// not one, and it used to appear there as a garbled row.
    #[test]
    fn the_outline_skips_frontmatter() {
        let src = "---\ntitle: x\ntags: [a]\n---\n\n# Only Me\n";
        let entries = toc_for(src);
        assert_eq!(entries.len(), 1, "got {entries:?}");
        assert_eq!(entries[0].text, "Only Me");
    }
```

Check `reader.rs:129-141` for the real `TocEntry` field names and the function that builds them; name the helper accordingly.

- [ ] **Step 2: Run and watch them fail**

```sh
cargo test --bin supermd frontmatter_ the_outline_skips_frontmatter
```

- [ ] **Step 3: Implement**

Add the pure detector to `src/markdown.rs` so the editor and the reading path share one definition:

```rust
/// Byte range of a YAML frontmatter block, if the source opens with
/// one. Only at the very start, and only when it closes -- a `---`
/// anywhere else is a thematic break and stays one.
pub fn frontmatter_range(src: &str) -> Option<std::ops::Range<usize>> {
    let rest = src.strip_prefix("---\n")?;
    let mut offset = 4;
    for line in rest.split_inclusive('\n') {
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed == "---" || trimmed == "..." {
            return Some(0..offset + line.len());
        }
        offset += line.len();
    }
    None
}
```

Add `FrontMatter(String)` to `Block` (`src/markdown.rs:55`). In `parse`, check `frontmatter_range` first; if present, push `Block::FrontMatter` with the inner text and parse only the remainder, offsetting nothing else — the reading path does not need source offsets.

In `src/editor/spans.rs`, take the same range and emit a single `StyleKind::FrontMatter` span over it instead of letting pulldown-cmark's setext heading through. Add that variant to `StyleKind` (`:10`). In `editor/mod.rs`'s `line_typography`, give it the mono family at `ui_size` in `fg_muted` — quiet, readable, obviously not prose.

In `src/reader.rs`, skip `Block::FrontMatter` when building the outline. In `src/view.rs`, render it as a compact muted block: mono, small, `code_bg` background, `border_subtle` hairline.

`heading_lines()` (`editor/mod.rs:668`) builds the editor's outline from heading spans — with frontmatter no longer producing one, it needs no change, but verify.

- [ ] **Step 4: Run**

```sh
cargo test --bin supermd frontmatter_ outline markdown:: spans:: && cargo test --bin supermd
cargo build --bin supermd
```

- [ ] **Step 5: Look at it**

```sh
cargo run -- examples/vault
```
Create a note with frontmatter. It should be quiet and small, and absent from the outline.

- [ ] **Step 6: Commit**

```bash
git add src/markdown.rs src/editor/spans.rs src/editor/mod.rs src/reader.rs src/view.rs
git commit -m "fix: frontmatter is metadata, not a giant heading

A --- block at the top of a file was unrecognised, so CommonMark's
setext rule made the metadata lines plus the closing delimiter into one
H2 -- five lines of large bold text, and a garbled outline entry.

Now detected once, shared by the editor and the reading path, rendered
quietly and kept out of the outline. Recognised only at the very start
and only when it closes, so a --- anywhere else is still a rule.

This is not frontmatter support: no tags, no aliases, no properties.
That is #52.

Closes #38"
```

---

### Task 13: HTML blocks stop vanishing (#37)

**Files:**
- Modify: `src/markdown.rs:509-511` (the catch-all) and `Block`
- Modify: `src/view.rs` (render the new block)
- Test: inline in both

`parse("<div>raw</div>")` returns an **empty document** — asserted today by `html_is_ignored` (`src/markdown.rs:866`). A user pasting an HTML snippet watches it disappear from the preview with no indication. Not rendering HTML is defensible; erasing it is not.

- [ ] **Step 1: Replace the test that asserts the bug**

`html_is_ignored` asserts the current behaviour. Rewrite it — the old expectation is the defect:

```rust
    /// HTML is not rendered, but it is never erased. The old behaviour
    /// dropped the block entirely, so a pasted snippet vanished from
    /// the preview with nothing to show the user it had gone.
    #[test]
    fn html_blocks_are_kept_as_literal_text() {
        let doc = parse("<div>raw</div>\n");
        assert!(
            matches!(doc.blocks.first(), Some(Block::Html(_))),
            "got {:?}",
            doc.blocks.first()
        );
        if let Some(Block::Html(s)) = doc.blocks.first() {
            assert!(s.contains("<div>raw</div>"), "the source survives: {s:?}");
        }
    }
```

- [ ] **Step 2: Run and watch it fail**

```sh
cargo test --bin supermd html_blocks_are_kept_as_literal_text
```

- [ ] **Step 3: Implement**

Add `Html(String)` to `Block`. In the event match, handle `Event::Html` and the `HtmlBlock` tag pair by accumulating the raw text into a `Block::Html` instead of falling into the catch-all at `:509`. Leave inline HTML (`Event::InlineHtml`) as it is — this issue is block level.

Render it in `src/view.rs` as literal monospace text on `code_bg` with a `border_subtle` hairline: visibly not prose, visibly not lost.

Update the comment at `:509` — it currently names HTML alongside footnotes and math as out of scope, and HTML no longer is.

- [ ] **Step 4: Run**

```sh
cargo test --bin supermd html_ markdown:: view:: && cargo test --bin supermd && cargo build --bin supermd
```

- [ ] **Step 5: Commit**

```bash
git add src/markdown.rs src/view.rs
git commit -m "fix: HTML blocks render as literal text instead of vanishing

parse(\"<div>raw</div>\") returned an empty document, and a test
asserted that. A user pasting a snippet watched it disappear from the
preview with no indication.

We still do not render HTML. We no longer erase it.

Closes #37"
```

---

### Task 14: Two table-command bugs (#33, #34)

**Files:**
- Modify: `src/editor/mod.rs:1554` (delete-row caret) and the table command handlers
- Test: inline in `src/editor/mod.rs`

Both live in the same handful of lines. #33: with the caret in the only body row, **Delete Row** leaves it in the delimiter, and the next keystroke produces `| z--- | --- |`, which `blocks()` then sees as zero tables. #34: a refusal on the header or separator row returns early and tells the user nothing.

- [ ] **Step 1: Write the failing tests**

```rust
    /// After deleting a row the caret must land somewhere you can
    /// type. The delimiter is not such a place: one keystroke there
    /// turns the table into a paragraph of pipes.
    #[gpui::test]
    fn delete_row_never_parks_the_caret_in_the_delimiter(cx: &mut TestAppContext) {
        let (_fx, editor, cx) = open_editor(cx, "t.md", "| a | b |\n| --- | --- |\n| 1 | 2 |\n");
        editor.update(cx, |ed, cx| {
            ed.set_cursor_at_line(2, cx);
            ed.table_delete_row(&TableDeleteRow, cx);
        });
        editor.update(cx, |ed, cx| {
            ed.insert_str("z", cx);
            let text = ed.text();
            assert!(
                !text.contains("z---"),
                "the caret was in the delimiter: {text:?}"
            );
            assert_eq!(
                crate::editor::blocks::blocks(&text)
                    .iter()
                    .filter(|b| matches!(b.kind, crate::editor::blocks::BlockKind::Table))
                    .count(),
                1,
                "still one table: {text:?}"
            );
        });
    }

    /// A command that declines tells the user why. Silence is
    /// indistinguishable from a broken command.
    #[gpui::test]
    fn deleting_the_header_row_says_why_it_refused(cx: &mut TestAppContext) {
        let (_fx, editor, cx) = open_editor(cx, "t.md", "| a | b |\n| --- | --- |\n| 1 | 2 |\n");
        editor.update(cx, |ed, cx| {
            ed.set_cursor_at_line(0, cx);
            ed.table_delete_row(&TableDeleteRow, cx);
        });
        editor.update(cx, |ed, _| {
            assert_eq!(ed.text(), "| a | b |\n| --- | --- |\n| 1 | 2 |\n", "unchanged");
            assert!(
                ed.last_command_error().is_some(),
                "the refusal reached the user"
            );
        });
    }
```

Grep for the real names: the action type, the cursor-setting helper used by existing table tests, and how an editor surfaces a message (the editor may need to emit an event the workspace turns into `show_command_error` — follow whatever `can_format()`-gated commands already do).

- [ ] **Step 2: Run and watch them fail**

```sh
cargo test --bin supermd delete_row_never_parks deleting_the_header_row_says_why
```

- [ ] **Step 3: Fix both**

At `src/editor/mod.rs:1554`, clamp to a **body** row rather than any row:

```rust
        // Row 1 is the delimiter; a caret there turns the next
        // keystroke into `| z--- | --- |`. Clamp to a body row, and
        // fall back to the header when the body is now empty.
        let rows = table_edit::rows(&new_block).len();
        let row = if rows > 2 { pos.row.clamp(2, rows - 1) } else { 0 };
```

Verify the row indices against `table_edit::rows` — header is 0, delimiter is 1, body starts at 2. Confirm by reading `src/editor/table_ops.rs:38` (`delete_row`), whose own tests document the separator rule.

For the refusal, give each early return a message routed the same way other command errors are.

- [ ] **Step 4: Run**

```sh
cargo test --bin supermd table_ delete_row deleting_the_header && cargo test --bin supermd
cargo build --bin supermd
```

- [ ] **Step 5: Commit**

```bash
git add src/editor/mod.rs
git commit -m "fix: delete-row caret and silent table refusals

Deleting the only body row parked the caret in the delimiter, where the
next character typed produced | z--- | --- | and the table stopped
being a table. Clamped to a body row, falling back to the header when
no body row remains.

Refusals on the header and separator rows now say why instead of
doing nothing, which was indistinguishable from a broken command.

Closes #33
Closes #34"
```

---

### Task 15: Right-click reaches a widget table (#35)

**Files:**
- Modify: `src/editor/mod.rs` (the right-click handler added in 5c1c424, and `render_table`)
- Test: inline in `src/editor/mod.rs`

The editor's context menu is attached to line elements. A table whose caret is outside renders as a widget, which is not a line element, so right-clicking it does nothing — the exact gesture people make when they want table commands.

- [ ] **Step 1: Write the failing test**

```rust
    /// Right-clicking a rendered table offers the table commands. The
    /// menu lives on line elements, and a widget is not one, so this
    /// gesture used to do nothing at all.
    #[gpui::test]
    fn right_clicking_a_widget_table_opens_the_table_menu(cx: &mut TestAppContext) {
        let (_fx, editor, cx) = open_editor(
            cx,
            "t.md",
            "para\n\n| a | b |\n| --- | --- |\n| 1 | 2 |\n\nafter\n",
        );
        // Caret in the paragraph, so the table renders as a widget.
        editor.update(cx, |ed, cx| ed.set_cursor_at_line(0, cx));
        cx.run_until_parked();
        editor.update(cx, |ed, cx| ed.right_click_table_row(2, cx));
        editor.update(cx, |ed, _| {
            assert!(
                ed.context_menu_facts().in_table,
                "the menu knows it is in a table"
            );
            assert!(
                (2..=4).contains(&ed.cursor_line()),
                "the caret moved into the table so the commands can act"
            );
        });
    }
```

Match the real names for the menu-facts accessor introduced in 5c1c424 — grep `src/menus.rs` and `src/editor/mod.rs` for the function that answers `in_table` / `in_ordered_list` / `on_link`.

- [ ] **Step 2: Run and watch it fail**

```sh
cargo test --bin supermd right_clicking_a_widget_table
```

- [ ] **Step 3: Handle right-click on the widget**

In `render_table` (`editor/mod.rs`, near `:3607`), add a `MouseButton::Right` handler beside the existing left-click row handler that already drops the caret onto a row's source line (`:3661`). It should place the caret on that row **and** open the same context menu the line handler opens, with facts computed after the caret moves — so `in_table` is true.

- [ ] **Step 4: Run**

```sh
cargo test --bin supermd right_click context_menu table_ && cargo test --bin supermd
cargo build --bin supermd
```

- [ ] **Step 5: Look at it**

```sh
cargo run -- examples/vault
```
Right-click a table you have not clicked into. The row/column commands should appear.

- [ ] **Step 6: Commit**

```bash
git add src/editor/mod.rs
git commit -m "fix: right-clicking a rendered table opens the table menu

The context menu is attached to line elements and a widget table is not
one, so the gesture people actually make when they want row and column
commands did nothing. Right-click now places the caret in the row and
opens the menu with the table facts already true.

Closes #35"
```

---

### Task 16: One ignore rule, two audiences (#36, #31)

**Files:**
- Modify: `src/files.rs` (`tree_walk_builder`, `IndexMatcher:103`)
- Modify: `src/workspace.rs` (`on_fs_events` — the batch early return)
- Test: inline in both

Two halves of the same confusion. #36: `tree_walk_builder` sets `git_ignore(false)` but leaves `WalkBuilder::ignore()` at its default `true`, so a file excluded by `.ignore` or `.rgignore` is dropped from the sidebar entirely rather than dimmed. #31: `on_fs_events` still returns early when no path in a batch passes the visibility gate, so a gitignored file created by an outside tool leaves the sidebar stale — the gate conflates "build noise" with "gitignored", and only the first should silence the watcher.

- [ ] **Step 1: Write the failing tests**

```rust
    /// The sidebar shows what the index excludes, dimmed. A .ignore
    /// file made rows vanish instead, which is the one affordance
    /// telling the user a file is outside the index.
    #[test]
    fn ignore_files_are_dimmed_in_the_sidebar_not_omitted() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join(".ignore"), "drafts/\n").unwrap();
        std::fs::create_dir(root.path().join("drafts")).unwrap();
        std::fs::write(root.path().join("drafts/x.md"), "# x\n").unwrap();
        let names = tree_entry_names(root.path());
        assert!(names.iter().any(|n| n == "drafts"), "present: {names:?}");
        let entry = tree_entry(root.path(), "drafts");
        assert!(entry.ignored, "and dimmed");
    }
```

```rust
    /// Build noise silences the watcher; a gitignored note does not.
    /// The old gate conflated them, so writing to a gitignored file
    /// from a terminal left the sidebar stale until something else
    /// happened.
    #[test]
    fn only_build_noise_silences_the_watcher() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join(".gitignore"), "drafts/\n").unwrap();
        assert!(
            !is_build_noise(root.path(), &root.path().join("drafts/secret.md")),
            "a gitignored note still refreshes the sidebar"
        );
        assert!(
            is_build_noise(root.path(), &root.path().join("target/debug/x.o")),
            "build output does not"
        );
        assert!(is_build_noise(root.path(), &root.path().join(".git/index")));
    }
```

- [ ] **Step 2: Run and watch them fail**

```sh
cargo test --bin supermd ignore_files_are_dimmed only_build_noise
```

- [ ] **Step 3: Split the predicate**

In `src/files.rs`, add the narrow question:

```rust
/// Is this path build output or VCS internals -- the churn the watcher
/// exists to ignore? This is deliberately NOT "is it gitignored": a
/// gitignored note is a note, and the sidebar shows it dimmed.
pub fn is_build_noise(root: &Path, path: &Path) -> bool {
    // IGNORED_DIRS / PROTECTED_ROOT_DIRS already name these; reuse them
    // rather than writing a second list.
    ...
}
```

Reuse the existing directory constants rather than introducing a second list — grep `src/files.rs` for `IGNORED_DIRS` and `PROTECTED_ROOT_DIRS` and build the predicate from them.

In `tree_walk_builder`, also set `.ignore(false)` and `.parents(false)` so `.ignore`/`.rgignore` files stop removing rows; the `ignored` flag on `FsEntry` still comes from the index matcher, so those rows arrive dimmed.

In `workspace.rs`'s `on_fs_events`, change the batch early return to use `is_build_noise` instead of the visibility gate. Leave the **per-path index admission** exactly as it is — that is `IndexMatcher`, it is correct, and it is not what this issue is about.

- [ ] **Step 4: Run**

```sh
cargo test --bin supermd ignore_ build_noise files:: on_fs_events && cargo test --bin supermd
cargo build --bin supermd
```

- [ ] **Step 5: Commit**

```bash
git add src/files.rs src/workspace.rs
git commit -m "fix: the sidebar shows ignored files, and the watcher wakes for them

tree_walk_builder left WalkBuilder::ignore() on, so a .ignore or
.rgignore file removed sidebar rows entirely instead of dimming them --
a hole in the goal 0.0.16 set out to reach.

And on_fs_events dropped a whole batch when no path passed the
visibility gate, which conflated build noise with gitignored notes.
Writing to a gitignored file from a terminal left the sidebar stale.
The gate now asks only whether the path is build output.

Index admission is unchanged: IndexMatcher still decides what the
knowledge index reads.

Closes #36
Closes #31"
```

---

### Task 17: Checkboxes toggle in the reading view too (#39)

**Files:**
- Modify: `src/view.rs:401-449` (the list row builder)
- Modify: `src/reader.rs` (accept the toggle and write it back)
- Test: inline in `src/reader.rs`

`display.rs:232-242` already carries a `toggle` payload so a click flips `[x]`↔`[ ]` in the editor without moving the caret, proven by `clicking_a_checkbox_glyph_toggles_the_task` (`mod.rs:7123`). The reading view renders the same `✓`/`○` at `view.rs:401-407` with no click handler and no element id — the marker is decoration. The same checkbox toggles in one view and not the other.

- [ ] **Step 1: Write the failing test**

```rust
    /// The editor has toggled checkboxes since the marker carried a
    /// toggle payload. The reading view drew the same glyph and did
    /// nothing with it.
    #[gpui::test]
    fn clicking_a_checkbox_in_the_reading_view_toggles_the_file(cx: &mut TestAppContext) {
        let _home = temp_home();
        let fx = tempfile::tempdir().unwrap();
        let path = fx.path().join("todo.md");
        std::fs::write(&path, "- [ ] one\n- [x] two\n").unwrap();
        let (reader, cx) = open_reader(cx, &path);
        cx.run_until_parked();
        reader.update(cx, |r, cx| r.toggle_task(0, cx));
        cx.run_until_parked();
        let on_disk = std::fs::read_to_string(&path).unwrap();
        assert_eq!(on_disk, "- [x] one\n- [x] two\n", "the file is the truth");
    }
```

Grep `src/reader.rs` for how a reader is opened in existing tests and match the helper name.

- [ ] **Step 2: Run and watch it fail**

```sh
cargo test --bin supermd clicking_a_checkbox_in_the_reading_view
```

- [ ] **Step 3: Implement**

Give each task row an element id and an `on_click` that calls a new `Reader::toggle_task(index, cx)`. The pure part — turning "the Nth task in the document" into a byte edit — belongs beside the existing markdown logic, not in the view:

```rust
/// Byte range of the Nth task marker's state character, and what it
/// should become. `None` when the index is past the last task.
pub fn task_toggle(src: &str, nth: usize) -> Option<(std::ops::Range<usize>, &'static str)>
```

Put it in `src/markdown.rs` next to `frontmatter_range`, test it directly with several documents, and have the reader apply the edit and save through the same path an editor save uses — the file on disk stays the source of truth.

- [ ] **Step 4: Run**

```sh
cargo test --bin supermd task_toggle clicking_a_checkbox reader:: && cargo test --bin supermd
cargo build --bin supermd
```

- [ ] **Step 5: Commit**

```bash
git add src/markdown.rs src/view.rs src/reader.rs
git commit -m "fix: checkboxes toggle in the reading view

The editor has flipped [x] on click since the marker carried a toggle
payload; the reading view drew the same glyph as decoration, so the
same checkbox toggled in one view and not the other.

The mapping from Nth task to byte edit is pure and tested on its own;
the view only dispatches.

Closes #39"
```

---

### Task 18: Docs and performance, batched (#42, #43, #44, #45)

**Files:**
- Modify: `src/files.rs` (`IndexMatcher` doc comment; `escapes_via_hardlink`)
- Modify: `docs/site/knowledge.md`, then regenerate `site/docs/`
- Modify: `src/editor/lists.rs:78` (`fence_bodies`) and its callers
- Test: inline

Four small independent edits of the same shape. One dispatch, one review.

- [ ] **Step 1: #42 — `.rgignore` is documented but not honoured**

`index_walk_builder` never calls `add_custom_ignore_filename(".rgignore")`. Harmless in effect — scanner, watcher and sidebar ignore it equally — but the `IndexMatcher` doc comment and the 0.0.16 fix report both claim it is honoured. **Correct the comment** rather than adding the call; adding it would change what the index admits, which is not this task's job.

- [ ] **Step 2: #43 — a unix-only promise on a platform-neutral page**

`escapes_via_hardlink` and `hardlinked_outside_the_workspace` are `#[cfg(unix)]` with a `false` stub elsewhere, so on Windows a hardlinked note is indexed normally. `docs/site/knowledge.md` states the guarantee without qualification. Qualify the sentence, then:

```sh
cargo run --example build_docs
```
and commit the regenerated `site/docs/` output.

- [ ] **Step 3: #44 — memoize the hardlink walk**

For each path with `nlink > 1` in a batch, `escapes_via_hardlink` walks the entire workspace to count links inside the root. Free for an ordinary vault; a vault inside a `cp -al` backup tree has `nlink > 1` on every file, so one save walks the workspace once per changed path. Build the link-count map **once per `on_fs_events` batch**, the way the matcher already is, and pass it in.

Test it by counting walks:

```rust
    /// A vault inside a hardlinked backup tree has nlink > 1 on every
    /// file. One save must not walk the workspace once per path.
    #[cfg(unix)]
    #[test]
    fn the_hardlink_walk_happens_once_per_batch() {
        // Build a root with three hardlinked notes, run the batch
        // admission with a counting walker, assert the walk ran once.
    }
```

Write that test concretely against whatever seam you introduce — a counter passed in, or a `LinkCounts` struct built once. If no seam exists without inventing one, say so in the report and leave #44 open rather than adding production scaffolding for a test.

- [ ] **Step 4: #45 — stop reparsing the whole document on every Enter**

`renumber_block` runs a full pulldown-cmark parse of the entire document to find fence ranges to avoid. Cost scales with document size, not list size, and it runs on a keystroke. `fence_bodies` already receives the text; scope it to the block range the caller already knows.

```rust
    /// Enter inside a list must not cost a parse of the whole file.
    #[test]
    fn renumber_only_parses_the_block_it_touches() {
        let long = format!("{}\n1. one\n1. two\n", "filler paragraph.\n\n".repeat(400));
        let block = long.len() - "1. one\n1. two\n".len()..long.len();
        let out = renumber_block(&long, block).expect("renumbers");
        assert!(out.contains("2. two"));
    }
```

Keep the fence-awareness the 0.0.16 fix added — a numbered line inside a fence must still not be renumbered.

- [ ] **Step 5: Run and commit**

```sh
cargo test --bin supermd && cargo test --bin supermd --no-default-features --features mas
cargo build --bin supermd
```

```bash
git add src docs site
git commit -m "fix: four small ones -- docs that overclaim, work that repeats

.rgignore was documented as honoured and never was: the comment was
wrong, not the code. knowledge.md promised hardlink exclusion on a
platform-neutral page when it is unix-only.

The hardlink walk now happens once per event batch rather than once per
changed path, which matters for a vault inside a cp -al backup tree
where every file has nlink > 1. And Enter in a numbered list no longer
parses the whole document to find fences it must avoid.

Closes #42
Closes #43
Closes #44
Closes #45"
```

---

### Task 19: Test gap, CI honesty, changelog (#46, #47, #48)

**Files:**
- Modify: `src/reader.rs` (guard the `describe` call site)
- Modify: `.github/workflows/release.yml`
- Create: `CHANGELOG.md`
- Test: inline in `src/reader.rs`

- [ ] **Step 1: #46 — guard `Reader::describe`'s call site**

The only test calls `describe` directly, so replacing the render closure's body with a call that passes no index leaves the suite green. If it regressed, every wiki and relative link in the reading view would preview as "does not exist".

Write a test that fails on that one-line change. If the render closure genuinely has no testable seam, extract the argument choice into a named function and test that — and say plainly in the report which of the two you did.

- [ ] **Step 2: #47 — CI stops reporting a build it did not do**

The `mas` job is gated on `secrets.MAS_CERT_P12 != ''`, which has never been set, so every release — 0.0.15 and 0.0.16 included — skipped certificate import, plugin build, bundling, the private-API check and the artifact upload, while reporting green. The App Store package has always been built locally.

Make the job **fail** on a release tag when the secrets are absent:

```yaml
      - name: Refuse to pass silently on a release tag
        if: ${{ startsWith(github.ref, 'refs/tags/v') && env.HAVE_CERT != 'true' }}
        run: |
          echo "::error::MAS signing secrets are absent, so no App Store package was built."
          echo "Set MAS_CERT_P12, MAS_CERT_PASSWORD, MAS_INSTALLER_P12,"
          echo "MAS_INSTALLER_PASSWORD and MAS_PROFILE, or build locally and"
          echo "acknowledge that this job produces nothing."
          exit 1
```

Do not add the secrets — that is the maintainer's call, and the point of this change is that the checkmark stops lying.

- [ ] **Step 3: #48 — a changelog the release flow can read**

The generated release body for 0.0.16 was a single line linking the PR and had to be rewritten by hand. The 0.0.16 note currently sits under a "Release notes" heading in `docs/HISTORY.md`, which nothing references.

Create a root `CHANGELOG.md` in Keep-a-Changelog shape, move the 0.0.16 note into it from `docs/HISTORY.md`, and add an `Unreleased` section. Leave the release workflow reading it as a follow-up unless it is a one-line change to `gh release create`.

- [ ] **Step 4: Run and commit**

```sh
cargo test --bin supermd && cargo build --bin supermd
```

```bash
git add src/reader.rs .github/workflows/release.yml CHANGELOG.md docs/HISTORY.md
git commit -m "chore: guard a wiring, stop a green checkmark from lying, add a changelog

Reader::describe was tested directly, so deleting the argument at the
render call site left the suite green while every wiki link in the
reading view would have previewed as missing.

The App Store CI job has skipped every step on every release while
reporting success, because the signing secrets have never been set. It
now fails on a release tag instead of passing silently.

And release notes have a file to live in rather than being rewritten by
hand after each publish.

Closes #46
Closes #47
Closes #48"
```

---

### Task 20: Ship 0.0.17

**Files:** `Cargo.toml`, `Cargo.lock`, `CHANGELOG.md`

- [ ] **Step 1: Full verification**

```sh
bash scripts/build_plugins.sh --fixtures
cargo test --bin supermd
cargo test --bin supermd --no-default-features --features mas
cargo build --bin supermd
cargo llvm-cov --summary-only --fail-under-lines 90
```
`grep -c SKIP` on the test output must be 0 — without built fixtures the sandbox tests no-op while still counting as passes.

- [ ] **Step 2: Look at every theme, in both modes**

```sh
cargo run -- examples/vault
```
Cycle all eight themes. Check a document, a table, a mermaid diagram, the graph, the finder, and a dialog. This is the check no test performs.

- [ ] **Step 3: Bump the version**

`Cargo.toml` to `0.0.17`, then `cargo update -p supermd --precise 0.0.17`. **Commit before tagging** — cargo-deb reads the manifest while the DMG and App Store builds take the tag, and `is_newer` comparing a stale manifest against the new tag shows every user an update prompt for the release they just installed.

```bash
git add Cargo.toml Cargo.lock CHANGELOG.md
git commit -m "release: v0.0.17"
```

- [ ] **Step 4: Hand the release to the user**

Tagging, pushing and submitting to App Review are outward-facing and irreversible. Stop here and report what remains:

```sh
git push origin master
git tag -a v0.0.17 -m "v0.0.17 — a proper desk"
git push origin v0.0.17
```

The App Store package is built locally — CI has never produced one — and needs `~/supermd-signing`, `scripts/bundle_mas.sh 0.0.17`, `xcrun altool --validate-app`, then `--upload-app`.

---

### Task 21: Reopening a window (#53)

**Files:**
- Modify: `src/main.rs` (`app_menus:301`, the action registration near `:417`)
- Modify: `src/commands.rs` if `NewWindow`'s declaration needs a menu placement change
- Test: inline in `src/main.rs` or `src/workspace.rs`

**Interfaces:**
- Consumes: `ws::NewWindow` (declared `src/commands.rs:167`), `Workspace::open_in_new_window`.
- Produces: nothing later tasks depend on.

**Runs BEFORE Task 7**, despite its number. Apple rejected 0.0.16 under Guideline 4 for this, submission `1626587c-7d6e-4cf7-a955-0cf8061edbe4`. Our own 0.0.16 whole-branch review found it first, ranked it Medium, and parked it without filing — which is how it shipped.

Only reachable since 0.0.16: before multiple windows there was never a running app with zero windows.

Three causes, three fixes.

- [ ] **Step 1: Write the failing test**

The app must survive losing its last window. Verify the real helper names first — `open_workspace` and `temp_home` live in `workspace::tests`, and the window-closing call used by `closing_a_window_drops_its_workspace_and_its_watcher_loop` is the model to copy.

```rust
    /// Closing every window must not strand the app. New Window is an
    /// application action, not a window action -- with no window open
    /// there is nothing for a window-scoped action to dispatch to,
    /// which is exactly what Apple rejected 0.0.16 for.
    #[gpui::test]
    fn new_window_works_with_no_window_open(cx: &mut TestAppContext) {
        let _home = temp_home();
        let fx = tempfile::tempdir().unwrap();
        std::fs::write(fx.path().join("n.md"), "# N\n").unwrap();
        let (ws, cx) = open_workspace(cx, fx.path());
        cx.run_until_parked();

        cx.update(|_, app| {
            let handle = app.windows().first().copied().expect("one window");
            handle.remove_window(app);
        });
        cx.run_until_parked();
        cx.update(|_, app| assert!(app.windows().is_empty(), "precondition: no windows"));
        drop(ws);

        cx.update(|_, app| app.dispatch_action(&NewWindow));
        cx.run_until_parked();
        cx.update(|_, app| {
            assert_eq!(app.windows().len(), 1, "New Window opened one from nothing");
        });
    }
```

`App::dispatch_action` may be named differently or may need a window; if a global dispatch is not directly callable in this gpui version, assert instead that the action is registered at application level and that the callback opens a window when invoked. Say in the report which form you used and why.

- [ ] **Step 2: Run it and watch it fail**

```sh
cargo test --bin supermd new_window_works_with_no_window_open
```
Expected: FAIL — no window is created, because `new_window` is a `Workspace` method (`src/workspace.rs:1576`) and there is no workspace to receive it.

- [ ] **Step 3: Register New Window at application level**

`src/main.rs:417` registers exactly one global action, `Quit`. Add `NewWindow` beside it, opening a window with no workspace required:

```rust
        cx.on_action(|_: &NewWindow, cx| {
            crate::workspace::open_in_new_window(None, cx);
        });
```

Check `open_in_new_window`'s real signature before using it — it takes `Option<PathBuf>` and an `&mut App` in Task 8 of the 0.0.16 plan, but verify rather than trust. The window-scoped handler on `Workspace` stays: with a window focused it wins, which is what `new_window_yields_cmd_shift_n_to_a_focused_sidebar` asserts.

- [ ] **Step 4: Reopen on Dock activation**

GPUI exposes the hook and we never call it — `App::on_reopen` (`vendor/gpui/src/app.rs:198`). Clicking the Dock icon with no windows open currently does nothing, which is almost certainly what the reviewer tried first.

```rust
        cx.on_reopen(|cx| {
            if cx.windows().is_empty() {
                crate::workspace::open_in_new_window(None, cx);
            }
        });
```

Guard on emptiness: reopen also fires when windows exist, and a spare window on every Dock click would be its own bug.

- [ ] **Step 5: Add a Window menu**

`app_menus` (`src/main.rs:301`) builds one menu. Apple's guidance names a Window menu listing open windows. Add one containing at least New Window, and the standard Minimize and Zoom if gpui exposes them. If listing *open windows* dynamically is not supported by gpui's `Menu` API, say so plainly in the report — New Window plus Dock reopen already satisfies "provide similar functionality in another menu item", which Apple offers as the alternative.

Adding a menu placement means the shortcut docs regenerate:

```sh
UPDATE_DOCS=1 cargo test shortcut_docs_match_the_table
cargo run --example build_docs
```

- [ ] **Step 6: Mutation-check, both suites, build**

Remove the global registration and confirm the new test fails. Then:

```sh
cargo test --bin supermd
cargo test --bin supermd --no-default-features --features mas
cargo build --bin supermd
```

- [ ] **Step 7: Commit**

```bash
git add src docs site
git commit -m "fix: the app survives losing its last window

New Window was scoped to a window, so with none open it had nothing to
dispatch to -- Apple rejected 0.0.16 for exactly this, and our own
review found it first and parked it without filing.

It is now an application action, the Dock icon reopens a window when
none are left, and a Window menu lists the way back.

Closes #53"
```

---

### Task 22: Appearance — Light, Dark or System (#new)

**Files:**
- Modify: `src/settings.rs` (the `Settings` struct, around line 8)
- Modify: `src/theme.rs` (`ThemeState::resolve`, around line 895)
- Modify: `src/workspace.rs` (the ⌘T theme picker, `toggle_theme_picker` ~3083 and its render)
- Modify: `src/commands.rs` if a new action is needed
- Test: inline in `src/theme.rs` and `src/settings.rs`

**Interfaces:**
- Produces: `settings::Appearance { System, Light, Dark }` and `Settings::appearance`.
- Consumes: `ThemeState::resolve`'s existing `system_dark` and `flux_blend` fields.

Today the app always follows the OS appearance, choosing between the user's
`light_theme` and `dark_theme`. There is no way to say "always light". This adds
one.

**The precedence, decided with the user:** explicit appearance → flux night →
system. An explicit choice beats flux. If the user has said Light, the app stays
light at midnight; `flux.warm_shift` still applies, so they get *warm* light
rather than dark. A setting that says "always Light" and then goes dark is a
setting that lies.

- [ ] **Step 1: Write the failing tests**

```rust
    /// An explicit appearance beats both the system and flux. A
    /// setting that says "always Light" and then goes dark at night is
    /// a setting that lies.
    #[test]
    fn an_explicit_appearance_overrides_system_and_flux() {
        let mut st = ThemeState::for_test();
        st.system_dark = true;

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
        let mut st = ThemeState::for_test();
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
```

`ThemeState::for_test()` may not exist — check how existing `theme.rs` tests
construct a `ThemeState` and follow that. If they build it by hand, do the same
rather than adding a constructor for the test's convenience.

```rust
    /// A settings file written before this field existed still loads,
    /// and gets the behaviour it had before.
    #[test]
    fn settings_without_appearance_default_to_system() {
        let s: Settings = toml::from_str("light_theme = \"Nord\"\n").expect("loads");
        assert_eq!(s.appearance, Appearance::System);
    }
```

- [ ] **Step 2: Run them and watch them fail**

```sh
cargo test --bin supermd an_explicit_appearance system_appearance settings_without_appearance
```
Expected: FAIL — `Appearance` does not exist.

- [ ] **Step 3: Add the setting**

`Settings` already carries `#[serde(default)]` at the struct level, so a new
field with a `Default` impl loads old files unchanged — no per-field attribute
needed.

```rust
/// Which appearance the app uses. `System` follows the OS, which is
/// what SuperMD did before this existed.
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "lowercase")]
pub enum Appearance {
    #[default]
    System,
    Light,
    Dark,
}
```

Add `pub appearance: Appearance` to `Settings` and to its `Default` impl.

- [ ] **Step 4: Teach `resolve` the precedence**

In `ThemeState::resolve` (`src/theme.rs:895`), the `dark` decision currently
reads flux then `system_dark`. Put the explicit choice ahead of both:

```rust
        let dark = match self.settings.appearance {
            Appearance::Light => false,
            Appearance::Dark => true,
            Appearance::System => {
                // Unchanged: flux night forces dark, otherwise follow
                // the system.
                if flux.enabled && flux.auto_dark && self.flux_blend >= 0.5 {
                    true
                } else {
                    self.system_dark
                }
            }
        };
```

Leave the warm-shift branch below it alone — an explicit appearance changes
*which* theme resolves, not whether colours drift with the time of day.

- [ ] **Step 5: Put it in the theme picker**

⌘T (`ToggleThemePicker`) is where someone looking for this will go. Add a
three-way control at the top of the picker — Light / Dark / System — that writes
the setting and applies immediately, the way the theme list already live-previews.

Follow whatever the picker already does to persist a choice; if the picker
currently only sets the `ActiveTheme` global and persists elsewhere, match that
path rather than inventing a second one.

- [ ] **Step 6: Mutation-check, suites, build**

Make `resolve` ignore the explicit appearance (fall through to the old logic)
and confirm `an_explicit_appearance_overrides_system_and_flux` fails. Then:

```sh
cargo test --bin supermd
cargo test --bin supermd --no-default-features --features mas
cargo build --bin supermd
```
Read the build's warnings, not just its exit code.

- [ ] **Step 7: Commit**

```bash
git add src
git commit -m "feat: choose Light, Dark or System

The app always followed the OS appearance, picking between the user's
light and dark themes. There was no way to say 'always light'.

An explicit choice beats flux: if you have said Light, the app stays
light at midnight and flux warms it rather than flipping it. A setting
that says always-Light and then goes dark is a setting that lies.

Precedence: explicit appearance, then flux night, then the system."
```

---

### Task 23: A surface for things that float

**Files:**
- Modify: `src/theme.rs` (new token through `theme_colors!`, `ThemeFileColors`, derivation, contrast guards)
- Modify: `src/elevation.rs` (the `Overlay` tier mapping, if background joins shadow and radius there)
- Modify: every overlay render site mapped by `Overlay::surface()` in Task 10
- Modify: `assets/themes/*.toml` (all eight), `docs/site/themes.md` (+ regenerate `site/docs/`)
- Test: inline in `src/theme.rs` and `src/elevation.rs`

**Interfaces:**
- Consumes: `elevation::Overlay`, `Overlay::surface()`, `.elevated()` from Task 10; `t.page_bg`, `t.panel_bg` from Tasks 2 and 8.
- Produces: `t.floating_bg`.

**Runs directly after Task 10**, despite its number. Decided with the user after Task 10's
implementer measured that a shadow cannot fix the problem.

In Gruvbox Dark the ⌘P finder reads as a hole in the page rather than a card above it:
its background (30,32,33) is darker than the page (40), and a shadow can only darken
the page beside it, which reads as a recess wall. The root cause is that `panel_bg`
serves three unrelated jobs — overlays floating above the page, table headers on the
page, and chrome — and no single value is right for all of them.

The rule: **a surface that floats above the page is never darker than the page.** In
dark themes that means a step lighter; in light themes, where the page is often at or
near white, it may equal the page and let the shadow separate them.

- [ ] **Step 1: Grep, do not assume, where `panel_bg` is painted**

List every site. Split them into overlays (the fifteen `.elevated()` sites from Task 10
and any container they sit in), table headers, and anything else. Only overlays move to
the new token. Report the list.

- [ ] **Step 2: Write the failing tests**

```rust
    /// A surface that floats above the page is never darker than it --
    /// otherwise the shadow beneath reads as a recess, not a lift.
    #[test]
    fn floating_surfaces_are_never_sunken_below_the_page() {
        for (name, t) in shipped_themes() {
            let lf = t.floating_bg.l;
            let lp = t.page_bg.l;
            assert!(lf >= lp - 0.005, "{name}: floating {lf:.3} sits below page {lp:.3}");
        }
    }
```

Extend the existing guards rather than writing parallel ones: `floating_bg` joins the
surface set in `worst_body_contrast` and `worst_muted_contrast`, and every pair that
paints on it — `hover_bg`, `selected_bg`, `border` — joins
`every_background_token_is_visible_on_what_it_is_painted_on`. Composite any
translucent token with `Hsla::blend` before measuring; `Theme::contrast` ignores alpha.

- [ ] **Step 3: Watch them fail**

Expected: `floating_bg` does not exist.

- [ ] **Step 4: Add the token**

Optional in `ThemeFileColors` with a derivation, through `theme_colors!` so flux warms it:
dark themes derive one step lighter than `page_bg`; light themes derive `page_bg` itself.
A theme that omits the key must keep loading.

- [ ] **Step 5: Point the overlays at it**

If Task 10's `.elevated()` can set the background along with shadow and radius without
fighting a site's own `.bg()`, centralise it there — then a new overlay cannot forget.
Otherwise update each site and say why centralising did not work.

- [ ] **Step 6: Tune the eight themes and look**

Set `floating_bg` explicitly per theme where the derivation is not right. Open the
finder, palette, search, theme picker and a dialog in Gruvbox Dark, one other dark
theme and one light theme. Report per surface, per theme.

- [ ] **Step 7: Docs, mutation check, suites, build**

Document `floating_bg` in `docs/site/themes.md`, regenerate. Make `floating_bg` equal
Gruvbox's old `panel_bg` and confirm the sunken test fails; remove it from the body-ink
surface set and confirm a band ceiling or floor fires. Both suites; read the build's
warnings.

- [ ] **Step 8: Commit**

```bash
git add src assets docs site
git commit -m "feat: things that float get a surface of their own

panel_bg served the sidebar, table headers and every popup, and no single
value suits all three. In Gruvbox Dark the finder came out darker than the
page it floats over, and a shadow can only darken the page beside it --
so it read as a hole rather than a card.

floating_bg is never darker than the page. Optional in theme files,
derived when absent, and measured by the contrast guards on every pair
painted on it."
```

---

### Task 24: The reading view renders images (#57)

**Files:**
- Modify: `src/markdown.rs` (a block-level image case; the inline placeholder stays)
- Modify: `src/view.rs` (render the block)
- Maybe modify: `src/editor/mod.rs` (extract path resolution so both paths share it)
- Test: inline in `src/markdown.rs` and `src/view.rs`

**Interfaces:**
- Consumes: `Block` (reading path), the editor's image path resolution.
- Produces: nothing later tasks depend on.

**Runs in Batch B**, decided with the user after two screenshots showed the same
document rendering an image in the editor and a placeholder in the reading view.

`src/markdown.rs:595-602` turns every image — inline or standalone — into `🖼 ` plus
its alt text, marked "Phase 0". The editor renders images properly:
`BlockKind::Image` → `ImageProjector` → `render_image` (`src/editor/mod.rs:3798`),
which resolves a local path against the document's directory, treats `http://` and
`https://` as remote, and falls back to `![alt](dest) — file not found`.

**An image on its own is a block; an image among words stays inline.** The
placeholder is right for the inline case and wrong for the standalone one.

- [ ] **Step 1: Write the failing tests**

```rust
    /// A standalone image is a block, not a run of text. The editor has
    /// rendered these since 0.0.14; the reading view showed alt text.
    #[test]
    fn a_standalone_image_is_its_own_block() {
        let doc = parse("Before\n\n![A city](city.png)\n\nAfter\n");
        assert!(
            doc.blocks.iter().any(|b| matches!(b, Block::Image { dest, alt }
                if dest == "city.png" && alt == "A city")),
            "standalone image did not become a block: {:?}", doc.blocks
        );
    }

    /// An image among words keeps the inline placeholder -- a picture
    /// cannot sit inside a line of prose.
    #[test]
    fn an_inline_image_keeps_its_placeholder() {
        let doc = parse("Text with ![a pic](p.png) inside.\n");
        assert!(!doc.blocks.iter().any(|b| matches!(b, Block::Image { .. })));
        let text = doc.plain_text();
        assert!(text.contains("🖼"), "inline placeholder lost: {text}");
    }
```

Verify `Block`'s real name and shape, and whether `plain_text()` exists, before
relying on either.

- [ ] **Step 2: Run them and watch them fail**

- [ ] **Step 3: Add the block case**

A `Start(Tag::Image)` that opens with no inline builder mid-paragraph — or whose
paragraph holds nothing else — becomes `Block::Image { alt, dest }`. Everything else
keeps the placeholder. Match how `blocks.rs` decides the same thing in the editor so
the two views agree; if the rules differ, say why in the report.

- [ ] **Step 4: Render it**

In `src/view.rs`, render the block with `gpui::img(...)`, constrained to the page
measure (not the window width — the document is a page since Task 4), with
`elevation::radius` for its corners. Resolve the path exactly as the editor does:
share `render_image`'s resolution rather than writing a second copy. **A missing file
must look deliberate**, matching the editor's `— file not found` rather than showing
nothing.

- [ ] **Step 5: Look at it**

Open a document with a standalone image, a missing image, an inline image and a
remote image, in the editor and the reading view. They must agree. Check a very wide
image is bounded by the page, and that the corner radius is not squared off by the
image's own fill (gpui's `ContentMask` cannot round-clip).

- [ ] **Step 6: Mutation check, suites, build**

Make the block case also catch inline images and confirm
`an_inline_image_keeps_its_placeholder` fails. Both suites; read the build's warnings.

- [ ] **Step 7: Commit**

```bash
git add src
git commit -m "fix: the reading view renders images instead of naming them

Every image became a placeholder, inline or not, while the editor drew
the picture -- the two views disagreed about the same document.

A standalone image is now a block and renders at the page measure,
resolving its path the way the editor already did. An image among
words keeps the placeholder: a picture cannot sit inside a line.

Closes #57"
```

---

### Task 25: Themes from base16, converted not copied

**Files:**
- Create: `assets/base16/*.yaml` (vendored scheme sources) and `assets/base16/LICENSE`
- Create: `examples/import_base16.rs` (the generator, run like `build_docs`)
- Create: `assets/themes/<scheme>.toml` (generated, committed)
- Modify: `src/theme.rs` (`builtin_theme_sources`, `shipped_themes`), `docs/site/themes.md`
- Test: inline in `examples/import_base16.rs` or a new `src/base16.rs` if the mapping belongs in the crate

**Interfaces:**
- Consumes: the theme file format (`ThemeFileColors`), `floating_bg` (Task 23), the contrast guards.
- Produces: many more shipped themes; Task 26 consumes the larger set.

`tinted-theming/schemes` (MIT) holds **340 base16 schemes** in one machine-readable
format, including every family the user asked for and four SuperMD already ships by
hand. One converter beats eighteen hand-written files, and the four hand-written
themes become the converter's test fixtures.

Base16 gives sixteen slots: `base00`–`base07` run background to foreground, and
`base08`–`base0F` are red, orange, yellow, green, cyan, blue, magenta, brown. A
`variant` field says `dark` or `light`.

**Decided with the user:** convert a curated set at build time and commit the
results — not all 340, and not a runtime importer (that is a follow-up once the
mapping is settled). Ship faithful and record bounded exemptions where a palette
fails our floors, exactly as Solarized does.

- [ ] **Step 1: Vendor the sources**

Copy the chosen schemes' YAML into `assets/base16/` with the upstream `LICENSE` and
a note saying where they came from and at what commit. Vendoring keeps the build
reproducible offline and the attribution honest.

Curated set (verified present in the registry; adjust with the user if they prefer
different ones):
`catppuccin-latte`, `catppuccin-frappe`, `catppuccin-macchiato`, `catppuccin-mocha`,
`rose-pine`, `rose-pine-moon`, `rose-pine-dawn`,
`tokyo-night-dark`, `tokyo-night-storm`, `tokyo-night-light`,
`dracula`, `everforest`, `kanagawa`, `onedark`, `one-light`, `monokai`,
`ayu-dark`, `ayu-light`, `github`, `zenburn`.

- [ ] **Step 2: Write the failing test — the converter must reproduce a theme we already approved**

The four hand-written themes (`nord`, `gruvbox-dark-hard`, `solarized-dark`,
`solarized-light`) exist in the registry too. Convert `nord` and compare against the
shipped `nord.toml`: the surfaces and ink must land within a small tolerance of the
values a human chose. Where they cannot match, the difference must be stated in the
report — that is the converter telling you what the mapping loses.

Do **not** overwrite the hand-written four. They stay as they are; they are the
fixtures.

- [ ] **Step 3: The mapping**

Write it once, in one place, as a pure function from the sixteen slots to our tokens.
The obvious mapping, to be checked against the fixtures rather than trusted:

| Ours | base16 |
|---|---|
| `page_bg` | `base00` |
| `bg` (the desk) | `base01` for dark; a step down from `base00` for light |
| `panel_bg` | `base01` |
| `floating_bg` | never darker than `page_bg` — a step up for dark, `page_bg` for light |
| `border` / `border_subtle` | `base02`, and `base02` at reduced alpha |
| `fg` / `fg_strong` / `fg_muted` | `base05` / `base06` or `base07` / `base03` or `base04` |
| `accent` | `base0D` |
| `hover_bg` / `selected_bg` | `base01` / `base02`, kept visible on every surface |
| syntax | `base08`–`base0F` by their published meanings |
| `shadow` | derived, translucent, never opaque |
| diff colours | `base0B` added, `base08` removed |

`base03` is "comments" in base16 and is usually low contrast — it is the slot most
likely to fail our muted floor. Choose `base04` where a scheme has a usable one.

- [ ] **Step 4: Generate, then measure**

Run the generator, commit the `.toml` files, and add every new theme to
`builtin_theme_sources` and `shipped_themes`. The contrast guards now measure them
all. **Never weaken a floor**: a theme that fails gets a bounded exception with its
measured number and a one-line reason, and the report lists every exception added.
If more than half the new themes need an exception for the same token, the mapping is
wrong — say so rather than filing twenty exceptions.

- [ ] **Step 5: Look at them**

You cannot eyeball twenty themes in a document, a table, a diagram, the finder and a
dialog. Do it properly for **four** — one from each family — and for any theme that
needed an exception. Report per theme. Say plainly which ones you did not open.

- [ ] **Step 6: Docs, mutation check, suites, build**

Document in `docs/site/themes.md` where the schemes come from, their licence, and how
to regenerate; regenerate `site/docs/`. Mutate the mapping (swap `base00` and
`base01`) and confirm a guard fires. Both suites; read the build's warnings.

- [ ] **Step 7: Commit**

```bash
git add assets examples src docs site
git commit -m "feat: twenty themes, converted from base16 rather than hand-written

tinted-theming/schemes publishes 340 schemes in one format, four of
which we already shipped by hand. One tested mapping produces the rest,
with the hand-written four as its fixtures.

Faithful to each palette: where one fails our contrast floors it gets a
bounded exception with its measured number, not a loosened floor."
```

---

### Task 26: A picker that can hold thirty themes

**Files:**
- Modify: `src/workspace.rs` (`toggle_theme_picker`, `theme_picker_apply`, the picker render, `ThemePickerState`)
- Test: inline in `src/workspace.rs`

**Interfaces:**
- Consumes: the theme list from `ThemeState`, the appearance control from Task 22.
- Produces: nothing later tasks depend on.

**Runs after Task 25**, which takes the picker from 8 entries to about 28.

Today ⌘T lists every theme, arrow keys move, each move previews live, Enter commits,
Escape restores the theme that was active when it opened. That works at eight and
falls apart at twenty-eight.

- [ ] **Step 1: Write the failing tests**

Type-to-filter is the whole feature, so test the filter as a pure function: matching
is case-insensitive and matches anywhere in the name ("moon" finds Rosé Pine Moon,
"cat" finds all four Catppuccin); a filter that matches nothing leaves the list empty
and commits nothing; clearing the filter restores the full list with the active theme
still marked.

Also test that filtering does not lose the cancel baseline: type, move, press Escape,
and the theme that was active when the picker opened must come back — the same
invariant Task 22 had to repair when the appearance control committed immediately.

- [ ] **Step 2: Watch them fail**

- [ ] **Step 3: Implement**

Add a filter input, seeded empty, that narrows as you type. Keep the appearance
control from Task 22 at the top. Group the list by appearance — dark and light — or
mark each row, so a twenty-eight-item list is navigable; whichever you choose, the
active theme must be visible without scrolling when the picker opens.

Preview on move stays. Escape restores. Enter commits.

- [ ] **Step 4: Mutation check, look at it, suites, build**

Mutate the filter to be case-sensitive and confirm a test fails. Then open the picker
with the full set, type a few letters, arrow through, Escape, and reopen — and report
what you saw. Both suites; read the build's warnings.

- [ ] **Step 5: Commit**

```bash
git add src
git commit -m "feat: the theme picker filters as you type

Eight themes fit in a list. Twenty-eight do not: the picker now filters
as you type and marks light from dark, while preview-on-move, Escape to
restore and Enter to commit all work as before."
```

---
