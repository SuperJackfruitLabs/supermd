//! WebAssembly extension host: manifest discovery under
//! ~/.supermd/plugins/, wasmtime component instances, and the
//! capability contract (Phase 1: pure functions only). Plugin
//! failures are data, never crashes.

use std::path::{Path, PathBuf};

#[derive(Clone, Debug, serde::Deserialize)]
pub struct CommandInfo {
    pub id: String,
    pub title: String,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub struct InlineRule {
    pub id: String,
    pub pattern: String,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub struct DecorationRule {
    pub pattern: String,
    /// accent | muted | strong | highlight
    pub style: String,
}

#[derive(Clone, Debug, serde::Deserialize)]
struct ExportInfoFile {
    id: String,
    name: String,
    #[serde(default)]
    extension: Option<String>,
}

/// A tree-sitter grammar a plugin contributes (grammar.wasm +
/// highlights.scm alongside the manifest).
#[derive(Clone, Debug, serde::Deserialize)]
pub struct GrammarInfo {
    pub name: String,
    pub extensions: Vec<String>,
    /// Filename stem for this grammar's wasm/scm pair; required when a
    /// plugin ships more than one grammar.
    #[serde(default)]
    pub files: Option<String>,
}

/// Grammar-only plugins ship grammar.wasm + highlights.scm instead of
/// a component; anything with a component surface needs plugin.wasm.
/// Public alias for zip validation in the catalog installer.
pub fn manifest_needs_component(meta: &PluginMeta) -> bool {
    needs_component(meta)
}

fn needs_component(meta: &PluginMeta) -> bool {
    meta.grammars.is_empty()
        || !meta.commands.is_empty()
        || !meta.fences.is_empty()
        || !meta.inline.is_empty()
        || meta.formats
        || meta.paste
        || !meta.exports.is_empty()
        || !meta.viewers.is_empty()
        || !meta.widgets.is_empty()
        || !meta.templates.is_empty()
        || !meta.hooks.is_empty()
}

/// (wasm, scm) paths for a grammar declaration.
pub fn grammar_paths(dir: &Path, g: &GrammarInfo) -> (PathBuf, PathBuf) {
    match &g.files {
        Some(stem) => (dir.join(format!("{stem}.wasm")), dir.join(format!("{stem}.scm"))),
        None => (dir.join("grammar.wasm"), dir.join("highlights.scm")),
    }
}

/// A custom viewer: claims file extensions whose content the plugin
/// renders to markdown for the Reader.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct ViewerRule {
    pub extensions: Vec<String>,
}

/// A status-strip widget (text-only).
#[derive(Clone, Debug, serde::Deserialize)]
pub struct WidgetRule {
    pub id: String,
}

/// A file template ("New: <name>" in the palette).
#[derive(Clone, Debug, serde::Deserialize)]
pub struct TemplateRule {
    pub id: String,
    pub name: String,
}

/// An export format a plugin offers ("Export: <name>" in the palette).
#[derive(Clone, Debug)]
pub struct ExportInfo {
    pub id: String,
    pub name: String,
    /// Suggested filename extension for the save dialog.
    pub extension: String,
}

#[derive(Clone, Debug)]
pub struct PluginMeta {
    pub name: String,
    pub version: String,
    pub fences: Vec<String>,
    pub commands: Vec<CommandInfo>,
    pub inline: Vec<InlineRule>,
    pub decorations: Vec<DecorationRule>,
    pub formats: bool,
    pub paste: bool,
    pub exports: Vec<ExportInfo>,
    pub grammars: Vec<GrammarInfo>,
    pub viewers: Vec<ViewerRule>,
    pub widgets: Vec<WidgetRule>,
    pub templates: Vec<TemplateRule>,
    /// Hook events; only "save" is understood.
    pub hooks: Vec<String>,
    pub capabilities: Vec<String>,
    pub dir: PathBuf,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestFile {
    name: String,
    version: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    authors: Option<Vec<String>>,
    #[serde(default)]
    fences: Vec<String>,
    #[serde(default)]
    commands: Vec<CommandInfo>,
    #[serde(default)]
    inline: Vec<InlineRule>,
    #[serde(default)]
    decorations: Vec<DecorationRule>,
    #[serde(default)]
    formats: bool,
    #[serde(default)]
    paste: bool,
    #[serde(default)]
    exports: Vec<ExportInfoFile>,
    #[serde(default)]
    grammars: Vec<GrammarInfo>,
    #[serde(default)]
    viewers: Vec<ViewerRule>,
    #[serde(default)]
    widgets: Vec<WidgetRule>,
    #[serde(default)]
    templates: Vec<TemplateRule>,
    #[serde(default)]
    hooks: Vec<String>,
    /// Only "workspace-read" and "net" are understood; anything else
    /// is rejected so old builds give a clear error for newer manifests.
    #[serde(default)]
    capabilities: Vec<String>,
}

pub fn parse_manifest(dir: &Path, toml_src: &str) -> Result<PluginMeta, String> {
    let file: ManifestFile = toml::from_str(toml_src).map_err(|e| e.to_string())?;
    for cap in &file.capabilities {
        if !matches!(cap.as_str(), "workspace-read" | "net") {
            return Err(format!(
                "manifest declares capability `{cap}`, which this SuperMD version \
                 does not support (known: workspace-read, net)"
            ));
        }
    }
    for rule in file.inline.iter().map(|r| &r.pattern).chain(
        file.decorations.iter().map(|r| &r.pattern),
    ) {
        regex::Regex::new(rule).map_err(|e| format!("invalid pattern regex: {e}"))?;
    }
    for d in &file.decorations {
        if !matches!(d.style.as_str(), "accent" | "muted" | "strong" | "highlight") {
            return Err(format!("unknown decoration style `{}`", d.style));
        }
    }
    for h in &file.hooks {
        if h != "save" {
            return Err(format!("unknown hook event `{h}` (known: save)"));
        }
    }
    if file.grammars.len() > 1 && file.grammars.iter().any(|g| g.files.is_none()) {
        return Err(
            "plugins with multiple grammars must set files = \"<stem>\" on each".to_string()
        );
    }
    let _ = (file.description, file.authors);
    Ok(PluginMeta {
        name: file.name,
        version: file.version,
        fences: file.fences,
        commands: file.commands,
        inline: file.inline,
        decorations: file.decorations,
        formats: file.formats,
        paste: file.paste,
        exports: file
            .exports
            .into_iter()
            .map(|e| ExportInfo {
                id: e.id,
                name: e.name,
                extension: e.extension.unwrap_or_else(|| "txt".to_string()),
            })
            .collect(),
        grammars: file.grammars,
        viewers: file.viewers,
        widgets: file.widgets,
        templates: file.templates,
        hooks: file.hooks,
        capabilities: file.capabilities,
        dir: dir.to_path_buf(),
    })
}

/// Scan a plugins directory: each subdir needs plugin.toml + plugin.wasm.
/// Returns loaded metas and per-directory failures (never fatal).
pub fn discover(plugins_dir: &Path) -> (Vec<PluginMeta>, Vec<(PathBuf, String)>) {
    let mut loaded = Vec::new();
    let mut failures = Vec::new();
    let entries = match std::fs::read_dir(plugins_dir) {
        Ok(e) => e,
        Err(_) => return (loaded, failures), // no dir yet: nothing installed
    };
    for entry in entries.flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let manifest_path = dir.join("plugin.toml");
        if !manifest_path.exists() {
            continue; // stray dir, not a plugin
        }
        let result = std::fs::read_to_string(&manifest_path)
            .map_err(|e| e.to_string())
            .and_then(|src| parse_manifest(&dir, &src))
            .and_then(|meta| {
                if needs_component(&meta) && !dir.join("plugin.wasm").exists() {
                    return Err("plugin.wasm missing".to_string());
                }
                for g in &meta.grammars {
                    let (wasm, scm) = grammar_paths(&dir, g);
                    if !wasm.exists() || !scm.exists() {
                        return Err(format!(
                            "grammar `{}` needs {} and {}",
                            g.name,
                            wasm.file_name().unwrap_or_default().to_string_lossy(),
                            scm.file_name().unwrap_or_default().to_string_lossy(),
                        ));
                    }
                }
                Ok(meta)
            });
        match result {
            Ok(meta) => loaded.push(meta),
            Err(e) => failures.push((dir, e)),
        }
    }
    loaded.sort_by(|a, b| a.name.cmp(&b.name));
    (loaded, failures)
}

// ── wasmtime host ──────────────────────────────────────────────────────

wasmtime::component::bindgen!({
    path: "plugins/wit/extension.wit",
    world: "extension",
});

/// 0.2 bindings live in their own module; 0.1 plugins keep working
/// through the fallback in `with_instance`.
mod v2 {
    wasmtime::component::bindgen!({
        path: "plugins/wit-v2/extension.wit",
        world: "extension",
    });
}

/// 0.3 bindings: adds export-document and the host-api fetch import.
mod v3 {
    wasmtime::component::bindgen!({
        path: "plugins/wit-v3/extension.wit",
        world: "extension",
    });
}

/// 0.4 bindings: adds render-view, status-text, render-template, and
/// on-save.
mod v4 {
    wasmtime::component::bindgen!({
        path: "plugins/wit-v4/extension.wit",
        world: "extension",
    });
}

use supermd::extension::types as wit_types;

// `CommandOutput` and the other WIT types are generated by bindgen!
// above and re-used directly (variants: ReplaceDocument,
// ReplaceSelection, InsertAtCursor).

/// How long a single plugin call may run before the epoch deadline
/// interrupts it (epoch ticks every 500ms; 4 ticks ≈ 2s).
const EPOCH_TICK_MS: u64 = 500;
const CALL_DEADLINE_TICKS: u64 = 4;

// ── net: host-mediated fetch ──────────────────────────────────────────

pub struct TransportRequest {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
}

#[derive(Debug)]
pub struct TransportResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    /// Location target when the status is a redirect.
    pub redirect: Option<String>,
}

