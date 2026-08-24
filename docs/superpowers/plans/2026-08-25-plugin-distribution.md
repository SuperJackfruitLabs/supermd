# Plugin Distribution Implementation Plan — Seeding + On-Demand Install

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fresh installs get the eight default plugins seeded on first run; the "Install Plugins…" palette flow downloads catalog plugins from GitHub, validates, and hot-installs them.

**Architecture:** `src/seeding.rs` (pure `plan_seeding` + marker + hash, wired into startup before `ExtensionHost::load`); `src/catalog.rs` (parse, org-pin, drift test); install flow (ureq download behind an injectable fetcher, `zip` crate extraction, temp-dir validation, move + reload); an `InstallOverlay` in the palette family; installers gain a default-plugins payload staged by `build_plugins.sh`.

**Tech Stack:** existing ureq; new deps `sha2 = "0.10"`, `zip` (deflate only).

**Spec:** `docs/superpowers/specs/2026-08-25-plugin-distribution-design.md`

## Global Constraints

- Default set (payload + seeding): `dot toc emoji tidy todo-marks word-count csv-view graphql`. Catalog lists all eleven first-party plugins (installed ones show ✓, enabling reinstall of deleted defaults).
- Seeding failures degrade to eprintln + no seeding; install failures leave `~/.supermd/plugins` untouched.
- Download URLs must start with `https://github.com/SuperJackfruitLabs/supermd/` or `https://raw.githubusercontent.com/SuperJackfruitLabs/supermd/`; 20 MB cap; 30 s timeout; sha256 verified before extraction.
- Zip validation before install: single top-level plugin dir named as cataloged, no traversal entries, manifest parses, required files present (via `needs_component`/`grammar_paths`).
- Dev runs (executable under `target/`) seed nothing (`bundled_plugins_dir` → None).
- All tests follow existing patterns (pure fns first; gpui suites with injectable I/O; table-mutating tests take `table_test_guard`).

---

### Task 1: Seeding (pure logic + startup wiring)

**Files:**
- Create: `src/seeding.rs`; Modify: `src/main.rs` (mod + startup call), `src/platform.rs` (`bundled_plugins_dir`)

**Interfaces (produces):**
- `pub struct SeededMarker { pub entries: Vec<SeededEntry> }`, `SeededEntry { name, version, hash }` (TOML round-trip via serde)
- `pub fn content_hash(dir: &Path) -> String` — sha256 over sorted (relative path, bytes) of all files
- `pub enum SeedAction { Install(String), Refresh(String) }`
- `pub fn plan_seeding(bundled: &[(String, String, String)], installed: &[(String, String)], marker: &SeededMarker) -> Vec<SeedAction>`
  — bundled = (name, version, hash); installed = (name, current content hash)
- `pub fn run_seeding(bundled_dir: &Path, plugins_dir: &Path)` — thin I/O: read manifests/hashes, plan, copy dirs, rewrite marker (`seeded.toml` in plugins_dir)
- `platform::bundled_plugins_dir() -> Option<PathBuf>` — probes `../Resources/plugins`, `../lib/supermd/plugins`, `./plugins` relative to `current_exe`; None when the exe path contains `target` (dev)

- [ ] **Step 1: Failing tests** (`mod tests` in seeding.rs) — the truth table:

