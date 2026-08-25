# Plugin Distribution Design — Seeding + On-Demand Install

**Date:** 2026-08-25
**Status:** Approved for planning
**Branch:** `plugin-distribution` (off master, post-v0.0.8)
**Target:** v0.0.9

## Purpose

Fix the out-of-box gap (fresh installs have zero plugins despite docs
advertising eleven) and make optional plugins installable from inside
the app. Two mechanisms: first-run **seeding** of a default set bundled
with the installers, and an **in-app installer** backed by a catalog in
the GitHub repo with per-plugin zips on the release. First-party only;
the third-party marketplace is a later cycle.

## The default set

Zero-capability, everyone-benefits plugins ship inside the app:

> dot, toc, emoji, tidy, todo-marks, word-count, csv-view, graphql

Capability-bearing or niche plugins stay optional:

> url-title (net), html-export, daily-note

## Seeding

- **Payload location** (installer-planted, read-only to the app):
  - macOS: `SuperMD.app/Contents/Resources/plugins/`
  - Linux: `<install prefix>/lib/supermd/plugins/` (deb) or
    `plugins/` next to the binary (tarball)
  - Windows: `plugins\` in the install directory
  - Resolution at runtime: a `bundled_plugins_dir()` in platform.rs
    that probes relative to the running executable
    (`../Resources/plugins`, `../lib/supermd/plugins`, `./plugins`),
    first hit wins; None in dev runs (target/…) is fine — dev seeds
    nothing.
- **Seeding pass** at startup, before `ExtensionHost::load`:
  - `~/.supermd/plugins/seeded.toml` records, per seeded plugin:
    name, version, and a content hash of what was seeded.
  - For each bundled plugin: if absent from the plugins dir AND not in
    the marker → copy + record. If absent but IN the marker → user
    deleted it: skip forever. If present and the installed content
    hash equals the marker's (user never touched it) and the bundled
    version is newer → replace + update marker. If present but hashes
    differ from the marker (user modified) → leave untouched.
  - Pure logic (`plan_seeding(bundled, installed, marker) -> Vec<Action>`)
    unit-tested; the I/O wrapper is thin.
- Failure anywhere degrades to "no seeding this launch" with an
  eprintln — never blocks startup.

## The catalog

`plugins/catalog.json`, committed on master, fetched from
`https://raw.githubusercontent.com/SuperJackfruitLabs/supermd/master/plugins/catalog.json`
only when the user opens the installer UI:

```json
{
  "catalog_version": 1,
  "plugins": [
    {
      "name": "url-title",
      "description": "Pasted links gain their page title (asks per-site consent)",
      "version": "0.1.0",
      "capabilities": ["net"],
      "download": "https://github.com/SuperJackfruitLabs/supermd/releases/download/v0.0.9/plugin-url-title.zip",
      "sha256": "<hex>"
    }
  ]
}
```

- Every entry in the catalog also exists in `plugins/` in-repo; a test
  keeps catalog names/versions in sync with the manifests (drift
  check, like nav.toml).
- The release workflow zips each dist plugin individually
  (`plugin-<name>.zip`, containing the plugin folder) alongside the
  existing `supermd-plugins.zip`, and a script step recomputes the
  `sha256` fields (release-time catalog patching happens in the
  workflow before the release publishes; the committed catalog carries
  the values for the latest released version).
- Download URLs MUST be under
  `https://github.com/SuperJackfruitLabs/supermd/` or
  `https://raw.githubusercontent.com/SuperJackfruitLabs/supermd/`;
  the app refuses anything else (org-pinning).

## In-app install

- Palette command **"Install Plugins…"** (always present, like Reload
  Plugins) opens a finder-family overlay listing catalog entries:
  name, description, capability tag rendered in user terms ("needs
  network access — asks per site"), and an "installed ✓" marker for
  names already in `~/.supermd/plugins` (dimmed, non-installable).
- Enter on an entry, on the background executor:
  1. Fetch the zip (org-pinned URL, HTTPS, 20 MB cap, 30 s timeout) —
     host-side reqwest/ureq, NOT the plugin fetch ladder (this is the
     app acting for the user, no consent banner; the action itself is
     the consent).
  2. Verify sha256 against the catalog.
  3. Unzip to a temp dir; validate: exactly one plugin folder,
     manifest parses, `needs_component`/grammar files present, name
     matches the catalog entry, no path traversal in zip entries.
  4. Move into `~/.supermd/plugins/<name>` (reject if it now exists),
     then run the Reload Plugins flow.
  5. Success strip "Installed <name>"; any failure → error strip,
     temp dir cleaned, plugins dir untouched.
- The overlay shows a short first-party notice ("Plugins are built and
  signed-for by the SuperMD project") — honest about the current trust
  model.
- No background catalog polling, no auto-updates of installed plugins
  (recorded for the registry cycle).

## Installer changes

- `bundle_macos.sh`: copy `dist/plugins/<default set>` into
  `Contents/Resources/plugins` before signing (signature covers them).
- deb packaging: install the default set under
  `/usr/lib/supermd/plugins`; tarball: `plugins/` beside the binary
  (install.sh copies alongside).
- Windows Inno Setup: add the `plugins\` dir to the install payload.
- `scripts/build_plugins.sh` unchanged; the release workflow gains the
  per-plugin zip + sha256 + catalog-patch steps.

## Testing

- `plan_seeding` truth table: fresh install seeds all; deleted seeded
  plugin never returns; user-modified plugin untouched on upgrade;
  untouched plugin upgraded when bundled version is newer; marker
  round-trip.
- `bundled_plugins_dir` probing (temp dirs mimicking each layout).
- Catalog: parse; org-pin rejection of foreign URLs; drift test
  catalog ↔ `plugins/*/plugin.toml` (names, versions, capabilities).
- Install flow pure parts: zip validation (traversal entries rejected,
  wrong-name rejected, manifest-invalid rejected), sha256 mismatch
  rejected — over fixture zips built in-test.
- Overlay: gpui suite in the established pattern (list from a local
  catalog fixture, installed-marker, install event emission) with the
  network call behind an injectable fetcher.
- Manual: fresh temp HOME → launch → defaults appear; delete one →
  relaunch → stays gone; Install Plugins… → url-title lands and works.

## Out of scope (recorded)

Third-party catalog entries and submission flow, plugin auto-update
notifications, uninstall UI, template starter-repo extraction, signature
verification beyond sha256 (rides on GitHub TLS + org pinning for now).
