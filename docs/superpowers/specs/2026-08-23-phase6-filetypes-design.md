# Phase 6: File Types — Seti Icons, More Grammars, Image Tabs

**Date:** 2026-08-23
**Status:** Approved design, pending implementation plan
**Depends on:** Phase 5

## Goal

Sidebar file icons from the full Seti UI pack (169 icons, 336 mapping
rules), tree-sitter grammars for ~8 more common languages, and a
read-only image viewer tab so images open instead of failing.

## Decisions already made

- Icon pack: Seti UI (MIT), vendored wholesale — all SVGs plus a
  GENERATED static Rust mapping table produced by a committed script
  (`scripts/vendor_seti.py`) parsing Seti's `mapping.less`. Builds stay
  offline; rerunning the script refreshes the vendor drop.
- Icons are visual labeling only — no coupling to grammar support.
- Seti's 12 color variables map to two theme-tint tables (light/dark).
- gpui `svg()` tinting behavior verified from vendored source before
  rendering work; fallback is the image pipeline if svg() cannot tint.
- New grammars (drop any whose crate lacks the modern LanguageFn API,
  documented in the commit): YAML, TOML, Ruby, Java, PHP, C++, Swift,
  Kotlin.
- Image tab: read-only, for png/jpg/jpeg/gif/webp/svg/bmp/ico; reuses
  the gpui image pipeline; a new `Tab::Image` variant. No zoom/pan.

## Design

- `src/seti.rs` (generated + committed): `ICONS: &[(&str, &[u8])]`
  (embedded SVG bytes), rule tables, and
  `icon_for(file_name: &str) -> (&'static str, SetiColor)` following
  Seti precedence: exact filename → filename substring → extension →
  default. `SetiColor` = the 12 Seti vars. Hand-written unit tests pin
  a sample of known mappings (rs→rust, tsx→typescript, Dockerfile→
  docker, webpack.config.js→webpack, unknown→default).
- `Assets` struct implementing `gpui::AssetSource` over the embedded
  map, registered with `Application::with_assets`.
- Sidebar rows: 14px tinted `svg()` icon between chevron slot and name;
  folder / open-folder icons for directories.
- `highlight.rs`: register new grammars with the existing `add` helper;
  extend `language_for_path` and alias table. One smoke test per new
  grammar (keyword capture present).
- `workspace.rs`: `Tab::Image { path, title }`; `open_path` routes
  image extensions there before the text path; renders centered,
  column-constrained image (Phase 4 widget styling); outline hidden.

## Out of scope

Custom icon theming/user icon packs, zoom/pan in the image tab,
grammar plugins/lazy loading, per-file language override.

## Testing

TDD: `seti::icon_for` precedence + samples; grammar smoke tests;
image-extension routing predicate (`is_image_path`). Shell (svg
rendering, AssetSource, image tab visuals): manual.