/// The only path to the network; production installs ureq, tests
/// inject a mock (and can assert it was never called).
pub type FetchTransport =
    std::sync::Arc<dyn Fn(&TransportRequest) -> Result<TransportResponse, String> + Send + Sync>;

const MAX_FETCHES_PER_CALL: u32 = 4;
const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
const MAX_REDIRECT_HOPS: u32 = 5;
/// 5s per fetch, in epoch ticks.
const FETCH_TIMEOUT_TICKS: u64 = 10;
/// Deadline for calls into net-capable plugins: the compute budget
/// plus the whole network budget. Pre-budgeted at call entry because
/// the generated host trait sees only HostState, not the store — a
/// slow server charges this allowance, never misread as a hang.
const NET_CALL_DEADLINE_TICKS: u64 =
    CALL_DEADLINE_TICKS + MAX_FETCHES_PER_CALL as u64 * FETCH_TIMEOUT_TICKS;

/// Per-call network state carried by the store.
struct NetCtx {
    /// Manifest declares `net`.
    declared: bool,
    /// Domains with a persisted grant / denial.
    granted: Vec<String>,
    denied: Vec<String>,
    transport: FetchTransport,
    fetches_used: u32,
}

/// Domain of an https URL, or None for any other scheme.
fn url_domain(url: &str) -> Option<&str> {
    url.strip_prefix("https://")?
        .split(['/', '?', '#'])
        .next()?
        .split(':')
        .next()
}

fn ureq_transport() -> FetchTransport {
    std::sync::Arc::new(|req: &TransportRequest| {
        let config = ureq::Agent::config_builder()
            .max_redirects(0)
            .http_status_as_error(false)
            .timeout_global(Some(std::time::Duration::from_secs(5)))
            .build();
        let agent: ureq::Agent = config.into();
        let result = match req.method.as_str() {
            "GET" => {
                let mut r = agent.get(&req.url);
                for (k, v) in &req.headers {
                    r = r.header(k, v);
                }
                r.call()
            }
            "POST" => {
                let mut r = agent.post(&req.url);
                for (k, v) in &req.headers {
                    r = r.header(k, v);
                }
                r.send(&req.body.clone().unwrap_or_default()[..])
            }
            other => return Err(format!("unsupported method {other}")),
        };
        let response = result.map_err(|e| e.to_string())?;
        let status = response.status().as_u16();
        let redirect = response
            .headers()
            .get("location")
            .and_then(|v| v.to_str().ok())
            .filter(|_| (300..400).contains(&status))
            .map(str::to_string);
        let headers = response
            .headers()
            .iter()
            .filter_map(|(k, v)| v.to_str().ok().map(|v| (k.to_string(), v.to_string())))
            .collect();
        use std::io::Read as _;
        let mut body = Vec::new();
        response
            .into_body()
            .into_reader()
            .take(MAX_RESPONSE_BYTES as u64 + 1)
            .read_to_end(&mut body)
            .map_err(|e| e.to_string())?;
        Ok(TransportResponse { status, headers, body, redirect })
    })
}

/// The whole net-enforcement ladder, shared by every world's host-api
/// impl. Returns (status, headers, body).
fn host_fetch(
    state: &mut HostState,
    method: &str,
    req_url: &str,
    req_headers: &[(String, String)],
    req_body: &Option<Vec<u8>>,
) -> Result<(u16, Vec<(String, String)>, Vec<u8>), String> {
    {
        let Some(net) = state.net.as_mut() else {
            return Err("net capability not declared".to_string());
        };
        if !net.declared {
            return Err("net capability not declared".to_string());
        }
        if net.fetches_used >= MAX_FETCHES_PER_CALL {
            return Err(format!("fetch limit ({MAX_FETCHES_PER_CALL} per call) exceeded"));
        }
        net.fetches_used += 1;
        let mut url = req_url.to_string();
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
                method: method.to_string(),
                url: url.clone(),
                headers: req_headers.to_vec(),
                body: req_body.clone(),
            })?;
            if let Some(target) = resp.redirect {
                // Redirect hops are bounded separately; the fetch
                // budget counts logical fetches, and every hop
                // re-enters the domain checks above.
                hops += 1;
                if hops > MAX_REDIRECT_HOPS {
                    return Err("too many redirects".to_string());
                }
                url = target;
                continue;
            }
            if resp.body.len() > MAX_RESPONSE_BYTES {
                return Err(format!("response exceeds {MAX_RESPONSE_BYTES} bytes"));
            }
            return Ok((resp.status, resp.headers, resp.body));
        }
    }
}

impl v3::supermd::extension::host_api::Host for HostState {
    fn fetch(
        &mut self,
        req: v3::supermd::extension::host_api::FetchRequest,
    ) -> Result<v3::supermd::extension::host_api::FetchResponse, String> {
        host_fetch(self, &req.method, &req.url, &req.headers, &req.body).map(
            |(status, headers, body)| v3::supermd::extension::host_api::FetchResponse {
                status,
                headers,
                body,
            },
        )
    }
}

impl v4::supermd::extension::host_api::Host for HostState {
    fn fetch(
        &mut self,
        req: v4::supermd::extension::host_api::FetchRequest,
    ) -> Result<v4::supermd::extension::host_api::FetchResponse, String> {
        host_fetch(self, &req.method, &req.url, &req.headers, &req.body).map(
            |(status, headers, body)| v4::supermd::extension::host_api::FetchResponse {
                status,
                headers,
                body,
            },
        )
    }
}

/// Per-store state: WASI with ZERO grants — no preopened dirs, no
/// env, no args, no network. Only stderr is inherited so plugin
/// panics are debuggable. This is the whole Phase 1 capability
/// surface.
struct HostState {
    ctx: wasmtime_wasi::WasiCtx,
    table: wasmtime_wasi::ResourceTable,
    /// Net context for 0.3 plugins; None for stores that can never
    /// fetch (v1/v2 worlds have no host-api import at all).
    net: Option<NetCtx>,
}

impl wasmtime_wasi::WasiView for HostState {
    fn ctx(&mut self) -> wasmtime_wasi::WasiCtxView<'_> {
        wasmtime_wasi::WasiCtxView { ctx: &mut self.ctx, table: &mut self.table }
    }
}

fn zero_grant_state() -> HostState {
    HostState {
        ctx: wasmtime_wasi::WasiCtxBuilder::new().inherit_stderr().build(),
        table: wasmtime_wasi::ResourceTable::new(),
        net: None,
    }
}

enum Bound {
    V1(wasmtime::Store<HostState>, Extension),
    V2(wasmtime::Store<HostState>, v2::Extension),
    V3(wasmtime::Store<HostState>, v3::Extension),
    V4(wasmtime::Store<HostState>, v4::Extension),
}

/// Host-side mirror of the wit template-context record.
pub struct TemplateContext {
    pub date: String,
    pub time: String,
    pub weekday: String,
    pub workspace: String,
}

struct LoadedPlugin {
    meta: PluginMeta,
    /// None for grammar-only plugins (no component surfaces).
    component: Option<wasmtime::component::Component>,
    /// Instantiated lazily; dropped after a trap so the next call
    /// gets a fresh store.
    instance: Option<Bound>,
}

pub struct ExtensionHost {
    engine: wasmtime::Engine,
    plugins: Vec<LoadedPlugin>,
    failures: Vec<(PathBuf, String)>,
    workspace_root: Option<PathBuf>,
    grants: std::collections::BTreeMap<String, Vec<String>>,
    transport: FetchTransport,
}

impl ExtensionHost {
    pub fn load(plugins_dir: &Path) -> Self {
        Self::load_with_target(plugins_dir, None)
    }

