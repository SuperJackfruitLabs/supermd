# Extensions Phase 5 Implementation Plan — Viewers, Widgets, Templates, Save Hook

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Custom viewers (file → markdown → Reader), a status strip with widget plugins, palette templates, and an always-on save hook; first-party word-count, daily-note, csv-view, and a toc save hook.

**Architecture:** WIT 0.4 world (fifth `bindgen!`, `Bound::V4`, fallback V4→V3→V2→V1) adds four exports. Viewers reuse the existing `EditorView::Preview(Entity<Reader>)` machinery; widgets live as debounced per-editor state; templates route through the palette `__template:` prefix and a pure `materialize_template` helper; hooks chain in the flush path after the formatter.

**Tech Stack:** existing wasmtime 48 host; `chrono 0.4` (clock only) for template context; wit-bindgen 0.60 guests.

**Spec:** `docs/superpowers/specs/2026-08-24-extensions-phase5-design.md`

## Global Constraints

- Plugin failures are data: viewer failure → source editor; widget failure → absent; template failure → error strip; hook failure → save unchanged. All calls under the epoch deadline, wasm never on the render/keystroke path.
- Fixture binding coverage stays complete: panic/hang/reader = v1, echo = v2, fetcher = v3, NEW `probe` = v4.
- Template writes are workspace-only, validated by the Phase 3 path rules, idempotent (exists → open).
- Hooks are NOT gated by `format_on_save`; they run after it, chained in load order, each generation-guarded.
- Only hook event `"save"` is understood; unknown events reject the plugin at load.
- Widget refresh: debounced ~500 ms after edits + on open; zero cost when no widget plugins are loaded.
- Fixture tests skip with an eprintln when fixtures are absent (existing pattern).

---

### Task 1: Manifest + contribution tables

**Files:**
- Modify: `src/extensions.rs`

**Interfaces (produces):**
- Manifest structs: `ViewerRule { extensions: Vec<String> }`, `WidgetRule { id: String }`, `TemplateRule { id: String, name: String }`; `PluginMeta` gains `viewers: Vec<ViewerRule>`, `widgets: Vec<WidgetRule>`, `templates: Vec<TemplateRule>`, `hooks: Vec<String>`.
- Tables (RwLock statics, rebuilt in `set_surface_tables`): `pub fn viewer_for_extension(ext: &str) -> Option<String>` (first plugin wins, load order), `pub fn widget_plugins() -> Vec<String>`, `pub fn template_entries() -> Vec<(String, String, String)>` (plugin, id, name), `pub fn hook_plugins() -> Vec<String>`.
- `needs_component` returns true when any of the four is non-empty.

- [ ] **Step 1: Failing tests** (manifest_tests)

```rust
    #[test]
    fn phase5_surfaces_parse_and_fill_tables() {
        let m = parse_manifest(
            Path::new("/p/x"),
            r#"
name = "p5"
version = "0"
hooks = ["save"]
[[viewers]]
extensions = ["csv", "tsv"]
[[widgets]]
id = "words"
[[templates]]
id = "daily"
name = "Daily Note"
"#,
        )
        .unwrap();
        assert_eq!(m.viewers[0].extensions, ["csv", "tsv"]);
        assert_eq!(m.widgets[0].id, "words");
        assert_eq!(m.templates[0].name, "Daily Note");
        assert_eq!(m.hooks, ["save"]);
        let other = parse_manifest(
            Path::new("/p/y"),
            "name=\"y\"\nversion=\"0\"\n[[viewers]]\nextensions=[\"csv\"]\n",
        )
        .unwrap();
        set_surface_tables(&[m, other]);
        assert_eq!(viewer_for_extension("csv"), Some("p5".to_string())); // first wins
        assert_eq!(viewer_for_extension("png"), None);
        assert_eq!(widget_plugins(), ["p5"]);
        assert_eq!(template_entries(), [("p5".to_string(), "daily".to_string(), "Daily Note".to_string())]);
        assert_eq!(hook_plugins(), ["p5"]);
    }

    #[test]
    fn unknown_hook_event_rejected() {
        let err = parse_manifest(
            Path::new("/p/x"),
            "name=\"x\"\nversion=\"0\"\nhooks=[\"open\"]\n",
        )
        .unwrap_err();
        assert!(err.contains("open"), "{err}");
    }

    #[test]
    fn phase5_surfaces_imply_component() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("w");
        std::fs::create_dir_all(&p).unwrap();
        std::fs::write(p.join("plugin.toml"), "name=\"w\"\nversion=\"0\"\n[[widgets]]\nid=\"x\"\n").unwrap();
        // no plugin.wasm
        let (ok, fail) = discover(dir.path());
        assert!(ok.is_empty());
        assert!(fail[0].1.contains("plugin.wasm"), "{}", fail[0].1);
    }
```

