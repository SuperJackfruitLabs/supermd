# SuperMD pipeline map

These paths are inputs in the selected SuperMD checkout, not files shipped with
this skill. Check the actual checkout before relying on a historical name.

| Boundary | Owner | Regression to inspect |
|---|---|---|
| Source and undo | src/editor/buffer.rs and core.rs | Byte-safe replacements, selection after editing, undo/redo |
| Source styling | src/editor/spans.rs | Source ranges, including hidden Markdown markers |
| Display mapping | src/editor/display.rs | Buffer/display translations while markers hide or reveal |
| Block widgets | src/editor/blocks.rs, projection.rs, projector.rs | Claimed line ranges and cursor-controlled reveal |
| Tables | src/editor/table_edit.rs | Alignment, cell navigation, cursor remapping |
| Native input/view | src/editor/mod.rs | Wrapped geometry, IME, focus, hit testing and vertical movement |
| Theme resolution | src/theme.rs, settings.rs, flux.rs | Theme mapping and time-dependent color transforms |

Use existing inline tests near the owning module. Relevant commands from the
repository root include `cargo test <test-filter>` and `cargo test`. Plugin-host
tests additionally need the repository's fixture plugin build and WASI target;
read current CLAUDE.md before choosing the build command. No plugin build is
required merely to explain an editor offset.

An effective Unicode/table case includes a multibyte cell and a selection whose
correct byte offset differs from its character count. Check bounds and UTF-8
boundaries before slicing. Test the meaningful before/after mapping; a round trip
alone can pass when both directions share the same wrong assumption.

Theme changes must flow through Theme::map_colors and the current resolution
path where applicable. Per-platform behavior belongs in src/platform.rs.
Keep generated documentation and generated icon sources under their existing
owners; a rendering bug does not justify editing generated output by hand.