```rust
    fn b(n: &str, v: &str, h: &str) -> (String, String, String) { (n.into(), v.into(), h.into()) }
    fn marker(entries: &[(&str, &str, &str)]) -> SeededMarker { /* build */ }

    #[test]
    fn fresh_install_seeds_everything() {
        let plan = plan_seeding(&[b("dot","1","h1"), b("toc","1","h2")], &[], &SeededMarker::default());
        assert_eq!(plan, vec![SeedAction::Install("dot".into()), SeedAction::Install("toc".into())]);
    }
    #[test]
    fn deleted_seeded_plugin_never_returns() {
        let m = marker(&[("dot","1","h1")]);
        assert!(plan_seeding(&[b("dot","1","h1")], &[], &m).is_empty());
    }
    #[test]
    fn untouched_plugin_refreshes_on_newer_bundled_version() {
        let m = marker(&[("dot","1","h1")]);
        // installed hash == marker hash (user never touched), bundled newer
        let plan = plan_seeding(&[b("dot","2","h9")], &[("dot".into(),"h1".into())], &m);
        assert_eq!(plan, vec![SeedAction::Refresh("dot".into())]);
        // same version → nothing
        assert!(plan_seeding(&[b("dot","1","h1")], &[("dot".into(),"h1".into())], &m).is_empty());
    }
    #[test]
    fn user_modified_plugin_is_never_touched() {
        let m = marker(&[("dot","1","h1")]);
        assert!(plan_seeding(&[b("dot","2","h9")], &[("dot".into(),"hX".into())], &m).is_empty());
    }
    #[test]
    fn unseeded_preexisting_plugin_is_left_alone() {
        // user installed "dot" manually before ever seeing seeding
        assert!(plan_seeding(&[b("dot","1","h1")], &[("dot".into(),"hX".into())], &SeededMarker::default()).is_empty());
    }
    #[test]
    fn marker_roundtrips_and_content_hash_is_stable() { /* toml round-trip; hash of a temp dir stable across calls, changes when a file changes */ }
    #[test]
    fn run_seeding_end_to_end() { /* temp bundled dir with 2 fake plugins + temp plugins dir: run → both appear + marker written; delete one + rerun → stays gone */ }
```

Version comparison: lexicographic on `(split '.' as u32 triples)` — helper `fn newer(a: &str, b: &str) -> bool` with its own test (10.0 > 9.0).

- [ ] **Step 2:** RED (`cargo test seeding`) → implement (add `sha2 = "0.10"` to deps) → GREEN.
- [ ] **Step 3:** `bundled_plugins_dir` in platform.rs with temp-dir probing test (build fake layouts; the dev-`target` exclusion asserted).
- [ ] **Step 4:** Wire into main.rs run(), immediately before the ExtensionHost block:

```rust
        if let Some(bundled) = platform::bundled_plugins_dir() {
            seeding::run_seeding(&bundled, &settings::config_dir().join("plugins"));
        }
```

- [ ] **Step 5:** Full suite; commit "feat: first-run plugin seeding from installer payload".

---

### Task 2: Catalog module + catalog.json + drift test

**Files:**
- Create: `src/catalog.rs`, `plugins/catalog.json`; Modify: `src/main.rs` (mod)

**Interfaces:**
- `pub struct CatalogEntry { pub name, description, version: String, pub capabilities: Vec<String>, pub download: String, pub sha256: String }`
- `pub fn parse_catalog(json: &str) -> Result<Vec<CatalogEntry>, String>` (rejects `catalog_version != 1`)
- `pub fn url_allowed(url: &str) -> bool` — org-pinned prefixes
- `pub const CATALOG_URL: &str = "https://raw.githubusercontent.com/SuperJackfruitLabs/supermd/master/plugins/catalog.json"`

- [ ] **Step 1:** Failing tests: parse a two-entry fixture; unknown catalog_version rejected; `url_allowed` accepts both prefixes, rejects `https://github.com/evil/…`, `http://…`, and `https://github.com.evil.com/…` (prefix must be followed by exact org/repo path). Drift test: for every entry in `plugins/catalog.json`, `plugins/<name>/plugin.toml` exists with matching name/version/capabilities; and every dist plugin (`build_plugins.sh` CRATES + graphql) has an entry.
- [ ] **Step 2:** Implement + write `plugins/catalog.json` with all 11 entries; `download` URLs point at `…/releases/download/v0.0.9/plugin-<name>.zip`; `sha256` fields carry `"pending"` until the release workflow patches them (drift test does NOT check sha256; a separate release-time step fills them).
- [ ] **Step 3:** GREEN; commit "feat: plugin catalog with org-pinned download URLs".