- [ ] **Step 2: Verify FAIL** (`unknown field viewers`).

- [ ] **Step 3: Implement** — structs with `#[derive(Clone, Debug, serde::Deserialize)]`; ManifestFile fields with `#[serde(default)]`; in `parse_manifest` validate `for h in &file.hooks { if h != "save" { return Err(format!("unknown hook event `{h}` (known: save)")); } }`; copy fields through to PluginMeta; extend `needs_component`:

```rust
        || !meta.viewers.is_empty()
        || !meta.widgets.is_empty()
        || !meta.templates.is_empty()
        || !meta.hooks.is_empty()
```

New statics + accessors alongside ENRICH_PLUGINS, filled in `set_surface_tables`:

```rust
static VIEWER_TABLE: std::sync::RwLock<Vec<(String, String)>> = std::sync::RwLock::new(Vec::new()); // (ext, plugin)
static WIDGET_PLUGINS: std::sync::RwLock<Vec<String>> = std::sync::RwLock::new(Vec::new());
static TEMPLATE_ENTRIES: std::sync::RwLock<Vec<(String, String, String)>> = std::sync::RwLock::new(Vec::new());
static HOOK_PLUGINS: std::sync::RwLock<Vec<String>> = std::sync::RwLock::new(Vec::new());
```

Viewer table: iterate metas in order, push (ext, plugin) only if ext not already present (first wins).

- [ ] **Step 4: GREEN + full suite; commit** — "feat: manifest surfaces for viewers, widgets, templates, hooks".

---

### Task 2: WIT 0.4 world, Bound::V4, probe fixture, host methods

**Files:**
- Create: `plugins/wit-v4/extension.wit`
- Create: `plugins/fixtures/probe/{Cargo.toml,src/lib.rs,plugin.toml}`
- Modify: `scripts/build_plugins.sh` (probe in fixtures list)
- Modify: `src/extensions.rs` (v4 bindgen, Bound::V4, fallback, 4 new methods, V4 arms on all existing surfaces)

**Interfaces (produces):**
- `ExtensionHost::render_view(plugin, filename, content) -> Result<String, String>`
- `ExtensionHost::status_text(plugin, document) -> Result<String, String>`
- `pub struct TemplateContext { pub date: String, pub time: String, pub weekday: String, pub workspace: String }`
- `ExtensionHost::render_template(plugin, id, ctx: &TemplateContext) -> Result<(String, String), String>` ((filename, content))
- `ExtensionHost::on_save(plugin, path, document) -> Result<Option<String>, String>`

- [ ] **Step 1: WIT** — `plugins/wit-v4/extension.wit`: copy wit-v3 verbatim (package line → `@0.4.0`), keep `types`, `host-api`, the 0.3 world contents, and append inside the world:

```wit
    /// Render a non-markdown file to MARKDOWN for the Reader.
    export render-view: func(filename: string, content: string)
        -> result<string, string>;

    /// One-line status text for the active document.
    export status-text: func(document: string)
        -> result<string, string>;

    record template-context {
        date: string,
        time: string,
        weekday: string,
        workspace: string,
    }
    record template-file { filename: string, content: string }
    /// Materialize a template ("New: Daily Note").
    export render-template: func(id: string, context: template-context)
        -> result<template-file, string>;

    /// Pre-save transform; none = save unchanged.
    export on-save: func(path: string, document: string)
        -> result<option<string>, string>;
```

- [ ] **Step 2: probe fixture** (v4; echo-style deterministic responses). Cargo.toml like fetcher's (name "probe"). lib.rs: `generate!({ path: "../../wit-v4", world: "extension" })`; old surfaces return `Err("unused")` except `process_paste → Ok(None)` and `format_document → Ok(d)`; new surfaces:

