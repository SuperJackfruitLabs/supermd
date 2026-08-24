# Extensions Phase 3 Design — Workspace & Data (fs-write + net)

**Date:** 2026-08-24
**Status:** Approved for planning
**Program:** `2026-08-24-extensions-roadmap.md` (Phase 3 of 5)
**Branch:** continues `extensions-phase1` (Phases 1–2 unmerged; this
phase builds directly on their runtime)

## Purpose

Ship the two capability-gated data surfaces: **exporters** (fs-write
via user-picked-path handles — the plugin never sees a path) and
**net** (host-mediated HTTPS fetch with per-domain consent). Plus the
async **enricher** pass that lets net plugins improve a paste without
ever blocking the UI. First-party: HTML exporter, URL-title enricher.
Importers (D2) and publishers (D3) get their WIT surface from this
phase but ship as third-party targets later.

## WIT evolution — `supermd:extension@0.3.0`

A third world; the host binds 0.1, 0.2, and 0.3 with per-plugin
fallback (`Bound::{V1,V2,V3}`), so all earlier plugins keep working
unchanged. Two changes:

```wit
interface host-api {
    record fetch-request {
        method: string,          // "GET" | "POST"
        url: string,             // https only
        headers: list<tuple<string, string>>,
        body: option<list<u8>>,
    }
    record fetch-response {
        status: u16,
        headers: list<tuple<string, string>>,
        body: list<u8>,
    }
    /// Host-mediated HTTPS. Errors are strings; a consent-shaped
    /// error ("consent required: <domain>") drives the banner flow.
    fetch: func(req: fetch-request) -> result<fetch-response, string>;
}

world extension {  // @0.3.0 — the 0.2 exports plus:
    import host-api;

    record export-file {
        path: string,            // relative; host validates
        bytes: list<u8>,
    }
    /// Produce the files for one export format. One entry = the host
    /// shows a save dialog; several = a directory picker.
    export export-document: func(document: string, format: string,
        theme: theme) -> result<list<export-file>, string>;
}
```

This is the first world with an **import**: the 0.3 `bindgen!` block
generates a host trait the `ExtensionHost` implements. 0.1/0.2 stores
link no host functions (unchanged). The theme parameter lets exporters
match the app's appearance (the HTML exporter inlines it as CSS).

## Manifest additions

```toml
# Export formats — palette command auto-added: "Export: <name>".
[[exports]]
id = "html"          # passed to export-document as `format`
name = "HTML"
extension = "html"   # suggested filename extension for the save dialog

capabilities = ["net"]   # now accepted alongside "workspace-read"
```

`capabilities = ["net"]` by itself grants nothing: every domain needs
its own persisted per-domain grant, prompted on first use. Unknown
capabilities still reject the plugin at load.

## fs-write: the plugin never sees a path

There is **no WASI filesystem write grant anywhere** — "fs-write" is
purely a host-side flow, which keeps the sandbox proof trivial:

1. User runs "Export: HTML" from the palette.
2. Host calls `export-document` on the background executor with the
   current document, format id, and active theme.
3. Host validates every returned `path`: reject absolute paths and any
   `..` component (per-plugin failure, nothing written).
4. One file → `prompt_for_new_path` seeded with `<doc stem>.<extension>`;
   the returned file takes the user's chosen name. Several files → a
   directory picker; files are written under it at their relative
   paths (creating subdirectories).
5. Cancel in the dialog = clean no-op.

Export runs before the dialog so the host knows single vs multi and so
plugin failures surface before the user picks a location.

## net: host-mediated fetch with per-domain consent

The `fetch` import is the only network path. Host enforcement, in
order, per call:

- Plugin's manifest lacks `net` → `Err("net capability not declared")`,
  and the transport is **never invoked** (exit criterion).
- URL is not `https://` → error.
- Domain has a persisted denial → error (quiet; asked once).
- Domain has no grant → `Err("consent required: <domain>")` — the same
  consent-shaped error path Phase 2 built for workspace-read; the
  banner reads "Plugin <name> wants to access <domain> — [Allow]
  [Deny]". Allow persists `"net:<domain>"` in `plugin_grants` and
  retries the originating call; Deny persists the refusal.
