# Extensions Phase 3 Implementation Plan — fs-write + net

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Exporters (bytes out, host-owned save dialogs — plugins never see paths) and a host-mediated HTTPS `fetch` import with per-domain consent, plus an async post-paste enrich pass; first-party `html-export` and `url-title` plugins.

**Architecture:** A third WIT world (`supermd:extension@0.3.0`) adds an `export-document` export and the first host **import** (`host-api.fetch`). The host implements fetch with a full enforcement ladder (capability → https → denial → grant → limits) over an injectable transport so tests never touch the network. Export writing and net consent reuse the Phase 2 dialog/banner/grants machinery.

**Tech Stack:** wasmtime 48 component model (triple `bindgen!`), ureq 3 (host transport), wit-bindgen 0.60 + `wasm32-wasip2` guests, pulldown-cmark (in-wasm for html-export).

**Spec:** `docs/superpowers/specs/2026-08-24-extensions-phase3-design.md`

## Global Constraints

- Branch: `extensions-phase1` (continues Phases 1–2; do not merge to master in this plan).
- Plugin failures are data — no plugin can crash or hang the app. Compute deadline stays 2 s (4 epoch ticks × 500 ms).
- Net budget per plugin call: 5 s per fetch, 2 MB response cap, max 4 fetches, https only, redirects only within granted domains.
- `capabilities = ["net"]` grants nothing by itself; every domain needs a persisted `"net:<domain>"` grant in `settings.plugin_grants`.
- fs-write = NO WASI write grant anywhere; exporters return bytes, the host writes to user-picked paths only.
- 0.1 and 0.2 plugins keep working unchanged (fallback chain V3 → V2 → V1).
- Guest crates: wit-bindgen 0.60, `cargo build --target wasm32-wasip2`, empty `[workspace]` table, types at `supermd::extension::types` (no `exports::` prefix).
- Fixture tests skip with an eprintln when `tests/fixtures/plugins/echo/plugin.wasm` is absent (existing pattern).
- Deviation from spec, recorded: the epoch deadline for net-capable calls is pre-budgeted at call entry (compute ticks + max-fetches × fetch-timeout ticks = 4 + 40 = 44 ticks ≈ 22 s) instead of extended inside the fetch import — wasmtime's generated host trait gives `&mut HostState`, not the store, so `set_epoch_deadline` is unreachable mid-call. Same guarantee (slow server ≠ misread hang; transport enforces the real 5 s × 4 network budget), simpler mechanics.

---

### Task 1: Manifest — `net` capability + `[[exports]]`

**Files:**
- Modify: `src/extensions.rs` (ManifestFile, PluginMeta, parse_manifest, manifest_tests)

**Interfaces:**
- Produces: `pub struct ExportInfo { pub id: String, pub name: String, pub extension: String }`; `PluginMeta.exports: Vec<ExportInfo>`; `parse_manifest` accepts `capabilities = ["net"]`.

- [ ] **Step 1: Write the failing tests** (in `manifest_tests`)

```rust
#[test]
fn net_capability_is_accepted() {
    let m = parse_manifest(
        Path::new("/p/x"),
        "name=\"x\"\nversion=\"0\"\ncapabilities=[\"net\"]\n",
    )
    .unwrap();
    assert_eq!(m.capabilities, ["net"]);
}

#[test]
fn unknown_capability_still_rejected() {
    let err = parse_manifest(
        Path::new("/p/x"),
        "name=\"x\"\nversion=\"0\"\ncapabilities=[\"gpu\"]\n",
    )
    .unwrap_err();
    assert!(err.contains("gpu"), "{err}");
}

#[test]
fn exports_parse_with_default_extension() {
    let m = parse_manifest(
        Path::new("/p/x"),
        r#"
name = "x"
version = "0"
[[exports]]
id = "html"
name = "HTML"
extension = "html"
[[exports]]
id = "raw"
name = "Raw"
"#,
    )
    .unwrap();
    assert_eq!(m.exports[0].id, "html");
    assert_eq!(m.exports[0].extension, "html");
    assert_eq!(m.exports[1].extension, "txt");
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test manifest_tests`
Expected: FAIL — no field `exports`, `net` rejected. NOTE: the existing test `capabilities_key_is_rejected_forward_compat` uses `net` as its unknown example — update it to use `"gpu"` (it becomes redundant with `unknown_capability_still_rejected`; delete it and keep the new name).

- [ ] **Step 3: Implement**

In `ManifestFile` add:

```rust
    #[serde(default)]
    exports: Vec<ExportInfoFile>,
```

Above it define:

```rust
#[derive(Clone, Debug, serde::Deserialize)]
pub struct ExportInfoFile {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub extension: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ExportInfo {
    pub id: String,
    pub name: String,
    pub extension: String,
}
```

Capability loop becomes:

```rust
    for cap in &file.capabilities {
        if !matches!(cap.as_str(), "workspace-read" | "net") {
            return Err(format!(
                "manifest declares capability `{cap}`, which this SuperMD version \
                 does not support (known: workspace-read, net)"
            ));
        }
    }
```

`PluginMeta` gains `pub exports: Vec<ExportInfo>`; in the `Ok(PluginMeta { ... })` construction add:

```rust
        exports: file
            .exports
            .into_iter()
            .map(|e| ExportInfo {
                id: e.id,
                name: e.name,
                extension: e.extension.unwrap_or_else(|| "txt".to_string()),
            })
            .collect(),
```

- [ ] **Step 4: Run tests** — `cargo test manifest_tests` → PASS; `cargo test` → all green.

- [ ] **Step 5: Commit** — `git add -A && git commit -m "feat: manifest accepts net capability and [[exports]] entries"`

---

### Task 2: WIT 0.3 world, triple bindgen, fetch enforcement ladder, fetcher fixtures

The core task. Order inside it: WIT file → guest fixture (so tests exist to run) → host side.

**Files:**
- Create: `plugins/wit-v3/extension.wit`
- Create: `plugins/fixtures/fetcher/{Cargo.toml,src/lib.rs,plugin.toml}`
- Create: `tests/fixtures/plugins/nofetch/plugin.toml` (via build script; same wasm as fetcher, no capabilities)
- Modify: `scripts/build_plugins.sh` (fetcher in `--fixtures`; copy wasm into a `nofetch` fixture dir with its own manifest)
- Modify: `src/extensions.rs` (v3 bindgen module, `NetCtx`, `FetchTransport`, Host impl, `Bound::V3`, fallback chain, deadline budget, `export_document`, `set_transport`)

**Interfaces:**
- Produces:
  - `pub type FetchTransport = std::sync::Arc<dyn Fn(&TransportRequest) -> Result<TransportResponse, String> + Send + Sync>;`
  - `pub struct TransportRequest { pub method: String, pub url: String, pub headers: Vec<(String, String)>, pub body: Option<Vec<u8>> }`
  - `pub struct TransportResponse { pub status: u16, pub headers: Vec<(String, String)>, pub body: Vec<u8>, pub redirect: Option<String> }` (redirect = Location when status is 3xx)
  - `ExtensionHost::set_transport(&mut self, t: FetchTransport)`
  - `ExtensionHost::export_document(&mut self, plugin: &str, document: &str, format: &str, theme: &crate::diagram::DiagramTheme) -> Result<Vec<(String, Vec<u8>)>, String>`
- Consumes: Task 1's `exports`/`net` manifest support.

- [ ] **Step 1: Write the WIT world**

`plugins/wit-v3/extension.wit` — the v2 file's `types` interface verbatim (same records), plus:

```wit
package supermd:extension@0.3.0;

interface types {
    record command-info { id: string, title: string }
    record theme {
        background: string,
        surface: string,
        primary: string,
        text: string,
        muted: string,
        border: string,
        font-body: string,
        dark: bool,
    }
    record command-input {
        document: string,
        selection-start: u32,
        selection-end: u32,
    }
    variant command-output {
        replace-document(string),
        replace-selection(string),
        insert-at-cursor(string),
    }
}

interface host-api {
    record fetch-request {
        method: string,
        url: string,
        headers: list<tuple<string, string>>,
        body: option<list<u8>>,
    }
    record fetch-response {
        status: u16,
        headers: list<tuple<string, string>>,
        body: list<u8>,
    }
    /// Host-mediated HTTPS. A consent-shaped error
    /// ("consent required: <domain>") drives the banner flow.
    fetch: func(req: fetch-request) -> result<fetch-response, string>;
}

world extension {
    use types.{theme, command-input, command-output};
    import host-api;

    export render-block: func(lang: string, source: string, theme: theme)
        -> result<string, string>;
    export run-command: func(id: string, input: command-input)
        -> result<command-output, string>;
    export render-inline: func(pattern-id: string, matched: string)
        -> result<string, string>;
    export format-document: func(document: string)
        -> result<string, string>;
    export process-paste: func(text: string)
        -> result<option<string>, string>;

    record export-file { path: string, bytes: list<u8> }
    export export-document: func(document: string, format: string,
        theme: theme) -> result<list<export-file>, string>;
}
```