---

### Task 3: Download + validate + install flow

**Files:**
- Modify: `src/catalog.rs` (install machinery), `Cargo.toml` (`zip` default-features=false features=["deflate"])

**Interfaces:**
- `pub type Fetcher = std::sync::Arc<dyn Fn(&str) -> Result<Vec<u8>, String> + Send + Sync>` + `pub fn ureq_fetcher() -> Fetcher` (30 s timeout, 20 MB read cap)
- `pub fn validate_plugin_zip(bytes: &[u8], expected_name: &str) -> Result<(), String>` — traversal-free entries, single root dir == expected_name, manifest parses, required files present
- `pub fn install_plugin(entry: &CatalogEntry, plugins_dir: &Path, fetch: &Fetcher) -> Result<(), String>` — url_allowed → fetch → sha256 check → validate → extract to tempdir → `fs::rename`/copy into plugins_dir (error if the name already exists)

- [ ] **Step 1:** Failing tests with in-test-built fixture zips (`zip` crate writer in tests): happy path installs and manifest is readable at the destination; traversal entry (`../x`) rejected; wrong root name rejected; bad manifest rejected; sha mismatch rejected (fetcher returns altered bytes); disallowed URL never calls the fetcher (call-recording mock, like the transport tests); existing destination rejected without touching it.
- [ ] **Step 2:** Implement; GREEN; commit "feat: plugin download, validation, and install".

---

### Task 4: Install Plugins… overlay