```rust
    fn render_view(filename: String, content: String) -> Result<String, String> {
        if content.contains("fail") {
            return Err("cannot view".into());
        }
        Ok(format!("# view:{filename}\n\n{content}\n"))
    }
    fn status_text(document: String) -> Result<String, String> {
        Ok(format!("status:{}", document.len()))
    }
    fn render_template(id: String, context: t::TemplateContext) -> Result<t::TemplateFile, String> {
        Ok(t::TemplateFile {
            filename: format!("from-template/{id}-{}.md", context.date),
            content: format!("# {id} on {} ({})\nws={}\n", context.date, context.weekday, context.workspace),
        })
    }
    fn on_save(_path: String, document: String) -> Result<Option<String>, String> {
        if document.contains("hookme") {
            Ok(Some(format!("{document}\n<!-- saved -->\n")))
        } else {
            Ok(None)
        }
    }
```

(`t::TemplateContext`/`t::TemplateFile` paths: the records are world-level, so likely at crate root like `ExportFile` — follow the compiler; the `t::` prefix above is best-guess.) plugin.toml:

```toml
name = "probe"
version = "0.1.0"
formats = true
hooks = ["save"]
[[viewers]]
extensions = ["prb"]
[[widgets]]
id = "len"
[[templates]]
id = "note"
name = "Probe Note"
```

Add `probe` to the fixtures CRATES list in build_plugins.sh; run `bash scripts/build_plugins.sh --fixtures`.

- [ ] **Step 3: Failing host tests**

```rust
    #[test]
    fn v4_surfaces_roundtrip_and_older_get_readable_errors() {
        let Some(dir) = fixtures_dir() else { eprintln!("SKIP"); return; };
        let mut host = ExtensionHost::load(&dir);
        assert_eq!(
            host.render_view("probe", "a.prb", "body").unwrap(),
            "# view:a.prb\n\nbody\n"
        );
        assert!(host.render_view("probe", "a.prb", "fail here").is_err());
        assert_eq!(host.status_text("probe", "12345").unwrap(), "status:5");
        let ctx = TemplateContext {
            date: "2026-08-24".into(),
            time: "16:00".into(),
            weekday: "Monday".into(),
            workspace: "notes".into(),
        };
        let (filename, content) = host.render_template("probe", "note", &ctx).unwrap();
        assert_eq!(filename, "from-template/note-2026-08-24.md");
        assert!(content.contains("Monday") && content.contains("ws=notes"), "{content}");
        assert_eq!(host.on_save("probe", "d.md", "plain").unwrap(), None);
        assert_eq!(
            host.on_save("probe", "d.md", "hookme").unwrap(),
            Some("hookme\n<!-- saved -->\n".to_string())
        );
        // old worlds err readably on the new surfaces
        let e = host.status_text("panic", "x").unwrap_err();
        assert!(e.contains("0.4"), "{e}");
        // and probe's old surfaces still work through the V4 binding
        assert_eq!(host.format_document("probe", "abc").unwrap(), "abc");
    }
```

- [ ] **Step 4: Implement host side** — `mod v4 { bindgen!({ path: "plugins/wit-v4/extension.wit", ... }) }`; `Bound::V4` variant; `ensure_bound` tries v4 first (fresh store per attempt, like v3): v4 → v3 → v2 → v1. Link BOTH host-api versions on the shared linker (v4's world imports `supermd:extension/host-api@0.4.0` — a separate interface identity from 0.3's; add `v4::supermd::extension::host_api::add_to_linker::<HostState, HasSelf<_>>(...)` and implement `v4::...::host_api::Host for HostState` by delegating to the same fetch logic — extract the ladder body into a free fn `fn host_fetch(state: &mut HostState, method, url, headers, body) -> Result<(u16, Vec<(String,String)>, Vec<u8>), String>` so both impls are 5-line shims). All five existing call surfaces gain `Bound::V4` arms (mirror V3; types under `v4::supermd::extension::types`). New methods follow the established shape, e.g.:

```rust
    /// 0.4-only. Blocking; call from the background executor only.
    pub fn render_view(&mut self, plugin: &str, filename: &str, content: &str) -> Result<String, String> {
        self.with_instance(plugin, |bound| match bound {
            Bound::V4(store, i) => i.call_render_view(store, filename, content),
            _ => Ok(Err("requires a 0.4 plugin".to_string())),
        })?
        .map_err(|e| e)
    }
```