- [ ] **Step 2: Write the fetcher fixture guest**

`plugins/fixtures/fetcher/Cargo.toml`:

```toml
[package]
name = "fetcher"
version = "0.1.0"
edition = "2021"

[workspace]

[lib]
crate-type = ["cdylib"]

[dependencies]
wit-bindgen = "0.60"
```

`plugins/fixtures/fetcher/src/lib.rs` — exercises fetch through `format-document` (host tests already drive that surface for the reader fixture), and exports through `export-document`:

```rust
wit_bindgen::generate!({ path: "../../wit-v3", world: "extension" });

use supermd::extension::host_api;
use supermd::extension::types as t;

struct Plugin;

impl Guest for Plugin {
    fn render_block(_lang: String, _source: String, _theme: t::Theme) -> Result<String, String> {
        Err("unused".into())
    }
    fn run_command(_id: String, _input: t::CommandInput) -> Result<t::CommandOutput, String> {
        Err("unused".into())
    }
    fn render_inline(_id: String, _matched: String) -> Result<String, String> {
        Err("unused".into())
    }
    /// document = URL to fetch; "twice:<url>" fetches the same URL twice;
    /// "five:<url>" issues five fetches (limit probe).
    fn format_document(document: String) -> Result<String, String> {
        let fetch_one = |url: &str| -> Result<String, String> {
            let resp = host_api::fetch(&host_api::FetchRequest {
                method: "GET".into(),
                url: url.into(),
                headers: vec![],
                body: None,
            })?;
            Ok(format!(
                "status={} body={}",
                resp.status,
                String::from_utf8_lossy(&resp.body)
            ))
        };
        if let Some(url) = document.strip_prefix("five:") {
            for _ in 0..4 {
                fetch_one(url)?;
            }
            return fetch_one(url); // the fifth — host must reject
        }
        if let Some(url) = document.strip_prefix("twice:") {
            fetch_one(url)?;
            return fetch_one(url);
        }
        fetch_one(&document)
    }
    fn process_paste(_text: String) -> Result<Option<String>, String> {
        Ok(None)
    }
    /// format "one" → single file; "many" → three files incl. a subdir;
    /// "evil" → a traversal path the host must reject.
    fn export_document(
        document: String,
        format: String,
        _theme: t::Theme,
    ) -> Result<Vec<ExportFile>, String> {
        let bytes = document.into_bytes();
        Ok(match format.as_str() {
            "one" => vec![ExportFile { path: "out.txt".into(), bytes }],
            "many" => vec![
                ExportFile { path: "index.html".into(), bytes: bytes.clone() },
                ExportFile { path: "assets/style.css".into(), bytes: bytes.clone() },
                ExportFile { path: "assets/app.js".into(), bytes },
            ],
            "evil" => vec![ExportFile { path: "../evil.txt".into(), bytes }],
            other => return Err(format!("unknown format {other}")),
        })
    }
}

export!(Plugin);
```

(`ExportFile` is generated at the world root because the record is declared in the world; if the compiler says it lives elsewhere, follow its path.)

`plugins/fixtures/fetcher/plugin.toml`:

```toml
name = "fetcher"
version = "0.1.0"
formats = true
capabilities = ["net"]

[[exports]]
id = "one"
name = "One"
extension = "txt"

[[exports]]
id = "many"
name = "Many"

[[exports]]
id = "evil"
name = "Evil"
```

- [ ] **Step 3: Update the build script**

In `scripts/build_plugins.sh`, add `fetcher` to the `--fixtures` list. After the copy loop for fixtures, add the nofetch clone (same wasm, capability-free manifest — proves enforcement is by declaration, not binary):

```bash
    mkdir -p "$OUT/nofetch"
    cp "$OUT/fetcher/plugin.wasm" "$OUT/nofetch/plugin.wasm"
    cat > "$OUT/nofetch/plugin.toml" <<'EOF'
name = "nofetch"
version = "0.1.0"
formats = true
EOF
```

Run: `bash scripts/build_plugins.sh --fixtures` — fetcher must compile (host side not needed for the guest build).

- [ ] **Step 4: Write the failing host tests** (in `host_tests`)

A mock transport that records calls:

```rust
    fn mock_transport(
        responses: Vec<Result<TransportResponse, String>>,
    ) -> (FetchTransport, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        let calls = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let calls2 = calls.clone();
        let remaining = std::sync::Mutex::new(responses);
        let t: FetchTransport = std::sync::Arc::new(move |req: &TransportRequest| {
            calls2.lock().unwrap().push(req.url.clone());
            let mut r = remaining.lock().unwrap();
            if r.is_empty() {
                Ok(TransportResponse {
                    status: 200,
                    headers: vec![],
                    body: b"ok".to_vec(),
                    redirect: None,
                })
            } else {
                r.remove(0)
            }
        });
        (t, calls)
    }

    fn granted_host(dir: &Path, domains: &[&str]) -> ExtensionHost {
        let mut host = ExtensionHost::load(dir);
        let mut grants = std::collections::BTreeMap::new();
        grants.insert(
            "fetcher".to_string(),
            domains.iter().map(|d| format!("net:{d}")).collect(),
        );
        host.set_grants(grants);
        host
    }

    #[test]
    fn fetch_without_net_capability_errs_and_transport_untouched() {
        let Some(dir) = fixtures_dir() else { eprintln!("SKIP"); return; };
        let mut host = ExtensionHost::load(&dir);
        let (t, calls) = mock_transport(vec![]);
        host.set_transport(t);
        let e = host.format_document("nofetch", "https://example.com/x").unwrap_err();
        assert!(e.contains("net"), "{e}");
        assert!(calls.lock().unwrap().is_empty(), "transport was invoked");
    }

    #[test]
    fn fetch_requires_https() {
        let Some(dir) = fixtures_dir() else { eprintln!("SKIP"); return; };
        let mut host = granted_host(&dir, &["example.com"]);
        let (t, calls) = mock_transport(vec![]);
        host.set_transport(t);
        assert!(host.format_document("fetcher", "http://example.com/x").is_err());
        assert!(calls.lock().unwrap().is_empty());
    }

    #[test]
    fn ungranted_domain_is_consent_shaped_and_denied_is_quiet() {
        let Some(dir) = fixtures_dir() else { eprintln!("SKIP"); return; };
        let mut host = ExtensionHost::load(&dir);
        let (t, calls) = mock_transport(vec![]);
        host.set_transport(t);
        let e = host.format_document("fetcher", "https://example.com/x").unwrap_err();
        assert!(e.contains("consent required: example.com"), "{e}");
        let mut grants = std::collections::BTreeMap::new();
        grants.insert("fetcher".into(), vec!["denied:net:example.com".to_string()]);
        host.set_grants(grants);
        let e = host.format_document("fetcher", "https://example.com/x").unwrap_err();
        assert!(!e.contains("consent required"), "denied must not re-prompt: {e}");
        assert!(calls.lock().unwrap().is_empty());
    }

    #[test]
    fn granted_fetch_roundtrips_through_plugin() {
        let Some(dir) = fixtures_dir() else { eprintln!("SKIP"); return; };
        let mut host = granted_host(&dir, &["example.com"]);
        let (t, _) = mock_transport(vec![Ok(TransportResponse {
            status: 200,
            headers: vec![],
            body: b"hello net".to_vec(),
            redirect: None,
        })]);
        host.set_transport(t);
        let out = host.format_document("fetcher", "https://example.com/x").unwrap();
        assert_eq!(out, "status=200 body=hello net");
    }

    #[test]
    fn redirect_to_ungranted_domain_is_blocked() {
        let Some(dir) = fixtures_dir() else { eprintln!("SKIP"); return; };
        let mut host = granted_host(&dir, &["a.com"]);
        let (t, calls) = mock_transport(vec![Ok(TransportResponse {
            status: 302,
            headers: vec![],
            body: vec![],
            redirect: Some("https://b.com/next".into()),
        })]);
        host.set_transport(t);
        assert!(host.format_document("fetcher", "https://a.com/x").is_err());
        assert_eq!(calls.lock().unwrap().len(), 1, "must not follow to b.com");
    }

    #[test]
    fn redirect_within_granted_domains_is_followed() {
        let Some(dir) = fixtures_dir() else { eprintln!("SKIP"); return; };
        let mut host = granted_host(&dir, &["a.com", "b.com"]);
        let (t, calls) = mock_transport(vec![
            Ok(TransportResponse { status: 302, headers: vec![], body: vec![], redirect: Some("https://b.com/next".into()) }),
            Ok(TransportResponse { status: 200, headers: vec![], body: b"followed".to_vec(), redirect: None }),
        ]);
        host.set_transport(t);
        let out = host.format_document("fetcher", "https://a.com/x").unwrap();
        assert_eq!(out, "status=200 body=followed");
        assert_eq!(calls.lock().unwrap().len(), 2);
    }

    #[test]
    fn oversized_response_is_rejected() {
        let Some(dir) = fixtures_dir() else { eprintln!("SKIP"); return; };
        let mut host = granted_host(&dir, &["example.com"]);
        let (t, _) = mock_transport(vec![Ok(TransportResponse {
            status: 200,
            headers: vec![],
            body: vec![0u8; 2 * 1024 * 1024 + 1],
            redirect: None,
        })]);
        host.set_transport(t);
        assert!(host.format_document("fetcher", "https://example.com/x").is_err());
    }

    #[test]
    fn fifth_fetch_in_one_call_is_rejected() {
        let Some(dir) = fixtures_dir() else { eprintln!("SKIP"); return; };
        let mut host = granted_host(&dir, &["example.com"]);
        let (t, calls) = mock_transport(vec![]);
        host.set_transport(t);
        assert!(host.format_document("fetcher", "five:https://example.com/x").is_err());
        assert_eq!(calls.lock().unwrap().len(), 4, "fifth must be pre-empted");
        // and the budget resets between calls:
        assert!(host.format_document("fetcher", "twice:https://example.com/x").is_ok());
    }

    #[test]
    fn export_document_roundtrips_and_v2_errs_readably() {
        let Some(dir) = fixtures_dir() else { eprintln!("SKIP"); return; };
        let mut host = ExtensionHost::load(&dir);
        let theme = crate::diagram::DiagramTheme::default_light();
        let files = host.export_document("fetcher", "doc-body", "many", &theme).unwrap();
        assert_eq!(files.len(), 3);
        assert_eq!(files[1].0, "assets/style.css");
        assert_eq!(files[0].1, b"doc-body");
        let e = host.export_document("echo", "d", "one", &theme).unwrap_err();
        assert!(e.contains("0.3"), "{e}");
    }
```