    /// `target` overrides the compilation target; `Some("pulley64")`
    /// compiles to Pulley bytecode and interprets it (no executable
    /// pages). None = native codegen.
    pub fn load_with_target(plugins_dir: &Path, target: Option<&str>) -> Self {
        let mut config = wasmtime::Config::new();
        config.epoch_interruption(true);
        if let Some(t) = target {
            config.target(t).expect("wasm target");
        }
        let engine = wasmtime::Engine::new(&config).expect("wasmtime engine");
        // Tick the epoch forever; deadlines are per-call offsets.
        {
            let engine = engine.clone();
            std::thread::Builder::new()
                .name("supermd-wasm-epoch".into())
                .spawn(move || loop {
                    std::thread::sleep(std::time::Duration::from_millis(EPOCH_TICK_MS));
                    engine.increment_epoch();
                })
                .expect("epoch thread");
        }

        let (metas, mut failures) = discover(plugins_dir);
        let mut plugins = Vec::new();
        for meta in metas {
            if !needs_component(&meta) {
                plugins.push(LoadedPlugin { meta, component: None, instance: None });
                continue;
            }
            match wasmtime::component::Component::from_file(&engine, meta.dir.join("plugin.wasm"))
            {
                Ok(component) => {
                    plugins.push(LoadedPlugin { meta, component: Some(component), instance: None })
                }
                Err(e) => failures.push((meta.dir.clone(), format!("compile: {e:#}"))),
            }
        }
        Self {
            engine,
            plugins,
            failures,
            workspace_root: None,
            grants: Default::default(),
            transport: ureq_transport(),
        }
    }

    /// Test hook; rebuilds instances so their stores pick it up.
    pub fn set_transport(&mut self, t: FetchTransport) {
        self.transport = t;
        for p in &mut self.plugins {
            p.instance = None;
        }
    }

    /// Workspace root used for workspace-read preopens.
    pub fn set_workspace_root(&mut self, root: Option<PathBuf>) {
        if self.workspace_root != root {
            self.workspace_root = root;
            // Instances carry preopens; rebuild on next call.
            for p in &mut self.plugins {
                p.instance = None;
            }
        }
    }

    /// Persisted capability grants; instances rebuild on change.
    pub fn set_grants(&mut self, grants: std::collections::BTreeMap<String, Vec<String>>) {
        if self.grants != grants {
            self.grants = grants;
            for p in &mut self.plugins {
                p.instance = None;
            }
        }
    }

    fn wants_workspace_read(&self, plugin: &str) -> bool {
        self.plugins
            .iter()
            .find(|p| p.meta.name == plugin)
            .is_some_and(|p| p.meta.capabilities.iter().any(|c| c == "workspace-read"))
    }

    fn granted(&self, plugin: &str, cap: &str) -> bool {
        self.grants
            .get(plugin)
            .is_some_and(|caps| caps.iter().any(|c| c == cap))
    }

    /// Granted (or denied) net domains for a plugin, from grant
    /// strings shaped "net:<domain>" / "denied:net:<domain>".
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

    /// Err when the plugin declares a capability that is not granted —
    /// checked host-side before any wasm runs.
    pub fn consent_gate(&self, plugin: &str) -> Result<(), String> {
        if self.wants_workspace_read(plugin) && !self.granted(plugin, "workspace-read") {
            return Err(format!("awaiting consent for workspace-read ({plugin})"));
        }
        Ok(())
    }

    pub fn plugins(&self) -> Vec<PluginMeta> {
        self.plugins.iter().map(|p| p.meta.clone()).collect()
    }

    pub fn failures(&self) -> &[(PathBuf, String)] {
        &self.failures
    }

    fn state_for(&self, plugin: &str) -> HostState {
        // Net context rides every store for a net-declaring plugin;
        // per-domain enforcement happens inside fetch.
        let net = self.plugins.iter().find(|p| p.meta.name == plugin).map(|p| NetCtx {
            declared: p.meta.capabilities.iter().any(|c| c == "net"),
            granted: self.net_domains(plugin, false),
            denied: self.net_domains(plugin, true),
            transport: self.transport.clone(),
            fetches_used: 0,
        });
        if self.wants_workspace_read(plugin) && self.granted(plugin, "workspace-read") {
            if let Some(root) = &self.workspace_root {
                let mut builder = wasmtime_wasi::WasiCtxBuilder::new();
                builder.inherit_stderr();
                if builder
                    .preopened_dir(root, "/workspace", wasmtime_wasi::FsPerms::ReadOnly)
                    .is_ok()
                {
                    return HostState {
                        ctx: builder.build(),
                        table: wasmtime_wasi::ResourceTable::new(),
                        net,
                    };
                }
            }
        }
        HostState { net, ..zero_grant_state() }
    }

    fn ensure_bound(&mut self, plugin: &str) -> Result<&mut Bound, String> {
        self.consent_gate(plugin)?;
        // A fresh store per instantiation attempt; a failed try may
        // leave partial state behind.
        let state_v4 = self.state_for(plugin);
        let state_v3 = self.state_for(plugin);
        let state_v2 = self.state_for(plugin);
        let Some(slot) = self.plugins.iter_mut().find(|p| p.meta.name == plugin) else {
            return Err(format!("no such plugin '{plugin}'"));
        };
        let Some(component) = slot.component.clone() else {
            return Err(format!("plugin '{plugin}' has no component (grammar-only)"));
        };
        if slot.instance.is_none() {
            let mut linker = wasmtime::component::Linker::new(&self.engine);
            // wasip2 std needs core WASI interfaces even for pure
            // code; the zero-grant ctx keeps the capability surface
            // empty (no fs, no env, no net). host-api is defined for
            // everyone but only 0.3 components import it.
            wasmtime_wasi::p2::add_to_linker_sync(&mut linker)
                .map_err(|e| format!("wasi link: {e:#}"))?;
            v3::supermd::extension::host_api::add_to_linker::<
                HostState,
                wasmtime::component::HasSelf<HostState>,
            >(&mut linker, |state| state)
            .map_err(|e| format!("host-api link: {e:#}"))?;
            v4::supermd::extension::host_api::add_to_linker::<
                HostState,
                wasmtime::component::HasSelf<HostState>,
            >(&mut linker, |state| state)
            .map_err(|e| format!("host-api v4 link: {e:#}"))?;
            // Try 0.4 down to 0.1, newest first.
            let mut store = wasmtime::Store::new(&self.engine, state_v4);
            store.set_epoch_deadline(CALL_DEADLINE_TICKS);
            match v4::Extension::instantiate(&mut store, &component, &linker) {
                Ok(instance) => {
                    slot.instance = Some(Bound::V4(store, instance));
                    return Ok(slot.instance.as_mut().expect("just instantiated"));
                }
                Err(_) => {}
            }
            let mut store = wasmtime::Store::new(&self.engine, state_v3);
            store.set_epoch_deadline(CALL_DEADLINE_TICKS);
            match v3::Extension::instantiate(&mut store, &component, &linker) {
                Ok(instance) => slot.instance = Some(Bound::V3(store, instance)),
                Err(_) => {
                    let mut store = wasmtime::Store::new(&self.engine, state_v2);
                    store.set_epoch_deadline(CALL_DEADLINE_TICKS);
                    match v2::Extension::instantiate(&mut store, &component, &linker) {
                        Ok(instance) => slot.instance = Some(Bound::V2(store, instance)),
                        Err(_) => {
                            // v1 plugins predate capabilities: zero-grant
                            let mut store =
                                wasmtime::Store::new(&self.engine, zero_grant_state());
                            store.set_epoch_deadline(CALL_DEADLINE_TICKS);
                            let instance =
                                Extension::instantiate(&mut store, &component, &linker)
                                    .map_err(|e| format!("link: {e:#}"))?;
                            slot.instance = Some(Bound::V1(store, instance));
                        }
                    }
                }
            }
        }
        Ok(slot.instance.as_mut().expect("just instantiated"))
    }

    fn poison(&mut self, plugin: &str) {
        if let Some(slot) = self.plugins.iter_mut().find(|p| p.meta.name == plugin) {
            slot.instance = None;
        }
    }

    fn with_instance<R>(
        &mut self,
        plugin: &str,
        call: impl FnOnce(&mut Bound) -> wasmtime::Result<R>,
    ) -> Result<R, String> {
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
        let bound = self.ensure_bound(plugin)?;
        let store = match bound {
            Bound::V1(store, _) => store,
            Bound::V2(store, _) => store,
            Bound::V3(store, _) => store,
            Bound::V4(store, _) => store,
        };
        store.set_epoch_deadline(ticks);
        // The fetch budget is per plugin call, not per store lifetime.
        if let Some(net) = store.data_mut().net.as_mut() {
            net.fetches_used = 0;
        }
        match call(bound) {
            Ok(v) => Ok(v),
            Err(e) => {
                // Trap or deadline: poison-drop the instance.
                self.poison(plugin);
                Err(format!("{e:#}"))
            }
        }
    }