(same pattern for `status_text`; `render_template` converts to/from the generated record; `on_save` returns the option through.)

- [ ] **Step 5: GREEN + full suite; commit** — "feat: wit 0.4 world with viewer/widget/template/hook surfaces".

---

### Task 3: Save-hook chain in the flush path + toc hook

**Files:**
- Modify: `src/editor/mod.rs` (`run_save_hooks` called from `flush` after `maybe_format_before_save`)
- Modify: `plugins/toc/` (wit path → `../wit-v4`, stubs for new surfaces, `on_save` refreshing markers, manifest `hooks = ["save"]`)

- [ ] **Step 1: Failing test** — hook logic is a thin chain over host calls; the pure part worth testing is chain semantics. Extract a free fn in `editor/mod.rs`:

```rust
/// Run the save-hook chain: each plugin sees the previous result;
/// Err/None leave the text unchanged for the next.
fn chain_save_hooks(
    text: String,
    path: &str,
    plugins: &[String],
    mut call: impl FnMut(&str, &str, &str) -> Result<Option<String>, String>,
) -> String {
    plugins.iter().fold(text, |acc, plugin| {
        match call(plugin, path, &acc) {
            Ok(Some(next)) => next,
            _ => acc,
        }
    })
}
```

Test (new `mod hook_tests` in editor/mod.rs):

```rust
    #[test]
    fn hooks_chain_in_order_and_skip_failures() {
        let plugins = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let out = chain_save_hooks("x".into(), "f.md", &plugins, |p, _, doc| match p {
            "a" => Ok(Some(format!("{doc}a"))),
            "b" => Err("boom".into()),
            _ => Ok(Some(format!("{doc}c"))),
        });
        assert_eq!(out, "xac");
    }
```

Write test first (FAIL: missing fn), implement, PASS.

- [ ] **Step 2: Wire into flush.** In `flush`, after `self.maybe_format_before_save(cx);`:

```rust
        self.run_save_hooks(cx);
```

```rust
    /// Always-on pre-save transforms (hooks = ["save"]), after the
    /// optional formatter, each generation-guarded.
    fn run_save_hooks(&mut self, cx: &mut Context<Self>) {
        let plugins = crate::extensions::hook_plugins();
        if plugins.is_empty() {
            return;
        }
        let Some(state) = cx.try_global::<crate::extensions::ExtensionState>() else {
            return;
        };
        let snapshot = self.core.buffer.text();
        let path = self.path.to_string_lossy().into_owned();
        let result = chain_save_hooks(snapshot.clone(), &path, &plugins, |p, path, doc| {
            state.0.lock().unwrap().on_save(p, path, doc)
        });
        if result != snapshot {
            // Same synchronous window as the formatter: the buffer
            // cannot move between snapshot and apply.
            self.apply_command_output(
                &crate::extensions::CommandOutput::ReplaceDocument(result),
                cx,
            );
        }
    }
```

(The flush path is synchronous on the main thread like `maybe_format_before_save`, so the generation guard is the unchanged-buffer window itself; note this in the code comment as done above.)

- [ ] **Step 3: toc plugin** — `plugins/toc/src/lib.rs`: wit path → `../wit-v4`; add stubs (`render_view`/`status_text`/`render_template` → `Err("unused")`) and:

```rust
    fn on_save(_path: String, document: String) -> Result<Option<String>, String> {
        Ok(update_between_markers(&document)) // None when no markers
    }
```