**Files:**
- Create: `src/install_ui.rs` (palette-family overlay); Modify: `src/workspace.rs` (action + wiring), `src/main.rs` (mod, keybinding-free — palette command only), `src/palette.rs` (nothing — entry added in workspace's toggle_palette as a synthetic `__install` command)

**Interfaces:**
- `InstallOverlay::new(entries: Vec<CatalogEntry>, installed: Vec<String>, cx)`; events `InstallEvent::{Install(CatalogEntry), Dismissed}`
- Workspace: `"Install Plugins…"` palette entry (`plugin: "supermd"`, `id: "__install"`); handler fetches the catalog on the background executor (via a `Fetcher`), opens the overlay; on `Install`, runs `catalog::install_plugin` in the background, then the reload-plugins flow; strips for success/failure. A `CATALOG_FETCHER` global set in main (ureq) and overridden in tests.

- [ ] **Step 1:** Overlay entity mirroring the Palette (rows: name+description, capability tag in user terms — map `net` → "needs network access — asks per site", `workspace-read` → "reads your open folder — asks first"; installed rows dimmed with ✓, Enter on them no-ops; trust notice line at the bottom). Up/Down/Enter/Escape on the Palette key context pattern (own context "InstallOverlay" + bindings in app_keybindings — remember the count test: bump `assert_eq!(bindings.len(), …)`).
- [ ] **Step 2:** gpui suite (established pattern): catalog fixture → rows listed; installed marker dims + blocks; Enter emits Install with the right entry; Escape dismisses. Workspace-level test: `__install` command with a mock fetcher serving the fixture catalog + a fixture zip → plugin lands in temp-HOME plugins dir and the host reloads (assert `plugins()` contains it). Table-mutating → `table_test_guard`.
- [ ] **Step 3:** GREEN; commit "feat: Install Plugins overlay with in-app catalog install".

---

### Task 5: Packaging + release workflow

**Files:**
- Modify: `scripts/build_plugins.sh` (stage `dist/default-plugins` after the dist build), `scripts/bundle_macos.sh` (copy into `Contents/Resources/plugins` before signing), `scripts/bundle_linux.sh` (tarball `plugins/` + install.sh copies), `Cargo.toml` `[package.metadata.deb]` assets (glob `dist/default-plugins` → `usr/lib/supermd/plugins/`), `scripts/windows/supermd.iss` (recursive `plugins\` source), `.github/workflows/release.yml`

**Steps:**
- [ ] **Step 1:** `build_plugins.sh` (non-fixtures): after the dist loop, `DEFAULTS="dot toc emoji tidy todo-marks word-count csv-view graphql"`; stage copies into `dist/default-plugins/<name>/`.
- [ ] **Step 2:** Bundlers: macOS copies `dist/default-plugins` → `$APP/Contents/Resources/plugins` (before codesign — REQUIRES the macos release job to run `build_plugins.sh` first; add that step with the wasm target); linux tarball adds `plugins/`, `install.sh` copies them next to the binary's expected `../lib/supermd/plugins`… simpler: tarball layout puts `plugins/` beside `supermd` (probe already checks `./plugins`); deb assets glob to `usr/lib/supermd/plugins/` (binary at `usr/bin` → probe `../lib/supermd/plugins` resolves). Windows iss: `Source: "..\\..\\dist\\default-plugins\\*"; DestDir: "{app}\\plugins"; Flags: recursesubdirs` (+ build_plugins step in the windows job).
- [ ] **Step 3:** release.yml linux job additions: per-plugin zips (`for d in dist/plugins/*/; do (cd dist/plugins && zip -r "../plugin-$(basename $d).zip" "$(basename $d)"); done`), sha256 patching of a catalog copy uploaded as a release asset AND a step that fails if committed catalog names ≠ dist plugins (drift honesty at release time; committed sha stays "pending" — the app fetches the catalog from master, so ALSO commit-back? NO — instead: the app fetches the catalog from the RELEASE asset? Decision: keep CATALOG_URL on master raw; add a post-release manual step documented in the plan: run `scripts/update_catalog_hashes.sh <version>` locally (downloads the zips, computes sha256, rewrites plugins/catalog.json, commit+push). Create that script in this step. Simple, auditable, and the drift test keeps it honest.)
- [ ] **Step 4:** Local verification: `bash scripts/build_plugins.sh && bash scripts/bundle_macos.sh 0.0.9-test` → `Contents/Resources/plugins` present + signed; fresh temp HOME + launch dev-built binary pointed at the bundle layout — actually run the INSTALLED bundle binary so `bundled_plugins_dir` probes Resources: launch, verify `~/.supermd/plugins` (temp HOME) gains the 8 defaults; delete one, relaunch, stays gone.
- [ ] **Step 5:** Full suite + commit "feat: default-plugin payloads in all installers; per-plugin release zips".

---

### Task 6: Docs + PR

- [ ] Update `docs/site/plugins.md`: defaults come pre-installed; optional ones via "Install Plugins…" in the palette (screenshots optional); regenerate docs site. Update template README pointer if needed.
- [ ] Manual smoke on this machine (bundle → install → fresh-HOME launch → defaults seeded → Install Plugins… → url-title from a LOCAL fixture… network catalog won't have v0.0.9 zips yet; verify the overlay lists and errors cleanly on "pending" sha).
- [ ] Full suite; push branch; open PR "Plugin distribution: first-run seeding + in-app installs".

## Self-Review Notes

- Spec coverage: seeding truth table ✔ (T1 tests mirror the spec's cases incl. the unseeded-preexisting rule), probing ✔, catalog + org-pin + drift ✔ (T2), download/validate/install ✔ (T3 incl. fetcher-never-called on bad URL), overlay + workspace flow ✔ (T4), installers/workflow ✔ (T5 — macOS/Windows jobs gain plugin builds; deb/tarball layouts match the probe paths), sha256 story resolved concretely (release zips + `update_catalog_hashes.sh` post-release; committed placeholder is "pending" and the app surfaces a clean error if a user installs before the hash commit lands — noted in T6 smoke).
- Keybinding-count test bump called out (T4) — the master CI lesson.
- Types consistent: `Fetcher` shared by catalog fetch and zip download; `CatalogEntry` flows T2→T3→T4.