    /// Blocking; call from the background executor only.
    pub fn render_block(
        &mut self,
        plugin: &str,
        lang: &str,
        source: &str,
        theme: &crate::diagram::DiagramTheme,
    ) -> Result<String, String> {
        let wit_theme = wit_types::Theme {
            background: theme.background.clone(),
            surface: theme.surface.clone(),
            primary: theme.primary.clone(),
            text: theme.text.clone(),
            muted: theme.muted.clone(),
            border: theme.border.clone(),
            font_body: theme.font_body.clone(),
            dark: theme.dark,
        };
        self.with_instance(plugin, |bound| match bound {
            Bound::V1(store, i) => i.call_render_block(store, lang, source, &wit_theme),
            Bound::V2(store, i) => {
                let t2 = v2::supermd::extension::types::Theme {
                    background: wit_theme.background.clone(),
                    surface: wit_theme.surface.clone(),
                    primary: wit_theme.primary.clone(),
                    text: wit_theme.text.clone(),
                    muted: wit_theme.muted.clone(),
                    border: wit_theme.border.clone(),
                    font_body: wit_theme.font_body.clone(),
                    dark: wit_theme.dark,
                };
                i.call_render_block(store, lang, source, &t2)
            }
            Bound::V3(store, i) => {
                let t3 = v3::supermd::extension::types::Theme {
                    background: wit_theme.background.clone(),
                    surface: wit_theme.surface.clone(),
                    primary: wit_theme.primary.clone(),
                    text: wit_theme.text.clone(),
                    muted: wit_theme.muted.clone(),
                    border: wit_theme.border.clone(),
                    font_body: wit_theme.font_body.clone(),
                    dark: wit_theme.dark,
                };
                i.call_render_block(store, lang, source, &t3)
            }
            Bound::V4(store, i) => {
                let t4 = v4::supermd::extension::types::Theme {
                    background: wit_theme.background.clone(),
                    surface: wit_theme.surface.clone(),
                    primary: wit_theme.primary.clone(),
                    text: wit_theme.text.clone(),
                    muted: wit_theme.muted.clone(),
                    border: wit_theme.border.clone(),
                    font_body: wit_theme.font_body.clone(),
                    dark: wit_theme.dark,
                };
                i.call_render_block(store, lang, source, &t4)
            }
        })?
        .map_err(|e| e)
    }

    /// Blocking; call from the background executor only.
    pub fn run_command(
        &mut self,
        plugin: &str,
        id: &str,
        document: &str,
        selection: std::ops::Range<usize>,
    ) -> Result<CommandOutput, String> {
        let input = wit_types::CommandInput {
            document: document.to_string(),
            selection_start: selection.start.min(u32::MAX as usize) as u32,
            selection_end: selection.end.min(u32::MAX as usize) as u32,
        };
        let out = self.with_instance(plugin, |bound| match bound {
            Bound::V1(store, i) => i.call_run_command(store, id, &input),
            Bound::V2(store, i) => {
                let in2 = v2::supermd::extension::types::CommandInput {
                    document: input.document.clone(),
                    selection_start: input.selection_start,
                    selection_end: input.selection_end,
                };
                i.call_run_command(store, id, &in2).map(|r| {
                    r.map(|o| {
                        use v2::supermd::extension::types::CommandOutput as O2;
                        match o {
                            O2::ReplaceDocument(s) => CommandOutput::ReplaceDocument(s),
                            O2::ReplaceSelection(s) => CommandOutput::ReplaceSelection(s),
                            O2::InsertAtCursor(s) => CommandOutput::InsertAtCursor(s),
                        }
                    })
                })
            }
            Bound::V3(store, i) => {
                let in3 = v3::supermd::extension::types::CommandInput {
                    document: input.document.clone(),
                    selection_start: input.selection_start,
                    selection_end: input.selection_end,
                };
                i.call_run_command(store, id, &in3).map(|r| {
                    r.map(|o| {
                        use v3::supermd::extension::types::CommandOutput as O3;
                        match o {
                            O3::ReplaceDocument(s) => CommandOutput::ReplaceDocument(s),
                            O3::ReplaceSelection(s) => CommandOutput::ReplaceSelection(s),
                            O3::InsertAtCursor(s) => CommandOutput::InsertAtCursor(s),
                        }
                    })
                })
            }
            Bound::V4(store, i) => {
                let in4 = v4::supermd::extension::types::CommandInput {
                    document: input.document.clone(),
                    selection_start: input.selection_start,
                    selection_end: input.selection_end,
                };
                i.call_run_command(store, id, &in4).map(|r| {
                    r.map(|o| {
                        use v4::supermd::extension::types::CommandOutput as O4;
                        match o {
                            O4::ReplaceDocument(s) => CommandOutput::ReplaceDocument(s),
                            O4::ReplaceSelection(s) => CommandOutput::ReplaceSelection(s),
                            O4::InsertAtCursor(s) => CommandOutput::InsertAtCursor(s),
                        }
                    })
                })
            }
        })?;
        out.map_err(|e| e)
    }

    /// 0.2-only surfaces; 0.1 plugins get a readable error.
    pub fn render_inline(
        &mut self,
        plugin: &str,
        pattern_id: &str,
        matched: &str,
    ) -> Result<String, String> {
        self.with_instance(plugin, |bound| match bound {
            Bound::V1(..) => Ok(Err("requires a 0.2 plugin".to_string())),
            Bound::V2(store, i) => i.call_render_inline(store, pattern_id, matched),
            Bound::V3(store, i) => i.call_render_inline(store, pattern_id, matched),
            Bound::V4(store, i) => i.call_render_inline(store, pattern_id, matched),
        })?
        .map_err(|e| e)
    }

    pub fn format_document(&mut self, plugin: &str, document: &str) -> Result<String, String> {
        self.with_instance(plugin, |bound| match bound {
            Bound::V1(..) => Ok(Err("requires a 0.2 plugin".to_string())),
            Bound::V2(store, i) => i.call_format_document(store, document),
            Bound::V3(store, i) => i.call_format_document(store, document),
            Bound::V4(store, i) => i.call_format_document(store, document),
        })?
        .map_err(|e| e)
    }

    pub fn process_paste(
        &mut self,
        plugin: &str,
        text: &str,
    ) -> Result<Option<String>, String> {
        self.with_instance(plugin, |bound| match bound {
            Bound::V1(..) => Ok(Err("requires a 0.2 plugin".to_string())),
            Bound::V2(store, i) => i.call_process_paste(store, text),
            Bound::V3(store, i) => i.call_process_paste(store, text),
            Bound::V4(store, i) => i.call_process_paste(store, text),
        })?
        .map_err(|e| e)
    }

    /// 0.3-only. Blocking; call from the background executor only.
    /// Returns (relative path, bytes) pairs — the caller owns
    /// validation and the destination.
    pub fn export_document(
        &mut self,
        plugin: &str,
        document: &str,
        format: &str,
        theme: &crate::diagram::DiagramTheme,
    ) -> Result<Vec<(String, Vec<u8>)>, String> {
        let t = theme.clone();
        self.with_instance(plugin, |bound| match bound {
            Bound::V1(..) | Bound::V2(..) => Ok(Err("requires a 0.3 plugin".to_string())),
            Bound::V3(store, i) => {
                let theme = v3::supermd::extension::types::Theme {
                    background: t.background.clone(),
                    surface: t.surface.clone(),
                    primary: t.primary.clone(),
                    text: t.text.clone(),
                    muted: t.muted.clone(),
                    border: t.border.clone(),
                    font_body: t.font_body.clone(),
                    dark: t.dark,
                };
                i.call_export_document(store, document, format, &theme).map(|r| {
                    r.map(|files| files.into_iter().map(|f| (f.path, f.bytes)).collect())
                })
            }
            Bound::V4(store, i) => {
                let theme = v4::supermd::extension::types::Theme {
                    background: t.background.clone(),
                    surface: t.surface.clone(),
                    primary: t.primary.clone(),
                    text: t.text.clone(),
                    muted: t.muted.clone(),
                    border: t.border.clone(),
                    font_body: t.font_body.clone(),
                    dark: t.dark,
                };
                i.call_export_document(store, document, format, &theme).map(|r| {
                    r.map(|files| files.into_iter().map(|f| (f.path, f.bytes)).collect())
                })
            }
        })?
        .map_err(|e| e)
    }

    /// 0.4-only. Blocking; call from the background executor only.
    pub fn render_view(
        &mut self,
        plugin: &str,
        filename: &str,
        content: &str,
    ) -> Result<String, String> {
        self.with_instance(plugin, |bound| match bound {
            Bound::V4(store, i) => i.call_render_view(store, filename, content),
            _ => Ok(Err("requires a 0.4 plugin".to_string())),
        })?
        .map_err(|e| e)
    }

    /// 0.4-only. Blocking; call from the background executor only.
    pub fn status_text(&mut self, plugin: &str, document: &str) -> Result<String, String> {
        self.with_instance(plugin, |bound| match bound {
            Bound::V4(store, i) => i.call_status_text(store, document),
            _ => Ok(Err("requires a 0.4 plugin".to_string())),
        })?
        .map_err(|e| e)
    }

    /// 0.4-only. Returns (relative filename, content); the caller owns
    /// validation and the write.
    pub fn render_template(
        &mut self,
        plugin: &str,
        id: &str,
        ctx: &TemplateContext,
    ) -> Result<(String, String), String> {
        let ctx = (ctx.date.clone(), ctx.time.clone(), ctx.weekday.clone(), ctx.workspace.clone());
        self.with_instance(plugin, |bound| match bound {
            Bound::V4(store, i) => {
                let wit_ctx = v4::TemplateContext {
                    date: ctx.0.clone(),
                    time: ctx.1.clone(),
                    weekday: ctx.2.clone(),
                    workspace: ctx.3.clone(),
                };
                i.call_render_template(store, id, &wit_ctx)
                    .map(|r| r.map(|f| (f.filename, f.content)))
            }
            _ => Ok(Err("requires a 0.4 plugin".to_string())),
        })?
        .map_err(|e| e)
    }