(Adapt to toc's actual internal fn signature — read `plugins/toc/src/lib.rs` first; its updater may return `String` + a changed flag; wrap so no-markers → None.) Manifest gains `hooks = ["save"]`. In-crate test: document with markers + stale toc → on_save Some(refreshed); without markers → None. Rebuild toc (`bash scripts/build_plugins.sh`).

- [ ] **Step 4: Full suite; commit** — "feat: save-hook chain; toc auto-refreshes its markers on save".

---

### Task 4: Templates — palette entries, context, materialization

**Files:**
- Modify: `src/extensions.rs` (`materialize_template` pure helper)
- Modify: `src/workspace.rs` (palette entries, `__template:` branch)
- Modify: `Cargo.toml` (`chrono = { version = "0.4", default-features = false, features = ["clock"] }`)

**Interfaces (produces):**
- `pub fn materialize_template(root: &Path, filename: &str, content: &str) -> Result<(PathBuf, bool), String>` — validates the relative path (reuses `validate_export_paths` rules), returns (absolute path, created) where created=false means it already existed (content untouched).
- `pub fn template_context(workspace: &str) -> TemplateContext` — chrono local now: `date` = `%Y-%m-%d`, `time` = `%H:%M`, `weekday` = `%A`.

- [ ] **Step 1: Failing tests** (extensions.rs, `mod template_tests`)

```rust
    #[test]
    fn materialize_validates_creates_and_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        assert!(materialize_template(dir.path(), "../evil.md", "x").is_err());
        assert!(materialize_template(dir.path(), "/abs.md", "x").is_err());
        let (path, created) =
            materialize_template(dir.path(), "journal/day.md", "# hi\n").unwrap();
        assert!(created);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "# hi\n");
        let (path2, created2) =
            materialize_template(dir.path(), "journal/day.md", "OVERWRITE").unwrap();
        assert_eq!(path, path2);
        assert!(!created2);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "# hi\n", "never overwrites");
    }

    #[test]
    fn template_context_shapes() {
        let ctx = template_context("notes");
        assert_eq!(ctx.workspace, "notes");
        assert_eq!(ctx.date.len(), 10, "{}", ctx.date); // YYYY-MM-DD
        assert!(ctx.date.chars().nth(4) == Some('-'));
        assert_eq!(ctx.time.len(), 5); // HH:MM
        assert!(!ctx.weekday.is_empty());
    }
```

- [ ] **Step 2: Verify FAIL, implement**

```rust
pub fn materialize_template(
    root: &Path,
    filename: &str,
    content: &str,
) -> Result<(PathBuf, bool), String> {
    validate_export_paths(&[(filename.to_string(), Vec::new())])?;
    let target = root.join(filename);
    if target.exists() {
        return Ok((target, false));
    }
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(&target, content).map_err(|e| e.to_string())?;
    Ok((target, true))
}

pub fn template_context(workspace: &str) -> TemplateContext {
    let now = chrono::Local::now();
    TemplateContext {
        date: now.format("%Y-%m-%d").to_string(),
        time: now.format("%H:%M").to_string(),
        weekday: now.format("%A").to_string(),
        workspace: workspace.to_string(),
    }
}
```

- [ ] **Step 3: Workspace wiring.** In `toggle_palette`, after the exports loop:

```rust
                for (plugin, id, name) in crate::extensions::template_entries() {
                    entries.push(crate::palette::PaletteEntry {
                        plugin,
                        id: format!("__template:{id}"),
                        title: format!("New: {name}"),
                    });
                }
```

In `run_plugin_command`, BEFORE the editable-tab guard (templates need no editor; move the branch above the `let Some(Tab::Editor …) else` block — the guard stays for everything else):

```rust
        if let Some(template_id) = id.strip_prefix("__template:") {
            let Some(root) = self.tree.as_ref().map(|t| t.root.clone()) else {
                self.show_command_error("Open a folder to use templates".into(), cx);
                return;
            };
            let Some(state) = cx.try_global::<crate::extensions::ExtensionState>() else {
                return;
            };
            let host = state.0.clone();
            let ctx_data = crate::extensions::template_context(
                &root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
            );
            let template_id = template_id.to_string();
            let run = cx.background_executor().spawn(async move {
                host.lock().unwrap().render_template(&plugin, &template_id, &ctx_data)
            });
            cx.spawn_in(window, async move |this, cx| {
                let result = run.await;
                this.update_in(cx, |this, window, cx| match result {
                    Ok((filename, content)) => {
                        match crate::extensions::materialize_template(&root, &filename, &content) {
                            Ok((path, _created)) => {
                                if let Some(tree) = &mut this.tree {
                                    tree.refresh();
                                }
                                this.open_path(&path, window, cx);
                            }
                            Err(e) => this.show_command_error(e, cx),
                        }
                    }
                    Err(e) => this.handle_plugin_error(plugin.clone(), e, cx),
                })
                .ok();
            })
            .detach();
            return;
        }
```

(`run_plugin_command` currently takes `_window: &mut Window` — rename to `window` for `cx.spawn_in`. `tree.refresh()` + `open_path` signatures: mirror the drop-folder path at workspace.rs:745. The `plugin` variable is moved into the closure; clone as the existing branches do.)

- [ ] **Step 4: Full suite + build; commit** — "feat: template surface with palette-driven workspace file creation".

---

### Task 5: Status strip + word-count plugin

**Files:**
- Modify: `src/editor/mod.rs` (`status_text` state + debounced task)
- Modify: `src/workspace.rs` (strip render)
- Create: `plugins/word-count/{Cargo.toml,src/lib.rs,plugin.toml}` (cdylib+rlib for tests)
- Modify: `scripts/build_plugins.sh` (dist list)

- [ ] **Step 1: word-count plugin with in-crate failing tests first**

```rust
#[cfg(test)]
mod tests {
    use super::word_stats;

    #[test]
    fn counts_words_outside_fences() {
        assert_eq!(word_stats("one two three"), "3 words · 1 min read");
        assert_eq!(word_stats("a b\n```\ncode words here\n```\nc"), "3 words · 1 min read");
        assert_eq!(word_stats(""), "0 words · 1 min read");
        // 401 words → 3 min
        let long = "w ".repeat(401);
        assert!(word_stats(&long).starts_with("401 words · 3 min"));
    }
}
```

Implementation: `pub fn word_stats(document: &str) -> String` — iterate lines, toggle in-fence on ``` prefix, count whitespace-split words outside fences; minutes = `(words + 199) / 200` clamped min 1; format with thousands? No — plain number (keep simple). `status_text` export returns `Ok(word_stats(&document))`; all other exports `Err("unused")`/passthroughs; wit path `../wit-v4`; manifest:

```toml
name = "word-count"
version = "0.1.0"
[[widgets]]
id = "words"
```

- [ ] **Step 2: Editor state.** Fields: `status_text: Option<SharedString>`, `status_task: Option<gpui::Task<()>>` (init None in from_text). Public accessor `pub fn status(&self) -> Option<SharedString>`. Debounced refresh:

```rust
    /// Debounced status-widget refresh (500ms after last edit).
    fn schedule_status(&mut self, cx: &mut Context<Self>) {
        if crate::extensions::widget_plugins().is_empty() {
            return;
        }
        let Some(state) = cx.try_global::<crate::extensions::ExtensionState>() else {
            return;
        };
        let host = state.0.clone();
        self.status_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(500))
                .await;
            let Ok(document) = this.update(cx, |this, _| this.core.buffer.text()) else {
                return;
            };
            let text = cx
                .background_executor()
                .spawn(async move {
                    let mut host = host.lock().unwrap();
                    let parts: Vec<String> = crate::extensions::widget_plugins()
                        .iter()
                        .filter_map(|p| host.status_text(p, &document).ok())
                        .collect();
                    parts.join(" · ")
                })
                .await;
            this.update(cx, |this, cx| {
                this.status_text = (!text.is_empty()).then(|| text.into());
                cx.notify();
            })
            .ok();
        }));
    }