Run: `cargo test host_tests` → FAIL (no `set_transport`, no types, fixture won't instantiate).

- [ ] **Step 5: Implement the host side**

In `src/extensions.rs`:

a) Third bindgen module:

```rust
/// 0.3 bindings: adds export-document and the host-api fetch import.
mod v3 {
    wasmtime::component::bindgen!({
        path: "plugins/wit-v3/extension.wit",
        world: "extension",
    });
}
```

b) Net context + transport types (module level):

```rust
pub struct TransportRequest {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
}

pub struct TransportResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    /// Location target when status is a redirect.
    pub redirect: Option<String>,
}

pub type FetchTransport =
    std::sync::Arc<dyn Fn(&TransportRequest) -> Result<TransportResponse, String> + Send + Sync>;

const MAX_FETCHES_PER_CALL: u32 = 4;
const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
const MAX_REDIRECT_HOPS: u32 = 5;
const FETCH_TIMEOUT_TICKS: u64 = 10; // 5s per fetch in epoch ticks
/// Deadline for calls into net-capable plugins: compute budget plus the
/// whole network budget (see plan header for why this is pre-budgeted).
const NET_CALL_DEADLINE_TICKS: u64 =
    CALL_DEADLINE_TICKS + MAX_FETCHES_PER_CALL as u64 * FETCH_TIMEOUT_TICKS;

/// Per-call network state carried by the store.
struct NetCtx {
    /// Manifest declares `net`.
    declared: bool,
    /// Domains with a persisted grant.
    granted: Vec<String>,
    /// Domains with a persisted denial.
    denied: Vec<String>,
    transport: FetchTransport,
    fetches_used: u32,
}
```

c) `HostState` gains `net: Option<NetCtx>`; `zero_grant_state()` and `state_for` set `net: None` / build it. `state_for` addition (after the workspace-read branch, before the fallback):

```rust
        // net context rides every store for a net-declaring plugin;
        // enforcement is per-domain inside fetch.
        let net = self.plugins.iter().find(|p| p.meta.name == plugin).map(|p| NetCtx {
            declared: p.meta.capabilities.iter().any(|c| c == "net"),
            granted: self.net_domains(plugin, "net:"),
            denied: self.net_domains(plugin, "denied:net:"),
            transport: self.transport.clone(),
            fetches_used: 0,
        });
```

with helper:

```rust
    fn net_domains(&self, plugin: &str, prefix: &str) -> Vec<String> {
        self.grants
            .get(plugin)
            .map(|caps| {
                caps.iter()
                    .filter_map(|c| c.strip_prefix(prefix).map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    }
```

Note the grant format: `"net:<domain>"` granted, `"denied:net:<domain>"` denied — `net_domains(_, "net:")` must not match denied entries, so filter `denied:` first: use `caps.iter().filter(|c| !c.starts_with("denied:"))` in the granted case. Exact code:

```rust
    fn net_domains(&self, plugin: &str, denied: bool) -> Vec<String> {
        let prefix = if denied { "denied:net:" } else { "net:" };
        self.grants
            .get(plugin)
            .map(|caps| {
                caps.iter()
                    .filter(|c| denied || !c.starts_with("denied:"))
                    .filter_map(|c| c.strip_prefix(prefix).map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    }
```

d) Implement the generated host trait on `HostState` (module path per bindgen; expected `v3::supermd::extension::host_api::Host`; follow the compiler if it differs). The whole enforcement ladder plus manual redirect loop:

```rust
fn url_domain(url: &str) -> Option<&str> {
    url.strip_prefix("https://")?.split(['/', '?', '#']).next()?.split(':').next()
}

impl v3::supermd::extension::host_api::Host for HostState {
    fn fetch(
        &mut self,
        req: v3::supermd::extension::host_api::FetchRequest,
    ) -> Result<v3::supermd::extension::host_api::FetchResponse, String> {
        use v3::supermd::extension::host_api::FetchResponse;
        let Some(net) = self.net.as_mut() else {
            return Err("net capability not declared".to_string());
        };
        if !net.declared {
            return Err("net capability not declared".to_string());
        }
        if net.fetches_used >= MAX_FETCHES_PER_CALL {
            return Err(format!("fetch limit ({MAX_FETCHES_PER_CALL} per call) exceeded"));
        }
        net.fetches_used += 1;
        let mut url = req.url.clone();
        let mut hops = 0u32;
        loop {
            let Some(domain) = url_domain(&url).map(str::to_string) else {
                return Err(format!("only https:// URLs are allowed: {url}"));
            };
            if net.denied.iter().any(|d| d == &domain) {
                return Err(format!("access to {domain} was denied"));
            }
            if !net.granted.iter().any(|d| d == &domain) {
                return Err(format!("consent required: {domain}"));
            }
            let resp = (net.transport)(&TransportRequest {
                method: req.method.clone(),
                url: url.clone(),
                headers: req.headers.clone(),
                body: req.body.clone(),
            })?;
            if let Some(target) = resp.redirect {
                hops += 1;
                if hops > MAX_REDIRECT_HOPS {
                    return Err("too many redirects".to_string());
                }
                url = target;
                continue; // next loop re-checks the new domain
            }
            if resp.body.len() > MAX_RESPONSE_BYTES {
                return Err(format!("response exceeds {MAX_RESPONSE_BYTES} bytes"));
            }
            return Ok(FetchResponse {
                status: resp.status,
                headers: resp.headers,
                body: resp.body,
            });
        }
    }
}
```