    /// 0.4-only pre-save transform.
    pub fn on_save(
        &mut self,
        plugin: &str,
        path: &str,
        document: &str,
    ) -> Result<Option<String>, String> {
        self.with_instance(plugin, |bound| match bound {
            Bound::V4(store, i) => i.call_on_save(store, path, document),
            _ => Ok(Err("requires a 0.4 plugin".to_string())),
        })?
        .map_err(|e| e)
    }
}

// ── export writing (host-owned; plugins never see paths) ─────────────

/// Exporter paths must stay relative and inside the chosen directory.
pub fn validate_export_paths(files: &[(String, Vec<u8>)]) -> Result<(), String> {
    if files.is_empty() {
        return Err("exporter returned no files".to_string());
    }
    for (path, _) in files {
        let p = Path::new(path);
        let bad = p.as_os_str().is_empty()
            || p.is_absolute()
            || p.components().any(|c| {
                matches!(
                    c,
                    std::path::Component::ParentDir
                        | std::path::Component::RootDir
                        | std::path::Component::Prefix(_)
                )
            });
        if bad {
            return Err(format!("exporter returned an unsafe path: {path}"));
        }
    }
    Ok(())
}

/// Where an export lands: the exact file the user picked (single-file
/// exports) or a user-picked directory (multi-file exports).
pub enum ExportDest {
    File(PathBuf),
    Dir(PathBuf),
}

pub fn write_export(files: &[(String, Vec<u8>)], dest: &ExportDest) -> Result<(), String> {
    validate_export_paths(files)?;
    match dest {
        ExportDest::File(path) => std::fs::write(path, &files[0].1).map_err(|e| e.to_string()),
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

// ── templates (host-owned writes, workspace-only) ────────────────────

/// Create a template's file under the workspace root, validating the
/// relative path. Never overwrites: (path, false) means it already
/// existed and the caller should just open it.
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

/// Context for render-template: wasm has no clock, so the host supplies
/// local date/time strings.
pub fn template_context(workspace: &str) -> TemplateContext {
    let now = chrono::Local::now();
    TemplateContext {
        date: now.format("%Y-%m-%d").to_string(),
        time: now.format("%H:%M").to_string(),
        weekday: now.format("%A").to_string(),
        workspace: workspace.to_string(),
    }
}

/// Tests that mutate the process-global contribution tables (or the
/// inline cache / grammar registry they cascade into) must serialize:
/// the test runner is multi-threaded and a parallel test's
/// refresh/cleanup would wipe state mid-assertion.
#[cfg(test)]
pub(crate) static TABLE_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
pub(crate) fn table_test_guard() -> std::sync::MutexGuard<'static, ()> {
    TABLE_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// Global handle; plugin calls lock briefly on the background executor.
pub struct ExtensionState(pub std::sync::Arc<std::sync::Mutex<ExtensionHost>>);

impl gpui::Global for ExtensionState {}

/// Snapshot of (plugin, version, claimed fences) for pure discovery
/// contexts (projection runs without cx access).
static FENCE_TABLE: std::sync::RwLock<Vec<(String, String, Vec<String>)>> =
    std::sync::RwLock::new(Vec::new());

pub fn set_fence_table(table: Vec<(String, String, Vec<String>)>) {
    *FENCE_TABLE.write().unwrap() = table;
}

pub fn fence_table() -> Vec<(String, String, Vec<String>)> {
    FENCE_TABLE.read().unwrap().clone()
}

static FORMAT_PLUGINS: std::sync::RwLock<Vec<String>> = std::sync::RwLock::new(Vec::new());
static PASTE_PLUGINS: std::sync::RwLock<Vec<String>> = std::sync::RwLock::new(Vec::new());
static ENRICH_PLUGINS: std::sync::RwLock<Vec<String>> = std::sync::RwLock::new(Vec::new());
/// (extension, plugin) — first plugin wins on overlap.
static VIEWER_TABLE: std::sync::RwLock<Vec<(String, String)>> =
    std::sync::RwLock::new(Vec::new());
static WIDGET_PLUGINS: std::sync::RwLock<Vec<String>> = std::sync::RwLock::new(Vec::new());
/// (plugin, template id, display name)
static TEMPLATE_ENTRIES: std::sync::RwLock<Vec<(String, String, String)>> =
    std::sync::RwLock::new(Vec::new());
static HOOK_PLUGINS: std::sync::RwLock<Vec<String>> = std::sync::RwLock::new(Vec::new());

pub fn set_surface_tables(metas: &[PluginMeta]) {
    let has_net = |m: &&PluginMeta| m.capabilities.iter().any(|c| c == "net");
    *FORMAT_PLUGINS.write().unwrap() =
        metas.iter().filter(|m| m.formats).map(|m| m.name.clone()).collect();
    // Paste plugins split by capability: net-free ones run on the
    // synchronous paste path; net-capable ones become async enrichers
    // (a network call must never block a paste).
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
    let mut viewers: Vec<(String, String)> = Vec::new();
    for m in metas {
        for rule in &m.viewers {
            for ext in &rule.extensions {
                if !viewers.iter().any(|(e, _)| e == ext) {
                    viewers.push((ext.clone(), m.name.clone()));
                }
            }
        }
    }
    *VIEWER_TABLE.write().unwrap() = viewers;
    *WIDGET_PLUGINS.write().unwrap() = metas
        .iter()
        .filter(|m| !m.widgets.is_empty())
        .map(|m| m.name.clone())
        .collect();
    *TEMPLATE_ENTRIES.write().unwrap() = metas
        .iter()
        .flat_map(|m| {
            m.templates
                .iter()
                .map(|t| (m.name.clone(), t.id.clone(), t.name.clone()))
        })
        .collect();
    *HOOK_PLUGINS.write().unwrap() = metas
        .iter()
        .filter(|m| m.hooks.iter().any(|h| h == "save"))
        .map(|m| m.name.clone())
        .collect();
}

pub fn viewer_for_extension(ext: &str) -> Option<String> {
    VIEWER_TABLE
        .read()
        .unwrap()
        .iter()
        .find(|(e, _)| e == ext)
        .map(|(_, p)| p.clone())
}

pub fn widget_plugins() -> Vec<String> {
    WIDGET_PLUGINS.read().unwrap().clone()
}

pub fn template_entries() -> Vec<(String, String, String)> {
    TEMPLATE_ENTRIES.read().unwrap().clone()
}

pub fn hook_plugins() -> Vec<String> {
    HOOK_PLUGINS.read().unwrap().clone()
}

pub fn format_plugins() -> Vec<String> {
    FORMAT_PLUGINS.read().unwrap().clone()
}

pub fn paste_plugins() -> Vec<String> {
    PASTE_PLUGINS.read().unwrap().clone()
}

pub fn enrich_plugins() -> Vec<String> {
    ENRICH_PLUGINS.read().unwrap().clone()
}

/// Apply a formatted result only if the document did not change while
/// the formatter ran.
pub fn apply_if_unchanged(snapshot: &str, current: &str, formatted: String) -> Option<String> {
    (snapshot == current).then_some(formatted)
}

/// Host-compiled decoration rules (regex validated at manifest parse).
pub struct CompiledDecoration {
    pub regex: regex::Regex,
    pub style: String,
}

static DECORATION_TABLE: std::sync::RwLock<Vec<CompiledDecoration>> =
    std::sync::RwLock::new(Vec::new());

pub fn set_decoration_table(rules: Vec<CompiledDecoration>) {
    *DECORATION_TABLE.write().unwrap() = rules;
}

pub fn with_decoration_table<R>(f: impl FnOnce(&[CompiledDecoration]) -> R) -> R {
    f(&DECORATION_TABLE.read().unwrap())
}

/// Host-compiled inline rules.
pub struct CompiledInline {
    pub plugin: String,
    pub id: String,
    pub regex: regex::Regex,
}

static INLINE_TABLE: std::sync::RwLock<Vec<CompiledInline>> = std::sync::RwLock::new(Vec::new());

pub fn set_inline_table(rules: Vec<CompiledInline>) {
    *INLINE_TABLE.write().unwrap() = rules;
}

pub fn with_inline_table<R>(f: impl FnOnce(&[CompiledInline]) -> R) -> R {
    f(&INLINE_TABLE.read().unwrap())
}

pub fn compile_inline(metas: &[PluginMeta]) -> Vec<CompiledInline> {
    metas
        .iter()
        .flat_map(|m| {
            m.inline.iter().filter_map(|r| {
                regex::Regex::new(&r.pattern).ok().map(|regex| CompiledInline {
                    plugin: m.name.clone(),
                    id: r.id.clone(),
                    regex,
                })
            })
        })
        .collect()
}

// ── inline cache + miss queue ─────────────────────────────────────────

type InlineKey = (String, String, String); // (plugin, pattern-id, matched)

/// Resolved inline replacements: Some(text) = render, None = permanent
/// failure (stay raw, don't re-ask). Plain statics: the lookup runs
/// inside restyle, which has no cx.
static INLINE_CACHE: std::sync::Mutex<Option<std::collections::HashMap<InlineKey, Option<String>>>> =
    std::sync::Mutex::new(None);
static INLINE_QUEUE: std::sync::Mutex<Vec<InlineKey>> = std::sync::Mutex::new(Vec::new());
static INLINE_GEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
const INLINE_CACHE_CAP: usize = 4096;

pub fn inline_lookup(plugin: &str, id: &str, matched: &str) -> Option<String> {
    let guard = INLINE_CACHE.lock().unwrap();
    guard
        .as_ref()?
        .get(&(plugin.to_string(), id.to_string(), matched.to_string()))?
        .clone()
}

/// True when the key is already resolved (even as a failure).
fn inline_resolved(key: &InlineKey) -> bool {
    INLINE_CACHE
        .lock()
        .unwrap()
        .as_ref()
        .is_some_and(|m| m.contains_key(key))
}

pub fn enqueue_inline(misses: Vec<(String, String, String)>) {
    let mut queue = INLINE_QUEUE.lock().unwrap();
    for key in misses {
        if !inline_resolved(&key) && !queue.contains(&key) {
            queue.push(key);
        }
    }
}

pub fn inline_generation() -> u64 {
    INLINE_GEN.load(std::sync::atomic::Ordering::Relaxed)
}

pub fn clear_inline_cache() {
    *INLINE_CACHE.lock().unwrap() = None;
    INLINE_QUEUE.lock().unwrap().clear();
    INLINE_GEN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

fn inline_insert(key: InlineKey, value: Option<String>) {
    let mut guard = INLINE_CACHE.lock().unwrap();
    let map = guard.get_or_insert_with(Default::default);
    if map.len() >= INLINE_CACHE_CAP {
        map.clear(); // simple pressure valve; deterministic values refill fast
    }
    map.insert(key, value);
    INLINE_GEN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

/// Rebuild all contribution tables from a loaded host.
pub fn refresh_tables(host: &mut ExtensionHost) {
    let metas = host.plugins();
    set_fence_table(
        metas
            .iter()
            .map(|p| (p.name.clone(), p.version.clone(), p.fences.clone()))
            .collect(),
    );
    set_decoration_table(compile_decorations(&metas));
    set_inline_table(compile_inline(&metas));
    set_surface_tables(&metas);
    clear_inline_cache();
    // Grammar plugins load into the highlight layer's registry;
    // failures join the load report like any other plugin problem.
    let grammar_specs: Vec<(String, PathBuf, GrammarInfo)> = metas
        .iter()
        .flat_map(|m| m.grammars.iter().map(|g| (m.name.clone(), m.dir.clone(), g.clone())))
        .collect();
    for (plugin, error) in crate::highlight::load_plugin_grammars(&grammar_specs) {
        host.failures.push((PathBuf::from(plugin), error));
    }
}

/// Startup task: resolve queued inline misses through the host on the
/// background executor and refresh windows when the cache advances.
pub fn start_inline_drainer(cx: &mut gpui::App) {
    let Some(state) = cx.try_global::<ExtensionState>().map(|s| s.0.clone()) else {
        return;
    };
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(100))
                .await;
            let batch: Vec<InlineKey> = {
                let mut q = INLINE_QUEUE.lock().unwrap();
                std::mem::take(&mut *q)
            };
            if batch.is_empty() {
                continue;
            }
            let host = state.clone();
            let resolved = cx
                .background_executor()
                .spawn(async move {
                    let mut out = Vec::new();
                    for key in batch {
                        let result = host
                            .lock()
                            .unwrap()
                            .render_inline(&key.0, &key.1, &key.2)
                            .ok();
                        out.push((key, result));
                    }
                    out
                })
                .await;
            for (key, value) in resolved {
                inline_insert(key, value);
            }
            if cx.update(|cx| cx.refresh_windows()).is_err() {
                break;
            }
        }
    })
    .detach();
}