```

Call `self.schedule_status(cx)` at the end of `after_edit` and once at the end of `from_text`... `from_text` returns Self before entity exists — cx.spawn needs the entity; from_text's `cx: &mut Context<Self>` supports spawn before construction completes? `cx.spawn` on Context<Self> uses weak handle — fine inside `cx.new` closure. If the compiler objects, call it from the first `after_edit` only and accept no status until first edit + tab switch — but first try in from_text. (Replacing an in-flight `status_task` drops/cancels the previous timer — that IS the debounce.)

- [ ] **Step 3: Strip render.** In the workspace's editor-pane render (near the `command_error` strip / active-tab element around workspace.rs:2378), when the active tab is `Tab::Editor { view: EditorView::Edit, editor }` and `editor.read(cx).status()` is Some, render an absolute-positioned bottom-right strip: muted small text (`text_size(px(11.))`, `text_color(t.muted)`, padding, `bg(t.panel_bg)` rounded), non-interactive. Mirror the command_error strip's styling/placement idioms.

- [ ] **Step 4: Host-level test** (probe fixture has widget `len`): none needed beyond Task 2's roundtrip — the strip is GPUI plumbing. Full suite + build; commit — "feat: status strip with widget plugins; word-count plugin".

---

### Task 6: Viewers — open-as-rendered, ⌘E toggle, csv-view plugin

**Files:**
- Modify: `src/workspace.rs` (`open_path` viewer branch, `toggle_preview` viewer branch, shared `spawn_viewer_render` helper)
- Create: `plugins/csv-view/{Cargo.toml,src/lib.rs,plugin.toml}` (cdylib+rlib)
- Modify: `scripts/build_plugins.sh` (dist list)

- [ ] **Step 1: csv-view plugin, in-crate failing tests first**

```rust
#[cfg(test)]
mod tests {
    use super::csv_markdown;