(If bindgen's generated trait signature wraps returns in `wasmtime::Result<...>` — i.e. `fn fetch(&mut self, req) -> wasmtime::Result<Result<FetchResponse, String>>` — wrap the body's result in `Ok(...)` accordingly. The inner `Result<_, String>` is the WIT-level result either way; a host-side `Err` must be the WIT error, never a trap.)

Note: the redirect loop's re-fetch does not increment `fetches_used` per hop — redirect hops are bounded by `MAX_REDIRECT_HOPS` instead; a plugin's fetch budget counts logical fetches.

e) Default ureq transport + `ExtensionHost.transport` field. Add `ureq = "3"` to `Cargo.toml` `[dependencies]`.

```rust
fn ureq_transport() -> FetchTransport {
    std::sync::Arc::new(|req: &TransportRequest| {
        let config = ureq::Agent::config_builder()
            .max_redirects(0)
            .timeout_global(Some(std::time::Duration::from_secs(5)))
            .build();
        let agent: ureq::Agent = config.into();
        let mut request = match req.method.as_str() {
            "GET" => agent.get(&req.url),
            "POST" => agent.post(&req.url),
            other => return Err(format!("unsupported method {other}")),
        };
        for (k, v) in &req.headers {
            request = request.header(k, v);
        }
        let result = if let Some(body) = &req.body {
            request.send(&body[..])
        } else {
            request.send_empty()
        };
        // ureq 3 returns Err on 4xx/5xx by default unless configured;
        // configure http_status_as_error(false) in the builder so
        // statuses pass through as data.
        let response = result.map_err(|e| e.to_string())?;
        let status = response.status().as_u16();
        let redirect = response
            .headers()
            .get("location")
            .and_then(|v| v.to_str().ok())
            .filter(|_| (300..400).contains(&status))
            .map(|s| s.to_string());
        let headers = response
            .headers()
            .iter()
            .filter_map(|(k, v)| v.to_str().ok().map(|v| (k.to_string(), v.to_string())))
            .collect();
        let mut body = Vec::new();
        use std::io::Read as _;
        response
            .into_body()
            .into_reader()
            .take(MAX_RESPONSE_BYTES as u64 + 1)
            .read_to_end(&mut body)
            .map_err(|e| e.to_string())?;
        Ok(TransportResponse { status, headers, body, redirect })
    })
}
```

(Add `.http_status_as_error(false)` to the config builder; exact ureq 3 method names may shift slightly — follow docs.rs/ureq/3 if the compiler objects. The transport is exercised by real usage, not unit tests; all ladder tests use the mock.)

`ExtensionHost` struct gains `transport: FetchTransport`; `load()` initializes `transport: ureq_transport()`; add:

```rust
    /// Test hook; also rebuilds instances so stores pick it up.
    pub fn set_transport(&mut self, t: FetchTransport) {
        self.transport = t;
        for p in &mut self.plugins {
            p.instance = None;
        }
    }
```

f) `Bound::V3` + fallback chain in `ensure_bound`. Add the variant, link host-api once on the shared linker, try v3 → v2 → v1:

```rust
enum Bound {
    V1(wasmtime::Store<HostState>, Extension),
    V2(wasmtime::Store<HostState>, v2::Extension),
    V3(wasmtime::Store<HostState>, v3::Extension),
}
```

In `ensure_bound`, after `add_to_linker_sync`:

```rust
            v3::supermd::extension::host_api::add_to_linker::<HostState, wasmtime::component::HasSelf<HostState>>(
                &mut linker,
                |state| state,
            )
            .map_err(|e| format!("host-api link: {e:#}"))?;
```

(Generic parameters per wasmtime 48's generated `add_to_linker`; if the generated helper is `add_to_linker_get_host` or takes only the closure, follow the compiler.) Then:

```rust
            match v3::Extension::instantiate(&mut store, &slot.component, &linker) {
                Ok(instance) => slot.instance = Some(Bound::V3(store, instance)),
                Err(_) => match v2::Extension::instantiate(&mut store, &slot.component, &linker) {
                    Ok(instance) => slot.instance = Some(Bound::V2(store, instance)),
                    Err(_) => { /* existing V1 fallback body unchanged */ }
                },
            }
```

CAUTION: each `instantiate` consumes work on the same store; a failed v3 attempt may leave junk in the store. Mirror the existing pattern instead: build a FRESH store per attempt (the current code already builds a fresh store for the v1 fallback — do the same for v2). `state_for` is cheap; call it per attempt.

g) Deadline budget in `with_instance`: replace the two `set_epoch_deadline` lines with a capability-aware pick:

```rust
        let ticks = if self
            .plugins
            .iter()
            .find(|p| p.meta.name == plugin)
            .is_some_and(|p| p.meta.capabilities.iter().any(|c| c == "net"))
        {
            NET_CALL_DEADLINE_TICKS
        } else {
            CALL_DEADLINE_TICKS
        };
```

(compute `ticks` BEFORE `ensure_bound` borrows self mutably, or read it from the bound's meta — borrow order matters; simplest is a small `fn call_ticks(&self, plugin: &str) -> u64` called first.)

h) All five existing call surfaces gain a `Bound::V3` arm mirroring V2 (types under `v3::supermd::extension::types`). New surface:

```rust
    /// 0.3-only. Blocking; call from the background executor only.
    pub fn export_document(
        &mut self,
        plugin: &str,
        document: &str,
        format: &str,
        theme: &crate::diagram::DiagramTheme,
    ) -> Result<Vec<(String, Vec<u8>)>, String> {
        let t = (
            theme.background.clone(), theme.surface.clone(), theme.primary.clone(),
            theme.text.clone(), theme.muted.clone(), theme.border.clone(),
            theme.font_body.clone(), theme.dark,
        );
        self.with_instance(plugin, |bound| match bound {
            Bound::V1(..) | Bound::V2(..) => Ok(Err("requires a 0.3 plugin".to_string())),
            Bound::V3(store, i) => {
                let theme = v3::supermd::extension::types::Theme {
                    background: t.0.clone(), surface: t.1.clone(), primary: t.2.clone(),
                    text: t.3.clone(), muted: t.4.clone(), border: t.5.clone(),
                    font_body: t.6.clone(), dark: t.7,
                };
                i.call_export_document(store, document, format, &theme)
                    .map(|r| r.map(|files| files.into_iter().map(|f| (f.path, f.bytes)).collect()))
            }
        })?
        .map_err(|e| e)
    }
```