/// Build the decoration table from loaded plugin metas.
pub fn compile_decorations(metas: &[PluginMeta]) -> Vec<CompiledDecoration> {
    metas
        .iter()
        .flat_map(|m| m.decorations.iter())
        .filter_map(|d| {
            regex::Regex::new(&d.pattern)
                .ok()
                .map(|regex| CompiledDecoration { regex, style: d.style.clone() })
        })
        .collect()
}

#[cfg(test)]
mod template_tests {
    use super::*;

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
        assert_eq!(ctx.date.len(), 10, "{}", ctx.date);
        assert_eq!(ctx.date.chars().nth(4), Some('-'));
        assert_eq!(ctx.time.len(), 5, "{}", ctx.time);
        assert!(!ctx.weekday.is_empty());
    }
}

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

    #[test]
    fn evil_export_paths_are_rejected() {
        let d = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/plugins");
        if !d.join("fetcher/plugin.wasm").exists() {
            eprintln!("SKIP: fixtures not built");
            return;
        }
        let mut host = ExtensionHost::load(&d);
        let files = host
            .export_document("fetcher", "d", "evil", &crate::diagram::DiagramTheme::default_light())
            .unwrap();
        assert!(validate_export_paths(&files).is_err());
    }
}

#[cfg(test)]
mod bench_backends {
    use super::*;
    use std::time::{Duration, Instant};

    /// Synthetic markdown with the features the bench plugins act on:
    /// headings (toc), prose (word-count/tidy), emoji shortcodes and
    /// calc expressions (inline), TODO marks (decorations).
    fn synth_doc(sections: usize) -> String {
        let mut s = String::from("# Benchmark Document\n\n");
        for i in 0..sections {
            s.push_str(&format!("## Section {i}\n\n"));
            s.push_str(
                "Lorem ipsum dolor sit amet, consectetur adipiscing elit -- \
                 sed do eiusmod tempor incididunt ut labore et dolore magna \
                 aliqua. \"Quoted prose\" with an ellipsis... and a :smile: \
                 shortcode. TODO: revisit this paragraph.\n\n",
            );
            s.push_str("A computed value {{2 km + 300 m}} sits inline.\n\n");
            s.push_str("- list item one\n- list item two\n\n");
        }
        s
    }

    fn median(mut v: Vec<Duration>) -> Duration {
        v.sort();
        v[v.len() / 2]
    }

    /// Time `f` after one warmup call (the first call instantiates).
    fn timed(iters: usize, mut f: impl FnMut() -> Result<(), String>) -> Result<Duration, String> {
        f()?;
        let mut samples = Vec::with_capacity(iters);
        for _ in 0..iters {
            let t = Instant::now();
            f()?;
            samples.push(t.elapsed());
        }
        Ok(median(samples))
    }

    /// Does the plugin host run under a hardened-runtime signature
    /// with NO JIT entitlements? Build, then:
    ///   codesign --force --options runtime --sign - <test-bin>
    ///   SUPERMD_PROBE_TARGET=pulley64 <test-bin> hardened_runtime_probe --ignored
    #[test]
    #[ignore = "run manually against a hardened-runtime-signed test binary"]
    fn hardened_runtime_probe() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("dist/plugins");
        if !dir.join("word-count/plugin.wasm").exists() {
            eprintln!("SKIP: plugins not built");
            return;
        }
        let target = std::env::var("SUPERMD_PROBE_TARGET").ok();
        eprintln!("probe target: {target:?}");
        let mut host = ExtensionHost::load_with_target(&dir, target.as_deref());
        let out = host.status_text("word-count", "hello world from the probe");
        eprintln!("status_text -> {out:?}");
        assert!(out.is_ok(), "plugin call failed: {out:?}");
    }

    #[test]
    #[ignore = "benchmark; run: cargo test --release bench_backends -- --ignored --nocapture"]
    fn pulley_vs_cranelift() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("dist/plugins");
        if !dir.join("word-count/plugin.wasm").exists() {
            eprintln!("SKIP: plugins not built (bash scripts/build_plugins.sh)");
            return;
        }

        let small = synth_doc(10);
        let large = synth_doc(400);
        let huge = synth_doc(3200);
        eprintln!(
            "doc sizes: small {} B, large {} B, huge {} B\n",
            small.len(),
            large.len(),
            huge.len()
        );

        for (label, target) in [("cranelift (native)", None), ("pulley64 (interp)", Some("pulley64"))]
        {
            let t0 = Instant::now();
            let mut host = ExtensionHost::load_with_target(&dir, target);
            let compile_all = t0.elapsed();
            let n = host.plugins().len();
            eprintln!("=== {label} ===");
            eprintln!("  compile {n} plugins       {:>10.1?}", compile_all);
            if !host.failures().is_empty() {
                eprintln!("  failures: {:?}", host.failures());
            }

            let cases: Vec<(&str, Box<dyn FnMut(&mut ExtensionHost) -> Result<(), String>>)> = vec![
                (
                    "word-count status (small)",
                    Box::new({
                        let d = small.clone();
                        move |h: &mut ExtensionHost| h.status_text("word-count", &d).map(|_| ())
                    }),
                ),
                (
                    "word-count status (large)",
                    Box::new({
                        let d = large.clone();
                        move |h: &mut ExtensionHost| h.status_text("word-count", &d).map(|_| ())
                    }),
                ),
                (
                    "calc render_inline",
                    Box::new(|h: &mut ExtensionHost| {
                        h.render_inline("calc", "expr", "2 km + 300 m").map(|_| ())
                    }),
                ),
                (
                    "emoji render_inline",
                    Box::new(|h: &mut ExtensionHost| {
                        h.render_inline("emoji", "shortcode", "tada").map(|_| ())
                    }),
                ),
                (
                    "tidy format (small)",
                    Box::new({
                        let d = small.clone();
                        move |h: &mut ExtensionHost| h.format_document("tidy", &d).map(|_| ())
                    }),
                ),
                (
                    "tidy format (large)",
                    Box::new({
                        let d = large.clone();
                        move |h: &mut ExtensionHost| h.format_document("tidy", &d).map(|_| ())
                    }),
                ),
                (
                    "tidy format (huge 1MB)",
                    Box::new({
                        let d = huge.clone();
                        move |h: &mut ExtensionHost| h.format_document("tidy", &d).map(|_| ())
                    }),
                ),
                (
                    "toc on_save (large)",
                    Box::new({
                        let d = large.clone();
                        move |h: &mut ExtensionHost| {
                            h.on_save("toc", "bench.md", &d).map(|_| ())
                        }
                    }),
                ),
            ];

            for (name, mut case) in cases {
                match timed(20, |
                | case(&mut host)) {
                    Ok(d) => eprintln!("  {name:<26} {:>10.1?}", d),
                    Err(e) => eprintln!("  {name:<26} ERR: {e}"),
                }
            }
            eprintln!();
        }
    }
}