    #[test]
    fn renders_comma_and_tab_tables() {
        let md = csv_markdown("a,b\n1,2\n3,4\n").unwrap();
        assert!(md.contains("| a | b |"), "{md}");
        assert!(md.contains("| --- | --- |"), "{md}");
        assert!(md.contains("| 3 | 4 |"), "{md}");
        let md = csv_markdown("x\ty\n1\t2\n").unwrap();
        assert!(md.contains("| x | y |"), "{md}");
    }

    #[test]
    fn escapes_pipes_and_caps_rows() {
        let md = csv_markdown("h1,h2\na|b,c\n").unwrap();
        assert!(md.contains("a\\|b"), "{md}");
        let big: String =
            std::iter::once("h,i".to_string())
                .chain((0..600).map(|n| format!("{n},{n}")))
                .collect::<Vec<_>>()
                .join("\n");
        let md = csv_markdown(&big).unwrap();
        assert!(md.contains("… 100 more rows"), "{md}");
        assert!(!md.contains("| 599 |"), "{md}");
    }

    #[test]
    fn rejects_non_tabular() {
        assert!(csv_markdown("just some prose without delimiters").is_err());
        assert!(csv_markdown("").is_err());
    }
}
```

Implementation `pub fn csv_markdown(content: &str) -> Result<String, String>`: pick delimiter — '\t' if the first non-empty line contains one, else ','; error when the first line has < 2 fields; split all non-empty lines, escape `|` → `\|`, first row = header, `| --- |` separator per column, cap 500 data rows with `\n… N more rows\n` tail. Ragged rows: pad short, truncate long to header width. `render_view` export: `csv_markdown(&content)` (filename unused); other exports stubbed; manifest:

```toml
name = "csv-view"
version = "0.1.0"
[[viewers]]
extensions = ["csv", "tsv"]
```

- [ ] **Step 2: Workspace open flow.** In `open_path`, after the editor tab is pushed (the ~line 715 site), add:

```rust
                if let Some(viewer) = path
                    .extension()
                    .and_then(|e| e.to_str())
                    .and_then(crate::extensions::viewer_for_extension)
                {
                    self.spawn_viewer_render(viewer, self.tabs.len() - 1, cx);
                }
```

The shared helper (used by open and toggle):

```rust
    /// Render the active editor's file through its viewer plugin and
    /// swap the tab to Preview when done. Failure leaves the source
    /// editor — a broken viewer never hides a file.
    fn spawn_viewer_render(&mut self, plugin: String, tab_ix: usize, cx: &mut Context<Self>) {
        let Some(Tab::Editor { editor, .. }) = self.tabs.get(tab_ix) else { return };
        let editor = editor.clone();
        let Some(state) = cx.try_global::<crate::extensions::ExtensionState>() else { return };
        let host = state.0.clone();
        let filename = editor.read(cx).title().to_string();
        let content = editor.read(cx).text();
        let run = cx.background_executor().spawn(async move {
            host.lock().unwrap().render_view(&plugin, &filename, &content)
        });
        cx.spawn(async move |this, cx| {
            let result = run.await;
            if let Ok(markdown) = result {
                this.update(cx, |this, cx| {
                    let langs = languages(cx);
                    let title = editor.read(cx).title();
                    // Only swap if that tab still shows this editor in Edit view.
                    if let Some(Tab::Editor { editor: e, view }) = this.tabs.get_mut(tab_ix) {
                        if *e == editor && matches!(view, EditorView::Edit) {
                            let reader = cx.new(|_| Reader::from_source(title, &markdown, &langs));
                            *view = EditorView::Preview(reader);
                            cx.notify();
                        }
                    }
                })
                .ok();
            }
        })
        .detach();
    }