(`ExportFile`'s generated Rust path: likely `v3::ExportFile` since the record is world-level; follow the compiler.)

- [ ] **Step 6: Rebuild fixtures and run**

Run: `bash scripts/build_plugins.sh --fixtures && cargo test host_tests`
Expected: all Task 2 tests PASS, and the Phase 1/2 regressions (`echo_*`, `panicking_*`, `hanging_*`, `v2_surfaces_*`, `reader_*`) still PASS (echo stays a 0.2 binding through the fallback; panic/hang stay 0.1).

- [ ] **Step 7: Full suite** — `cargo test` → green.

- [ ] **Step 8: Commit** — `git add -A && git commit -m "feat: wit 0.3 world with host-mediated fetch and export-document"`

---

### Task 3: Export path validation + write logic (pure, no dialogs)

**Files:**
- Modify: `src/extensions.rs` (or a small `mod` inside it): `validate_export_paths`, `write_export`

**Interfaces:**
- Produces:
  - `pub fn validate_export_paths(files: &[(String, Vec<u8>)]) -> Result<(), String>` — rejects empty sets, absolute paths, and any `..` component.
  - `pub enum ExportDest { File(std::path::PathBuf), Dir(std::path::PathBuf) }`
  - `pub fn write_export(files: &[(String, Vec<u8>)], dest: &ExportDest) -> Result<(), String>` — `File` writes files[0] to the exact path; `Dir` writes each under it, creating subdirs.
- Consumes: Task 2's `export_document` return shape `Vec<(String, Vec<u8>)>`.

- [ ] **Step 1: Write the failing tests** (new `mod export_tests` in extensions.rs)

```rust
#[cfg(test)]
mod export_tests {
    use super::*;

    fn f(path: &str) -> (String, Vec<u8>) {
        (path.to_string(), b"x".to_vec())
    }

    #[test]
    fn validate_rejects_traversal_absolute_and_empty() {
        assert!(validate_export_paths(&[f("ok.txt")]).is_ok());
        assert!(validate_export_paths(&[f("sub/ok.txt")]).is_ok());
        assert!(validate_export_paths(&[f("../evil.txt")]).is_err());
        assert!(validate_export_paths(&[f("sub/../../evil.txt")]).is_err());
        assert!(validate_export_paths(&[f("/abs.txt")]).is_err());
        assert!(validate_export_paths(&[]).is_err());
    }

    #[test]
    fn write_single_file_lands_at_exact_path() {
        let dir = tempfile::tempdir().unwrap();
        let dest = ExportDest::File(dir.path().join("chosen-name.html"));
        write_export(&[("ignored.html".into(), b"body".to_vec())], &dest).unwrap();
        assert_eq!(std::fs::read(dir.path().join("chosen-name.html")).unwrap(), b"body");
    }

    #[test]
    fn write_many_creates_subdirs_under_dir() {
        let dir = tempfile::tempdir().unwrap();
        let dest = ExportDest::Dir(dir.path().to_path_buf());
        write_export(
            &[
                ("index.html".into(), b"i".to_vec()),
                ("assets/style.css".into(), b"c".to_vec()),
            ],
            &dest,
        )
        .unwrap();
        assert_eq!(std::fs::read(dir.path().join("assets/style.css")).unwrap(), b"c");
    }
}
```

- [ ] **Step 2: Run to verify FAIL** — `cargo test export_tests` → missing functions.

- [ ] **Step 3: Implement**

```rust
/// Exporter paths must stay relative and inside the chosen directory.
pub fn validate_export_paths(files: &[(String, Vec<u8>)]) -> Result<(), String> {
    if files.is_empty() {
        return Err("exporter returned no files".to_string());
    }
    for (path, _) in files {
        let p = std::path::Path::new(path);
        let bad = p.is_absolute()
            || p.components().any(|c| {
                matches!(
                    c,
                    std::path::Component::ParentDir | std::path::Component::RootDir
                        | std::path::Component::Prefix(_)
                )
            });
        if bad {
            return Err(format!("exporter returned an unsafe path: {path}"));
        }
    }
    Ok(())
}

pub enum ExportDest {
    File(std::path::PathBuf),
    Dir(std::path::PathBuf),
}

pub fn write_export(files: &[(String, Vec<u8>)], dest: &ExportDest) -> Result<(), String> {
    validate_export_paths(files)?;
    match dest {
        ExportDest::File(path) => {
            std::fs::write(path, &files[0].1).map_err(|e| e.to_string())
        }
        ExportDest::Dir(root) => {
            for (rel, bytes) in files {
                let target = root.join(rel);
                if let Some(parent) = target.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                }
                std::fs::write(&target, bytes).map_err(|e| e.to_string())?;
            }
            Ok(())
        }
    }
}
```

- [ ] **Step 4: Run** — `cargo test export_tests` → PASS; also add a host test asserting `validate_export_paths` rejects the fetcher fixture's "evil" format output:

```rust
    #[test]
    fn evil_export_paths_are_rejected() {
        let Some(dir) = fixtures_dir() else { eprintln!("SKIP"); return; };
        let mut host = ExtensionHost::load(&dir);
        let files = host
            .export_document("fetcher", "d", "evil", &crate::diagram::DiagramTheme::default_light())
            .unwrap();
        assert!(validate_export_paths(&files).is_err());
    }
```

- [ ] **Step 5: Commit** — `git commit -am "feat: export path validation and write logic"`

---

### Task 4: Workspace export flow — palette entries, dialogs, writes

**Files:**
- Modify: `src/workspace.rs` (`toggle_palette` entry building, `run_plugin_command` `__export:` branch)

**Interfaces:**
- Consumes: `PluginMeta.exports`, `ExtensionHost::export_document`, `validate_export_paths`, `write_export`, `ExportDest`, gpui `cx.prompt_for_new_path(directory, suggested_name)` and `cx.prompt_for_paths(PathPromptOptions { directories: true, .. })` (existing use at workspace.rs:751).
- Produces: palette ids of the form `"__export:<format-id>"`; suggested filename `<active file stem>.<extension>`.

No pure unit seam worth faking dialogs for — this task is wiring, verified by the fixture-driven host tests (already green) plus a manual smoke test. TDD applies to logic; dialog plumbing follows the existing `open_folder` pattern.

- [ ] **Step 1: Palette entries.** In `toggle_palette`, after the `format_plugins()` loop:

```rust
                for p in host.plugins() {
                    for e in &p.exports {
                        entries.push(crate::palette::PaletteEntry {
                            plugin: p.name.clone(),
                            id: format!("__export:{}", e.id),
                            title: format!("Export: {}", e.name),
                        });
                    }
                }
```

- [ ] **Step 2: The `__export:` branch** in `run_plugin_command`, right after the `__format` branch (same shape: background call, then UI):

```rust
        if let Some(format) = id.strip_prefix("__export:") {
            let format = format.to_string();
            let plugin_bg = plugin.clone();
            let theme = crate::theme::active_diagram_theme(cx); // see note below
            let stem = self
                .tabs
                .get(self.active)
                .and_then(|t| match t {
                    Tab::Editor { editor, .. } => {
                        editor.read(cx).path().file_stem().map(|s| s.to_string_lossy().into_owned())
                    }
                    _ => None,
                })
                .unwrap_or_else(|| "export".to_string());
            let extension = cx
                .try_global::<crate::extensions::ExtensionState>()
                .and_then(|s| {
                    s.0.lock().unwrap().plugins().iter().find(|p| p.name == plugin).and_then(|p| {
                        p.exports.iter().find(|e| e.id == format).map(|e| e.extension.clone())
                    })
                })
                .unwrap_or_else(|| "txt".to_string());
            let run = cx.background_executor().spawn(async move {
                host.lock().unwrap().export_document(&plugin_bg, &document, &format, &theme)
            });
            cx.spawn(async move |this, cx| {
                let result = run.await;
                this.update(cx, |this, cx| match result {
                    Ok(files) => {
                        if let Err(e) = crate::extensions::validate_export_paths(&files) {
                            this.show_command_error(e, cx);
                            return;
                        }
                        this.finish_export(files, stem, extension, cx);
                    }
                    Err(e) => this.handle_plugin_error(plugin.clone(), e, cx),
                })
                .ok();
            })
            .detach();
            return;
        }
```

Notes for the implementer:
- `crate::theme::active_diagram_theme(cx)` — whatever helper the diagram widgets use today to build a `DiagramTheme` from the active theme; find it with `grep -n "DiagramTheme" src/theme.rs src/editor/projector.rs` and reuse (do NOT invent a second theme-mapping).
- `editor.read(cx).path()` — check the editor's actual accessor for its file path (`grep -n "fn path" src/editor/mod.rs`); adjust.

- [ ] **Step 3: `finish_export`** on Workspace — dialog choice by file count:

```rust
    fn finish_export(
        &mut self,
        files: Vec<(String, Vec<u8>)>,
        stem: String,
        extension: String,
        cx: &mut Context<Self>,
    ) {
        use crate::extensions::{write_export, ExportDest};
        if files.len() == 1 {
            let rx = cx.prompt_for_new_path(
                &crate::platform::home_dir(),
                Some(&format!("{stem}.{extension}")),
            );
            cx.spawn(async move |this, cx| {
                if let Ok(Ok(Some(path))) = rx.await {
                    let result = write_export(&files, &ExportDest::File(path));
                    this.update(cx, |this, cx| {
                        if let Err(e) = result {
                            this.show_command_error(e, cx);
                        } else {
                            this.show_command_error("Exported".to_string(), cx);
                        }
                    })
                    .ok();
                }
            })
            .detach();
        } else {
            let rx = cx.prompt_for_paths(gpui::PathPromptOptions {
                files: false,
                directories: true,
                multiple: false,
                prompt: None,
            });
            cx.spawn(async move |this, cx| {
                if let Ok(Ok(Some(mut paths))) = rx.await {
                    if let Some(dir) = paths.pop() {
                        let result = write_export(&files, &ExportDest::Dir(dir));
                        this.update(cx, |this, cx| {
                            if let Err(e) = result {
                                this.show_command_error(e, cx);
                            } else {
                                this.show_command_error("Exported".to_string(), cx);
                            }
                        })
                        .ok();
                    }
                }
            })
            .detach();
        }
    }
```

(Match `prompt_for_new_path`'s exact signature/return from the vendored gpui at scratchpad/gpui-0.2.2/src/app.rs:1129 and the existing `prompt_for_paths` call at workspace.rs:751 — the `PathPromptOptions` fields there are authoritative. The success message reuses the transient strip; "Exported" auto-dismisses in 4 s.)

- [ ] **Step 4: Build + full suite** — `cargo build && cargo test` → green (no new unit tests; wiring only).

- [ ] **Step 5: Commit** — `git commit -am "feat: export palette commands with save-dialog flow"`

---

### Task 5: Surface-table split — sync paste vs net enrichers

**Files:**
- Modify: `src/extensions.rs` (`set_surface_tables`, new `enrich_plugins()`; tests)
- Modify: `src/editor/mod.rs` paste comment only (behavior change lands in Task 6)

**Interfaces:**
- Produces: `pub fn enrich_plugins() -> Vec<String>` — paste plugins WITH `net`; `paste_plugins()` now returns only paste plugins WITHOUT `net`.

- [ ] **Step 1: Write the failing test** (in `manifest_tests` or a new small mod)

```rust
    #[test]
    fn paste_tables_split_by_net_capability() {
        let sync_meta = parse_manifest(
            Path::new("/p/tidy"),
            "name=\"tidy\"\nversion=\"0\"\npaste=true\n",
        )
        .unwrap();
        let net_meta = parse_manifest(
            Path::new("/p/url-title"),
            "name=\"url-title\"\nversion=\"0\"\npaste=true\ncapabilities=[\"net\"]\n",
        )
        .unwrap();
        set_surface_tables(&[sync_meta, net_meta]);
        assert_eq!(paste_plugins(), ["tidy"]);
        assert_eq!(enrich_plugins(), ["url-title"]);
    }
```

- [ ] **Step 2: Verify FAIL** — `cargo test paste_tables_split` → no `enrich_plugins`.

- [ ] **Step 3: Implement** — in `set_surface_tables`:

```rust
static ENRICH_PLUGINS: std::sync::RwLock<Vec<String>> = std::sync::RwLock::new(Vec::new());

pub fn set_surface_tables(metas: &[PluginMeta]) {
    let has_net = |m: &&PluginMeta| m.capabilities.iter().any(|c| c == "net");
    *FORMAT_PLUGINS.write().unwrap() =
        metas.iter().filter(|m| m.formats).map(|m| m.name.clone()).collect();
    *PASTE_PLUGINS.write().unwrap() = metas
        .iter()
        .filter(|m| m.paste && !has_net(m))
        .map(|m| m.name.clone())
        .collect();
    *ENRICH_PLUGINS.write().unwrap() = metas
        .iter()
        .filter(|m| m.paste && has_net(m))
        .map(|m| m.name.clone())
        .collect();
}

pub fn enrich_plugins() -> Vec<String> {
    ENRICH_PLUGINS.read().unwrap().clone()
}
```

- [ ] **Step 4: Run** — `cargo test` → green (tests share the static tables across threads; if the new test races another table-touching test, serialize by naming it into the same test as its neighbors or use distinct plugin names — distinct names as written suffice because assertions are exact-equality; if flaky, merge the assertions into one test).

- [ ] **Step 5: Commit** — `git commit -am "feat: split paste plugins into sync and net-enricher tables"`

---

### Task 6: Editor async enrich pass + net consent banner + retry

**Files:**
- Modify: `src/editor/mod.rs` (paste hook, `pending_enrich` field, `EditorEvent`, `start_enrich`, `apply_enrichment`, `retry_enrich`)
- Modify: `src/workspace.rs` (`make_editor` helper + 4 call sites, `consent_request` becomes `(String, String)`, `handle_plugin_error` parses net consent, `resolve_consent` takes the cap, banner copy, retry after Allow)
- Modify: `src/editor/core.rs` ONLY if a range-replace helper is missing (check first; `Selection { anchor, head }` + `insert` covers it)

**Interfaces:**
- Produces:
  - `pub enum EditorEvent { ConsentNeeded { plugin: String, cap: String } }` + `impl EventEmitter<EditorEvent> for Editor`
  - `Editor::retry_enrich(&mut self, cx: &mut Context<Self>)`
  - Workspace `consent_request: Option<(String, String)>` — (plugin, cap) where cap is `"workspace-read"` or `"net:<domain>"`.
- Consumes: `enrich_plugins()`, `ExtensionState`, `process_paste`, the consent-shaped errors `"awaiting consent for workspace-read"` and `"consent required: <domain>"`.

- [ ] **Step 1: Write the failing core-logic test** (editor test module; pure text logic — the generation guard):

```rust
    #[test]
    fn enrichment_applies_only_when_snapshot_matches() {
        // free function so it's testable without an Entity:
        assert_eq!(
            enrich_plan("abc URL def", 4..7, "abc URL def", "[T](URL)"),
            Some(("abc [T](URL) def".to_string(), 4..12))
        );
        // document moved since the paste → discard
        assert_eq!(enrich_plan("abc URL defX", 4..7, "abc URL def", "[T](URL)"), None);
    }
```

- [ ] **Step 2: Verify FAIL**, then implement the pure helper in `editor/mod.rs`:

```rust
/// Compute the enriched document and the replacement's new range, or
/// None when the document changed since the paste snapshot.
fn enrich_plan(
    current: &str,
    pasted: std::ops::Range<usize>,
    snapshot: &str,
    replacement: &str,
) -> Option<(String, std::ops::Range<usize>)> {
    if current != snapshot {
        return None;
    }
    let mut out = String::with_capacity(current.len());
    out.push_str(&current[..pasted.start]);
    out.push_str(replacement);
    out.push_str(&current[pasted.end..]);
    Some((out, pasted.start..pasted.start + replacement.len()))
}
```

Run: PASS.

- [ ] **Step 3: Wire the async pass.** Editor gains:

```rust
    /// A paste awaiting (or retrying) net enrichment.
    pending_enrich: Option<PendingEnrich>,
```

```rust
struct PendingEnrich {
    range: std::ops::Range<usize>,
    snapshot: String,
    pasted: String,
}
```

At the end of `paste` (after `self.insert_str(&out, cx);`):

```rust
        if !crate::extensions::enrich_plugins().is_empty() {
            let head = self.core.selection.head;
            self.pending_enrich = Some(PendingEnrich {
                range: head - out.len()..head,
                snapshot: self.core.buffer.text(),
                pasted: out.clone(),
            });
            self.start_enrich(cx);
        }
```

`start_enrich` + apply:

```rust
    /// Run net-capable paste plugins in the background; first Some wins.
    fn start_enrich(&mut self, cx: &mut Context<Self>) {
        let Some(pending) = self.pending_enrich.as_ref() else { return };
        let Some(state) = cx.try_global::<crate::extensions::ExtensionState>() else { return };
        let host = state.0.clone();
        let text = pending.pasted.clone();
        let task = cx.background_executor().spawn(async move {
            let plugins = crate::extensions::enrich_plugins();
            let mut consent: Option<(String, String)> = None;
            for plugin in plugins {
                match host.lock().unwrap().process_paste(&plugin, &text) {
                    Ok(Some(replacement)) => return Ok(Some(replacement)),
                    Ok(None) => {}
                    Err(e) => {
                        if let Some(domain) = e.split("consent required: ").nth(1) {
                            consent = Some((plugin, format!("net:{domain}")));
                        }
                        // other errors: enrichment is best-effort; skip
                    }
                }
            }
            match consent {
                Some(c) => Err(c),
                None => Ok(None),
            }
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| match result {
                Ok(Some(replacement)) => this.apply_enrichment(&replacement, cx),
                Ok(None) => this.pending_enrich = None,
                Err((plugin, cap)) => {
                    // keep pending_enrich for the retry after Allow
                    cx.emit(EditorEvent::ConsentNeeded { plugin, cap });
                }
            })
            .ok();
        })
        .detach();
    }

    fn apply_enrichment(&mut self, replacement: &str, cx: &mut Context<Self>) {
        let Some(pending) = self.pending_enrich.take() else { return };
        let current = self.core.buffer.text();
        let Some((_, _)) = enrich_plan(&current, pending.range.clone(), &pending.snapshot, replacement)
        else {
            return; // document moved; forfeit (recorded honest limit)
        };
        self.core.break_undo_group();
        self.core.selection = Selection { anchor: pending.range.start, head: pending.range.end };
        self.core.insert(replacement, Instant::now());
        self.core.break_undo_group();
        self.after_edit(cx);
    }

    /// Called by the workspace after a net grant lands.
    pub fn retry_enrich(&mut self, cx: &mut Context<Self>) {
        self.start_enrich(cx);
    }
```

And the event type near the Editor struct:

```rust
pub enum EditorEvent {
    ConsentNeeded { plugin: String, cap: String },
}

impl gpui::EventEmitter<EditorEvent> for Editor {}
```

Initialize `pending_enrich: None` in the constructor(s).

- [ ] **Step 4: Workspace side.** Add a `make_editor` helper and use it at all 4 `cx.new(|cx| Editor::from_text(...))` sites (lines ~279, ~292, ~631, ~715):

```rust
    fn make_editor(
        &mut self,
        path: &std::path::Path,
        text: String,
        cx: &mut Context<Self>,
    ) -> Entity<Editor> {
        let langs = crate::highlight::languages(cx);
        let editor = cx.new(|cx| Editor::from_text(path, text, &langs, cx));
        cx.subscribe(&editor, |this, _editor, event, cx| match event {
            crate::editor::EditorEvent::ConsentNeeded { plugin, cap } => {
                this.consent_request = Some((plugin.clone(), cap.clone()));
                cx.notify();
            }
        })
        .detach();
        editor
    }
```

(Adapt: some call sites may not have `&mut self`/`Context<Self>` handy in closures — if a site builds the editor inside a `cx.spawn`/`update`, call the helper inside the `this.update` scope. Follow the surrounding structure; the subscription must be created with the workspace's `Context`.)

Consent plumbing changes:
- `consent_request: Option<(String, String)>` (plugin, cap).
- `handle_plugin_error`: also match net:

```rust
        if error.contains("awaiting consent") {
            self.consent_request = Some((plugin, "workspace-read".to_string()));
            cx.notify();
        } else if let Some(domain) = error.split("consent required: ").nth(1) {
            self.consent_request = Some((plugin, format!("net:{}", domain.trim())));
            cx.notify();
        } else {
            self.show_command_error(error, cx);
        }
```

- `resolve_consent`:

```rust
    fn resolve_consent(&mut self, allow: bool, cx: &mut Context<Self>) {
        let Some((plugin, cap)) = self.consent_request.take() else { return };
        let dir = crate::settings::config_dir();
        let mut settings = crate::settings::load(&dir);
        let grant = if allow { cap.clone() } else { format!("denied:{cap}") };
        settings.plugin_grants.entry(plugin).or_default().push(grant);
        let _ = crate::settings::save(&dir, &settings);
        if let Some(state) = cx.try_global::<crate::extensions::ExtensionState>() {
            state.0.lock().unwrap().set_grants(settings.plugin_grants.clone());
        }
        if allow && cap.starts_with("net:") {
            // retry the enrichment that raised the banner
            if let Some(Tab::Editor { editor, .. }) = self.tabs.get(self.active) {
                editor.clone().update(cx, |editor, cx| editor.retry_enrich(cx));
            }
        } else {
            self.show_command_error(
                if allow { "Access granted — run the command again".to_string() }
                else { "Access denied".to_string() },
                cx,
            );
        }
    }
```

- Banner copy (render site ~line 2619): destructure the tuple; text becomes:

```rust
    let msg = if cap.starts_with("net:") {
        format!("Plugin {} wants to access {}", plugin, &cap[4..])
    } else {
        format!("Plugin {} wants to read files in this workspace", plugin)
    };
```

- IMPORTANT grant-format consistency: Task 2's `net_domains` reads granted `"net:<d>"` / denied `"denied:net:<d>"`, and the Phase 2 workspace-read gate reads `"workspace-read"` — `resolve_consent` above emits exactly these (`cap` is `"workspace-read"` or `"net:<d>"`, denial prefixes `denied:`). The Phase 2 denial string changes from `"denied:workspace-read"`… it stays identical (`denied:` + cap). No migration needed.

- [ ] **Step 5: Settings round-trip test** (in `src/settings.rs` tests, alongside the existing plugin_grants coverage):

```rust
    #[test]
    fn net_domain_grants_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = load(dir.path());
        s.plugin_grants.insert(
            "url-title".into(),
            vec!["net:en.wikipedia.org".into(), "denied:net:evil.com".into()],
        );
        save(dir.path(), &s).unwrap();
        let s2 = load(dir.path());
        assert_eq!(s2.plugin_grants["url-title"], ["net:en.wikipedia.org", "denied:net:evil.com"]);
    }
```

(Check the actual `load`/`save` signatures in settings.rs first and mirror the existing tests' shape.)

- [ ] **Step 6: Full suite + build** — `cargo test && cargo build` → green.

- [ ] **Step 7: Commit** — `git commit -am "feat: async net enrich pass with per-domain consent and retry"`

---### Task 7: url-title plugin

**Files:**
- Create: `plugins/url-title/{Cargo.toml,src/lib.rs,plugin.toml}`
- Modify: `scripts/build_plugins.sh` (add `url-title` to the dist list)

**Interfaces:**
- Consumes: WIT 0.3 world (`../wit-v3`), `host_api::fetch`.

- [ ] **Step 1: Scaffold** — copy `plugins/template/` shape; Cargo.toml `name = "url-title"`, wit path `"../wit-v3"`.

- [ ] **Step 2: Write the in-crate failing tests** (pure logic in the guest crate, run on host target via `cargo test` inside the plugin dir):

```rust
#[cfg(test)]
mod tests {
    use super::{bare_https_url, extract_title};

    #[test]
    fn detects_only_single_bare_https_urls() {
        assert_eq!(bare_https_url("https://a.com/x"), Some("https://a.com/x"));
        assert_eq!(bare_https_url("  https://a.com/x \n"), Some("https://a.com/x"));
        assert_eq!(bare_https_url("http://a.com/x"), None); // http left alone
        assert_eq!(bare_https_url("see https://a.com"), None); // not bare
        assert_eq!(bare_https_url("hello"), None);
    }

    #[test]
    fn extracts_and_cleans_titles() {
        assert_eq!(
            extract_title("<html><head><title>Hi &amp; Bye</title></head></html>"),
            Some("Hi & Bye".to_string())
        );
        assert_eq!(
            extract_title("<TITLE>\n  Spaced\n  Out </TITLE>"),
            Some("Spaced Out".to_string())
        );
        assert_eq!(extract_title("<html>no title</html>"), None);
    }
}
```

- [ ] **Step 3: Verify FAIL** (`cd plugins/url-title && cargo test` — needs `[lib] crate-type = ["cdylib", "rlib"]` so tests link; the other guest crates use plain cdylib — add `"rlib"` here only).

- [ ] **Step 4: Implement**

```rust
wit_bindgen::generate!({ path: "../wit-v3", world: "extension" });

use supermd::extension::host_api;
use supermd::extension::types as t;

pub fn bare_https_url(text: &str) -> Option<&str> {
    let trimmed = text.trim();
    (trimmed.starts_with("https://")
        && !trimmed.contains(char::is_whitespace)
        && trimmed.len() > "https://".len())
    .then_some(trimmed)
}

pub fn extract_title(html: &str) -> Option<String> {
    let lower = html.to_lowercase();
    let start = lower.find("<title")?;
    let open_end = html[start..].find('>')? + start + 1;
    let close = lower[open_end..].find("</title")? + open_end;
    let raw = &html[open_end..close];
    let unescaped = raw
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'");
    let cleaned = unescaped.split_whitespace().collect::<Vec<_>>().join(" ");
    (!cleaned.is_empty()).then(|| cleaned.chars().take(200).collect())
}

struct Plugin;

impl Guest for Plugin {
    fn render_block(_: String, _: String, _: t::Theme) -> Result<String, String> {
        Err("unused".into())
    }
    fn run_command(_: String, _: t::CommandInput) -> Result<t::CommandOutput, String> {
        Err("unused".into())
    }
    fn render_inline(_: String, _: String) -> Result<String, String> {
        Err("unused".into())
    }
    fn format_document(d: String) -> Result<String, String> {
        Ok(d)
    }
    fn process_paste(text: String) -> Result<Option<String>, String> {
        let Some(url) = bare_https_url(&text) else { return Ok(None) };
        let resp = host_api::fetch(&host_api::FetchRequest {
            method: "GET".into(),
            url: url.into(),
            headers: vec![("accept".into(), "text/html".into())],
            body: None,
        })?; // consent-shaped errors propagate to raise the banner
        if resp.status != 200 {
            return Ok(None);
        }
        let html = String::from_utf8_lossy(&resp.body);
        Ok(extract_title(&html).map(|title| format!("[{title}]({url})")))
    }
    fn export_document(_: String, _: String, _: t::Theme) -> Result<Vec<ExportFile>, String> {
        Err("unused".into())
    }
}

export!(Plugin);
```

`plugin.toml`:

```toml
name = "url-title"
version = "0.1.0"
paste = true
capabilities = ["net"]
```

- [ ] **Step 5: Run in-crate tests** (PASS), build wasm: `bash scripts/build_plugins.sh` (dist list now includes url-title).

- [ ] **Step 6: Host-side proof** — add to `host_tests` a paste-through-fetch test using the fetcher-style mock (install url-title into a temp plugins dir? No — dist plugins aren't fixtures). Instead assert via the fetcher fixture that consent errors PROPAGATE through `process_paste`: the fetcher's `process_paste` returns `Ok(None)`, so reuse `format_document`-based ladder tests (already green). The url-title behavior is covered by its in-crate tests; the ladder is covered by fixtures. No new host test needed — note this explicitly in the commit message.

- [ ] **Step 7: Commit** — `git commit -am "feat: url-title enricher plugin (net-capable paste)"`

---

### Task 8: html-export plugin + wrap-up

**Files:**
- Create: `plugins/html-export/{Cargo.toml,src/lib.rs,plugin.toml}`
- Modify: `scripts/build_plugins.sh` (dist list), `plugins/template/README.md` (0.3 world, fetch import, exports table), `plugins/template/` wit path → `../wit-v3` + new stubs

**Interfaces:**
- Consumes: WIT 0.3, `pulldown-cmark` (guest dep, default features minus html? default is fine).

- [ ] **Step 1: In-crate failing test**

```rust
#[cfg(test)]
mod tests {
    use super::render_html;

    #[test]
    fn renders_standalone_themed_html() {
        let theme = super::t::Theme {
            background: "#101010".into(),
            surface: "#181818".into(),
            primary: "#4a9eff".into(),
            text: "#e0e0e0".into(),
            muted: "#808080".into(),
            border: "#303030".into(),
            font_body: "Helvetica".into(),
            dark: true,
        };
        let html = render_html("# Hello\n\nworld *em*\n", &theme);
        assert!(html.starts_with("<!doctype html>"));
        assert!(html.contains("<h1>Hello</h1>"));
        assert!(html.contains("<em>em</em>"));
        assert!(html.contains("#101010"), "theme background inlined");
        assert!(html.contains("Helvetica"));
        assert!(!html.contains("src=\"http"), "no external assets");
    }
}
```

(cdylib+rlib like url-title so tests link.)

- [ ] **Step 2: Verify FAIL, then implement**

```rust
wit_bindgen::generate!({ path: "../wit-v3", world: "extension" });

pub use supermd::extension::types as t;

pub fn render_html(markdown: &str, theme: &t::Theme) -> String {
    let parser = pulldown_cmark::Parser::new_ext(
        markdown,
        pulldown_cmark::Options::ENABLE_TABLES
            | pulldown_cmark::Options::ENABLE_STRIKETHROUGH
            | pulldown_cmark::Options::ENABLE_TASKLISTS,
    );
    let mut body = String::new();
    pulldown_cmark::html::push_html(&mut body, parser);
    format!(
        "<!doctype html>\n<html><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
         <style>\
         body{{background:{bg};color:{text};font-family:{font},sans-serif;\
         max-width:44rem;margin:2rem auto;padding:0 1rem;line-height:1.6}}\
         a{{color:{primary}}}\
         code,pre{{background:{surface};border:1px solid {border};border-radius:4px}}\
         code{{padding:.1em .3em}}pre{{padding:.8em;overflow-x:auto}}pre code{{border:0;padding:0}}\
         blockquote{{border-left:3px solid {border};margin-left:0;padding-left:1em;color:{muted}}}\
         table{{border-collapse:collapse}}td,th{{border:1px solid {border};padding:.3em .6em}}\
         </style></head><body>\n{body}</body></html>\n",
        bg = theme.background,
        text = theme.text,
        font = theme.font_body,
        primary = theme.primary,
        surface = theme.surface,
        border = theme.border,
        muted = theme.muted,
    )
}

struct Plugin;

impl Guest for Plugin {
    fn render_block(_: String, _: String, _: t::Theme) -> Result<String, String> {
        Err("unused".into())
    }
    fn run_command(_: String, _: t::CommandInput) -> Result<t::CommandOutput, String> {
        Err("unused".into())
    }
    fn render_inline(_: String, _: String) -> Result<String, String> {
        Err("unused".into())
    }
    fn format_document(d: String) -> Result<String, String> {
        Ok(d)
    }
    fn process_paste(_: String) -> Result<Option<String>, String> {
        Ok(None)
    }
    fn export_document(
        document: String,
        format: String,
        theme: t::Theme,
    ) -> Result<Vec<ExportFile>, String> {
        if format != "html" {
            return Err(format!("unknown format {format}"));
        }
        Ok(vec![ExportFile {
            path: "export.html".into(),
            bytes: render_html(&document, &theme).into_bytes(),
        }])
    }
}

export!(Plugin);
```

`plugin.toml`:

```toml
name = "html-export"
version = "0.1.0"

[[exports]]
id = "html"
name = "HTML"
extension = "html"
```

Cargo.toml deps: `wit-bindgen = "0.60"`, `pulldown-cmark = { version = "0.13", default-features = false, features = ["html"] }` (check the crate's feature list; plain default features are acceptable if "html" isn't a feature).

- [ ] **Step 3: Template refresh** — point `plugins/template/` at `../wit-v3`, add `export_document` stub + a commented `host_api::fetch` example, README section on `[[exports]]`, `capabilities = ["net"]`, and the consent model (domains prompted on first use).

- [ ] **Step 4: Build everything** — `bash scripts/build_plugins.sh && bash scripts/build_plugins.sh --fixtures && cargo test` → green.

- [ ] **Step 5: Manual smoke test** (macOS, dev build): install dist plugins to `~/.supermd/plugins`, run the app, open a markdown file → ⌘⇧P → "Export: HTML" → save dialog with `<stem>.html` → file opens in a browser with theme colors. Paste `https://en.wikipedia.org/wiki/Markdown` → consent banner → Allow → link becomes `[Markdown - Wikipedia](…)`.

- [ ] **Step 6: Commit** — `git commit -am "feat: html-export plugin, template 0.3 refresh"`

- [ ] **Step 7: Push and verify CI** — `git push`; the branch has no CI trigger (pushes only run on master), so run the local suite one final time and note that 3-OS verification lands with the eventual PR.

## Self-Review Notes

- Spec coverage: fetch ladder ✔ (Task 2 tests map 1:1 to the spec's testing strategy), export validation/writing ✔ (Task 3), palette+dialogs ✔ (Task 4), table split ✔ (Task 5), async enrich + consent + retry ✔ (Task 6), url-title ✔ (Task 7), html-export + template ✔ (Task 8), grants round-trip ✔ (Task 6 Step 5). Spec's "denial persisted → quiet" ✔ (Task 2 test). Deviation (pre-budgeted deadline) recorded in Global Constraints and mirrored from the spec's intent.
- Known compiler-guided points (bindgen module paths, `add_to_linker` generics, ureq 3 builder methods, gpui prompt signatures) are flagged inline where they occur; in each case the authoritative reference is named.
- Type consistency: `Vec<(String, Vec<u8>)>` is the export currency across Tasks 2→3→4; `(plugin, cap)` tuple across Task 6's editor/workspace boundary; grant strings `net:<d>` / `denied:net:<d>` consistent between Tasks 2 and 6.