#[cfg(test)]
mod host_tests {
    use super::*;

    fn fixtures_dir() -> Option<PathBuf> {
        let d = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/plugins");
        d.join("echo/plugin.wasm").exists().then_some(d)
    }

    #[test]
    fn echo_renderer_roundtrips() {
        let Some(dir) = fixtures_dir() else {
            eprintln!("SKIP: fixtures not built (scripts/build_plugins.sh --fixtures)");
            return;
        };
        let mut host = ExtensionHost::load(&dir);
        assert!(host.failures().is_empty(), "{:?}", host.failures());
        let svg = host
            .render_block(
                "echo",
                "echo-fixture",
                "hello",
                &crate::diagram::DiagramTheme::default_light(),
            )
            .unwrap();
        assert!(svg.contains("echo-fixture:hello"), "{svg}");
    }

    #[test]
    fn echo_command_roundtrips() {
        let Some(dir) = fixtures_dir() else { eprintln!("SKIP"); return; };
        let mut host = ExtensionHost::load(&dir);
        let out = host.run_command("echo", "echo.run", "doc", 0..0).unwrap();
        match out {
            CommandOutput::InsertAtCursor(s) => assert_eq!(s, "echo:echo.run"),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn panicking_plugin_returns_err_and_recovers() {
        let Some(dir) = fixtures_dir() else { eprintln!("SKIP"); return; };
        let mut host = ExtensionHost::load(&dir);
        let theme = crate::diagram::DiagramTheme::default_light();
        assert!(host.render_block("panic", "x", "y", &theme).is_err());
        assert!(host.render_block("echo", "l", "s", &theme).is_ok());
    }

    #[test]
    fn hanging_plugin_hits_deadline() {
        let Some(dir) = fixtures_dir() else { eprintln!("SKIP"); return; };
        let mut host = ExtensionHost::load(&dir);
        let theme = crate::diagram::DiagramTheme::default_light();
        let t0 = std::time::Instant::now();
        assert!(host.render_block("hang", "x", "y", &theme).is_err());
        assert!(t0.elapsed() < std::time::Duration::from_secs(10));
        // and the host recovers afterward
        assert!(host.render_block("echo", "l", "s", &theme).is_ok());
    }

    #[test]
    fn v2_surfaces_roundtrip_and_v1_gets_readable_error() {
        let Some(dir) = fixtures_dir() else { eprintln!("SKIP"); return; };
        let mut host = ExtensionHost::load(&dir);
        assert_eq!(host.render_inline("echo", "e", ":x:").unwrap(), "[e::x:]");
        assert_eq!(host.format_document("echo", "abc").unwrap(), "ABC");
        assert_eq!(host.process_paste("echo", "ab").unwrap(), Some("ba".to_string()));
        // panic fixture stayed on 0.1: new surfaces err readably,
        // old surfaces still trap-recover.
        let e = host.render_inline("panic", "e", "x").unwrap_err();
        assert!(e.contains("0.2"), "{e}");
    }

    #[test]
    fn reader_denied_without_grant() {
        let Some(dir) = fixtures_dir() else { eprintln!("SKIP"); return; };
        let mut host = ExtensionHost::load(&dir);
        let e = host.format_document("reader", "x").unwrap_err();
        assert!(e.contains("consent"), "{e}");
    }

    #[test]
    fn reader_reads_probe_with_grant_but_cannot_escape() {
        let Some(dir) = fixtures_dir() else { eprintln!("SKIP"); return; };
        let ws = tempfile::tempdir().unwrap();
        std::fs::write(ws.path().join("probe.txt"), "workspace contents").unwrap();
        // a file OUTSIDE the workspace that an escape would reach
        let parent = ws.path().parent().unwrap();
        let _ = std::fs::write(parent.join("outside.txt"), "secret");
        let mut host = ExtensionHost::load(&dir);
        host.set_workspace_root(Some(ws.path().to_path_buf()));
        let mut grants = std::collections::BTreeMap::new();
        grants.insert("reader".to_string(), vec!["workspace-read".to_string()]);
        host.set_grants(grants);
        let body = host.format_document("reader", "x").unwrap();
        assert!(body.contains("workspace contents"), "{body}");
        let escape = host.format_document("reader", "escape");
        match escape {
            Ok(s) => assert!(!s.contains("secret"), "preopen escape leaked: {s}"),
            Err(_) => {} // denied is the expected shape
        }
    }

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
            Ok(TransportResponse {
                status: 302,
                headers: vec![],
                body: vec![],
                redirect: Some("https://b.com/next".into()),
            }),
            Ok(TransportResponse {
                status: 200,
                headers: vec![],
                body: b"followed".to_vec(),
                redirect: None,
            }),
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

    #[test]
    fn dist_plugins_all_load_and_html_export_roundtrips() {
        let d = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("dist/plugins");
        if !d.join("html-export/plugin.wasm").exists() {
            eprintln!("SKIP: dist plugins not built (scripts/build_plugins.sh)");
            return;
        }
        let mut host = ExtensionHost::load(&d);
        assert!(host.failures().is_empty(), "{:?}", host.failures());
        let files = host
            .export_document(
                "html-export",
                "# Title\n\nbody\n",
                "html",
                &crate::diagram::DiagramTheme::default_light(),
            )
            .unwrap();
        assert_eq!(files.len(), 1);
        assert!(validate_export_paths(&files).is_ok());
        let html = String::from_utf8(files[0].1.clone()).unwrap();
        assert!(html.contains("<h1>Title</h1>"), "{html}");
    }

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
        // and probe's old surfaces work through the V4 binding
        assert_eq!(host.format_document("probe", "abc").unwrap(), "abc");
    }

    /// Every world binding runs its conversion arm for every surface,
    /// even when the plugin answers "unused" — and the 0.4 world's
    /// host-api import reaches the same fetch ladder as 0.3's.
    #[test]
    fn version_arms_convert_across_all_surfaces() {
        let Some(dir) = fixtures_dir() else { eprintln!("SKIP"); return; };
        let mut host = ExtensionHost::load(&dir);
        let theme = crate::diagram::DiagramTheme::default_light();
        for plugin in ["fetcher", "probe"] {
            assert!(host.render_block(plugin, "x", "y", &theme).is_err());
            assert!(host.run_command(plugin, "id", "doc", 0..0).is_err());
            assert!(host.render_inline(plugin, "p", "m").is_err());
            assert_eq!(host.process_paste(plugin, "text").unwrap(), None);
        }
        assert!(host.export_document("probe", "d", "nope", &theme).is_err());
        // v4 fetch: same ladder, same consent shape, then success.
        let (t, calls) = mock_transport(vec![Ok(TransportResponse {
            status: 200,
            headers: vec![],
            body: b"pong".to_vec(),
            redirect: None,
        })]);
        host.set_transport(t);
        let e = host
            .format_document("probe", "fetchv4:https://example.com/x")
            .unwrap_err();
        assert!(e.contains("consent required: example.com"), "{e}");
        assert!(calls.lock().unwrap().is_empty());
        let mut grants = std::collections::BTreeMap::new();
        grants.insert("probe".to_string(), vec!["net:example.com".to_string()]);
        host.set_grants(grants);
        assert_eq!(
            host.format_document("probe", "fetchv4:https://example.com/x").unwrap(),
            "v4 status=200 body=pong"
        );
    }

    #[test]
    fn ureq_transport_rejects_unknown_methods_without_network() {
        let t = ureq_transport();
        let e = t(&TransportRequest {
            method: "PATCH".into(),
            url: "https://example.com".into(),
            headers: vec![],
            body: None,
        })
        .unwrap_err();
        assert!(e.contains("PATCH"), "{e}");
    }

    #[test]
    fn unknown_plugin_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let mut host = ExtensionHost::load(dir.path());
        assert!(host
            .render_block("ghost", "x", "y", &crate::diagram::DiagramTheme::default_light())
            .is_err());
    }
}