```

(`*e == editor`: Entity implements PartialEq; `editor.read` inside `this.update` closure — take title/langs before mutable borrow of tabs; reorder per compiler. Viewer render on OPEN swaps only from Edit — if the user already toggled manually, leave them be.)

- [ ] **Step 3: ⌘E toggle.** In `toggle_preview`'s else-branch (Edit → Preview): if the file's extension has a viewer, call `self.spawn_viewer_render(viewer, self.active, cx)` INSTEAD of building `Reader::from_source` from raw text (flush first, as the current code does). Preview → Edit direction unchanged. Toggling back to preview re-renders (fresh call each time — edits show).

- [ ] **Step 4: Full suite + build all plugins; commit** — "feat: custom viewers render files to markdown; csv-view plugin".

---

### Task 7: Packaging, docs, smoke, wrap-up

**Files:**
- Modify: `scripts/build_plugins.sh` (dist CRATES gains `word-count csv-view daily-note`)
- Create: `plugins/daily-note/{Cargo.toml,src/lib.rs,plugin.toml}`
- Modify: `plugins/template/{src/lib.rs,README.md}` (wit-v4, new stubs, surfaces documented)

- [ ] **Step 1: daily-note plugin** (in-crate test first):

```rust
#[cfg(test)]
mod tests {
    #[test]
    fn renders_dated_note() {
        let f = super::daily(&super::Ctx {
            date: "2026-08-24".into(),
            weekday: "Monday".into(),
        });
        assert_eq!(f.0, "journal/2026-08-24.md");
        assert!(f.1.starts_with("# Monday, 2026-08-24\n"), "{}", f.1);
        assert!(f.1.contains("- [ ] "), "{}", f.1);
    }
}
```

(`Ctx` = a tiny local struct so the pure fn is testable without wit types; `render_template` maps the wit context into it.) Manifest:

```toml
name = "daily-note"
version = "0.1.0"
[[templates]]
id = "daily"
name = "Daily Note"
```

Content: `# <weekday>, <date>\n\n## Today\n\n- [ ] \n\n## Notes\n\n`.

- [ ] **Step 2: Template crate refresh** — `plugins/template/` wit path → `../wit-v4`, stubs for the four new exports, README section "Viewers, widgets, templates, hooks" documenting the manifest shapes and the honest limits (viewers are read-only markdown projections; widgets text-only; hook event `save` only).

- [ ] **Step 3: Build everything + full suite** — `bash scripts/build_plugins.sh && bash scripts/build_plugins.sh --fixtures && cargo test && cargo build`.

- [ ] **Step 4: Smoke test** (macOS, dev build; `pkill -f supermd` first, launch on a scratch workspace):
  1. Copy dist plugins to `~/.supermd/plugins/`.
  2. Open a folder; palette → "New: Daily Note" → journal file created + opened; run again → same file opens (no overwrite).
  3. Type in a markdown doc → status strip shows "N words · M min read" after ~½ s.
  4. Open a `.csv` → renders as a table; ⌘E → raw source; ⌘E → table again.
  5. Add `<!-- toc --><!-- /toc -->` + headings to a doc, save (⌘S) → toc fills in.
  Screenshot the csv view and the status strip.

- [ ] **Step 5: Commit + push** — "feat: daily-note plugin, template 0.4 refresh, phase 5 wrap"; push branch.

## Self-Review Notes

- Spec coverage: manifest+tables ✔ (T1), WIT 0.4 + fixture coverage map ✔ (T2, `probe` keeps V2 proof via echo untouched), hook chain + toc ✔ (T3 — chain order and skip-on-failure unit-tested; the sync-window note replaces a separate generation-guard test because flush is single-threaded, matching the existing formatter's guarantee), templates ✔ (T4 — idempotence and traversal tests), widgets ✔ (T5), viewers ✔ (T6 — fallback = no swap on Err; toggle re-renders), first-party ✔ (T5/T6/T7 + toc), packaging/docs ✔ (T7).
- Type consistency: `TemplateContext` host struct (T2) consumed by `template_context` (T4); `render_template` returns `(String, String)` consumed by `materialize_template(root, &filename, &content)`; `viewer_for_extension` (T1) consumed in T6; `hook_plugins` (T1) consumed in T3; `widget_plugins` (T1) consumed in T5.
- Compiler-guided points flagged inline: generated record paths in the probe fixture, `run_plugin_command` window param rename, borrow ordering in `spawn_viewer_render`, `schedule_status` from `from_text`.
- Palette `__template:` branch placement above the editable-tab guard is called out explicitly (templates need no open editor).