- Granted → the host performs the request via `ureq` with: 5 s
  timeout, 2 MB response cap, redirects followed only within granted
  domains (a redirect elsewhere is an error, not a prompt), and at
  most **4 fetches per plugin call**.

Grants live in the existing `settings.plugin_grants` map (values now
mix `"workspace-read"` and `"net:<domain>"`), persist across restarts,
and are revocable the same way workspace-read is.

**Deadline interaction:** epoch ticks keep the 2 s compute cap, but
each `fetch` entry extends the store's epoch deadline by the fetch
timeout (10 ticks), so a slow server is charged to the network budget,
not misread as a plugin hang. Compute time between fetches stays
capped.

**Testability:** the transport is a host-side function value
(`Arc<dyn Fn(FetchRequest) -> Result<FetchResponse, String>>`) on the
`ExtensionHost`; production installs the ureq transport, tests inject
a mock and can assert it was never called.

## Enrichers: async post-paste (net plugins only)

A network call inside the synchronous paste path would freeze the UI
for up to 5 s, so paste plugins split by capability:

- Plugins **without** `net`: the Phase 2 synchronous first-Some-wins
  pass, unchanged.
- Plugins **with** `net`: excluded from the sync pass. After the paste
  lands, the host runs their `process-paste` on the background
  executor with the pasted text; a `Some(replacement)` is applied only
  if the document generation hasn't moved since the paste (the
  formatter's `apply_if_unchanged` guard), replacing the pasted range
  in one undo step. Generation moved → result discarded silently
  (recorded honest limit: typing immediately after pasting forfeits
  enrichment).
- A consent-shaped error from the enrich call raises the banner; Allow
  retries the enrichment under the same generation guard.

## First-party plugins (Phase 3)

- **`plugins/html-export/`** — exporter (`pulldown-cmark` compiled to
  wasm): one standalone HTML file, theme colors and fonts inlined as
  CSS, no external assets. Proves the single-file dialog path.
- **`plugins/url-title/`** — enricher (`capabilities = ["net"]`,
  `paste = true`): pasted text that is exactly one bare http(s) URL →
  fetch (host upgrades nothing; http URLs are left alone), parse
  `<title>`, return `[Title](url)`. Anything else → `None`. Drives
  the per-domain consent banner naturally, one domain at a time.
- Fixture: **`fixtures/fetcher`** — declares `net`, echoes whatever
  the fetch transport returns (status + body) so host tests can prove
  the whole enforcement ladder. The existing no-capability fixtures
  prove the negative.

## Error handling

Same contract as Phases 1–2: plugin failures are data. Export failure
or invalid returned paths → per-plugin error surfaced like a command
failure, nothing written. Fetch errors are `Err` strings inside the
plugin — it can degrade (url-title returns `None`; the paste stays a
bare URL). Enrichment failures are silent; the pasted text is already
correct. Dialog cancel is not an error.

## Testing strategy

- Manifest: `[[exports]]` parsing; `net` accepted; unknown capability
  still rejects; exports without an `extension` default sensibly ("txt").
- Fetch ladder (mock transport, fetcher fixture): no `net` declared →
  Err and transport never invoked; https-only; ungranted → consent-
  shaped Err; denial persisted → quiet Err; granted → response
  roundtrips through the plugin; redirect to an ungranted domain
  blocked; response over 2 MB rejected; fifth fetch in one call
  rejected.
- Grants: `"net:<domain>"` persists in settings round-trip; revocation
  blocks the next call.
- Export: path validation rejects absolute and `..` paths; single vs
  multi file classification; write-under-directory creates subdirs
  (pure fs test, no dialog); html-export output contains theme colors
  and the rendered document (in-crate test).
- url-title in-crate: bare-URL detection (whitespace-trimmed, single
  token), title extraction incl. entities, non-URL paste → `None`.
- Enrich pass: generation-moved discards result; consent retry
  applies after grant (host-level test driving the guard directly).

## Out of scope (recorded)

Importers (D2) and publishers (D3) — surface exists, plugins are
third-party targets; POST is specified in `fetch-request` for their
benefit but no first-party plugin uses it. PDF export (needs a wasm
PDF story). `net:*` wildcard grants. Streaming/chunked responses.
Grammars (Phase 4), UI surfaces (Phase 5).