#[cfg(test)]
mod drainer_tests {
    use super::*;
    use gpui::TestAppContext;

    /// The background drainer resolves queued inline misses through the
    /// host and publishes them to the cache (bumping the generation).
    #[gpui::test]
    fn drainer_resolves_queued_inline_misses(cx: &mut TestAppContext) {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/plugins");
        if !dir.join("echo/plugin.wasm").exists() {
            eprintln!("SKIP: fixtures not built");
            return;
        }
        let _tables = table_test_guard();
        let host = ExtensionHost::load(&dir);
        cx.update(|cx| {
            cx.set_global(ExtensionState(std::sync::Arc::new(std::sync::Mutex::new(host))));
        });
        clear_inline_cache();
        let gen_before = inline_generation();
        enqueue_inline(vec![(
            "echo".to_string(),
            "cov".to_string(),
            ":drain:".to_string(),
        )]);
        // Re-enqueueing an unresolved key is a no-op (dedup path).
        enqueue_inline(vec![(
            "echo".to_string(),
            "cov".to_string(),
            ":drain:".to_string(),
        )]);
        cx.update(|cx| start_inline_drainer(cx));
        for _ in 0..50 {
            cx.executor().advance_clock(std::time::Duration::from_millis(60));
            cx.run_until_parked();
            if inline_lookup("echo", "cov", ":drain:").is_some() {
                break;
            }
        }
        assert_eq!(
            inline_lookup("echo", "cov", ":drain:"),
            Some("[cov::drain:]".to_string())
        );
        assert!(inline_generation() > gen_before, "cache generation advanced");
        // Resolved keys are not re-queued.
        enqueue_inline(vec![(
            "echo".to_string(),
            "cov".to_string(),
            ":drain:".to_string(),
        )]);
        assert!(INLINE_QUEUE.lock().unwrap().is_empty());
        clear_inline_cache();
    }
}

#[cfg(test)]
mod manifest_tests {
    use super::*;

    #[test]
    fn manifest_parses_contributions() {
        let m = parse_manifest(
            Path::new("/p/dot"),
            r#"
name = "dot"
version = "0.1.0"
fences = ["dot", "graphviz"]
[[commands]]
id = "dot.about"
title = "About Dot"
"#,
        )
        .unwrap();
        assert_eq!(m.name, "dot");
        assert_eq!(m.fences, ["dot", "graphviz"]);
        assert_eq!(m.commands[0].id, "dot.about");
        assert_eq!(m.commands[0].title, "About Dot");
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
    fn net_capability_is_accepted() {
        let m = parse_manifest(
            Path::new("/p/x"),
            "name=\"x\"\nversion=\"0\"\ncapabilities=[\"net\"]\n",
        )
        .unwrap();
        assert_eq!(m.capabilities, ["net"]);
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

    #[test]
    fn apply_if_unchanged_guards_generation() {
        assert_eq!(apply_if_unchanged("a", "a", "A".into()), Some("A".into()));
        assert_eq!(apply_if_unchanged("a", "ab", "A".into()), None);
    }

    #[test]
    fn workspace_read_capability_is_accepted() {
        let m = parse_manifest(
            Path::new("/p/x"),
            "name=\"x\"\nversion=\"0\"\ncapabilities=[\"workspace-read\"]\n",
        )
        .unwrap();
        assert_eq!(m.capabilities, ["workspace-read"]);
    }

    /// One test, sequential scenarios: the surface tables are process
    /// globals and parallel tests would race them.
    #[test]
    fn surface_tables_fill_from_metas() {
        let _tables = table_test_guard();
        // paste plugins split by net capability
        let sync_meta = parse_manifest(
            Path::new("/p/tidy"),
            "name=\"tidy-split-test\"\nversion=\"0\"\npaste=true\n",
        )
        .unwrap();
        let net_meta = parse_manifest(
            Path::new("/p/url-title"),
            "name=\"url-split-test\"\nversion=\"0\"\npaste=true\ncapabilities=[\"net\"]\n",
        )
        .unwrap();
        set_surface_tables(&[sync_meta, net_meta]);
        assert_eq!(paste_plugins(), ["tidy-split-test"]);
        assert_eq!(enrich_plugins(), ["url-split-test"]);

        phase5_surfaces_parse_and_fill_tables_scenario();
    }

    #[test]
    fn phase2_surfaces_parse() {
        let m = parse_manifest(
            Path::new("/p/x"),
            r#"
name = "x"
version = "0"
formats = true
paste = true
[[inline]]
id = "e"
pattern = ":([a-z]+):"
[[decorations]]
pattern = "\\b(TODO)\\b"
style = "accent"
"#,
        )
        .unwrap();
        assert!(m.formats && m.paste);
        assert_eq!(m.inline[0].id, "e");
        assert_eq!(m.decorations[0].style, "accent");
    }

    #[test]
    fn invalid_decoration_regex_fails_discover() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("badre");
        std::fs::create_dir_all(&p).unwrap();
        std::fs::write(
            p.join("plugin.toml"),
            "name=\"badre\"\nversion=\"0\"\n[[decorations]]\npattern=\"([\"\nstyle=\"accent\"\n",
        )
        .unwrap();
        std::fs::write(p.join("plugin.wasm"), b"stub").unwrap();
        let (ok, fail) = discover(dir.path());
        assert!(ok.is_empty());
        assert_eq!(fail.len(), 1);
        assert!(fail[0].1.contains("regex") || fail[0].1.contains("pattern"), "{}", fail[0].1);
    }

    #[test]
    fn discover_collects_good_and_bad() {
        let dir = tempfile::tempdir().unwrap();
        let good = dir.path().join("good");
        std::fs::create_dir_all(&good).unwrap();
        std::fs::write(
            good.join("plugin.toml"),
            "name=\"good\"\nversion=\"1\"\nfences=[\"x\"]\n",
        )
        .unwrap();
        std::fs::write(good.join("plugin.wasm"), b"stub").unwrap();
        let bad = dir.path().join("bad");
        std::fs::create_dir_all(&bad).unwrap();
        std::fs::write(bad.join("plugin.toml"), "not toml [").unwrap();
        let (ok, fail) = discover(dir.path());
        assert_eq!(ok.len(), 1);
        assert_eq!(ok[0].name, "good");
        assert_eq!(fail.len(), 1);
    }

    fn phase5_surfaces_parse_and_fill_tables_scenario() {
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
        assert_eq!(
            template_entries(),
            [("p5".to_string(), "daily".to_string(), "Daily Note".to_string())]
        );
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
        std::fs::write(
            p.join("plugin.toml"),
            "name=\"w\"\nversion=\"0\"\n[[widgets]]\nid=\"x\"\n",
        )
        .unwrap();
        let (ok, fail) = discover(dir.path());
        assert!(ok.is_empty());
        assert!(fail[0].1.contains("plugin.wasm"), "{}", fail[0].1);
    }

    #[test]
    fn grammars_parse_and_relax_wasm_requirement() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("g");
        std::fs::create_dir_all(&p).unwrap();
        std::fs::write(
            p.join("plugin.toml"),
            "name=\"g\"\nversion=\"0\"\n[[grammars]]\nname=\"graphql\"\nextensions=[\"graphql\",\"gql\"]\n",
        )
        .unwrap();
        std::fs::write(p.join("grammar.wasm"), b"\0asm").unwrap();
        std::fs::write(p.join("highlights.scm"), "(comment) @comment").unwrap();
        let (ok, fail) = discover(dir.path());
        assert_eq!(fail.len(), 0, "{fail:?}");
        assert_eq!(ok.len(), 1);
        assert_eq!(ok[0].grammars[0].name, "graphql");
        assert_eq!(ok[0].grammars[0].extensions, ["graphql", "gql"]);
    }

    #[test]
    fn grammar_missing_files_fails_discover() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("g");
        std::fs::create_dir_all(&p).unwrap();
        std::fs::write(
            p.join("plugin.toml"),
            "name=\"g\"\nversion=\"0\"\n[[grammars]]\nname=\"x\"\nextensions=[\"x\"]\n",
        )
        .unwrap();
        let (ok, fail) = discover(dir.path());
        assert!(ok.is_empty());
        assert_eq!(fail.len(), 1);
        assert!(fail[0].1.contains("grammar"), "{}", fail[0].1);
    }

    #[test]
    fn multi_grammar_requires_files_stem() {
        let two = r#"
name = "g"
version = "0"
[[grammars]]
name = "a"
extensions = ["a"]
[[grammars]]
name = "b"
extensions = ["b"]
"#;
        let err = parse_manifest(Path::new("/p/g"), two).unwrap_err();
        assert!(err.contains("files"), "{err}");
    }

    #[test]
    fn discover_requires_wasm_file() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("nowasm");
        std::fs::create_dir_all(&p).unwrap();
        std::fs::write(p.join("plugin.toml"), "name=\"n\"\nversion=\"1\"\n").unwrap();
        let (ok, fail) = discover(dir.path());
        assert!(ok.is_empty());
        assert_eq!(fail.len(), 1);
        assert!(fail[0].1.contains("plugin.wasm"));
    }
}
