# Shipping to the Mac App Store

How SuperMD's sandboxed build gets signed, packaged and submitted — and the
things that cost a day to discover the first time. Written after the first
submission, while it was all still fresh.

Account-specific values (key ids, the app's Apple ID) are deliberately not
here; they live with the credentials. Everything below is the process.

## What the `mas` build is

`--no-default-features --features mas` selects three swaps:

- the plugin host interprets Pulley bytecode instead of JIT-compiling, so the
  bundle needs **no** `com.apple.security.cs.*` entitlements
- the wasm grammar surface compiles out, dropping tree-sitter's wasmtime 24
  (the second JIT) and with it GraphQL highlighting
- the in-app plugin catalog gives way to Import Plugin… and a `supermd://`
  handoff, so the app never browses a storefront

Plus security-scoped bookmarks, `NSFileManager` trashing, `NSWorkspace`
reveal, and no process spawns.

## Building and submitting

```sh
bash scripts/build_plugins.sh

APP_IDENTITY="Apple Distribution: <Org> (<TEAMID>)" \
INSTALLER_IDENTITY="3rd Party Mac Developer Installer: <Org> (<TEAMID>)" \
PROFILE=/path/to/MacAppStore.provisionprofile \
bash scripts/bundle_mas.sh <version>

codesign -d --entitlements - dist/SuperMD.app   # 6 keys, and no cs.* entries

xcrun altool --validate-app -f dist/SuperMD-mas-<version>.pkg -t macos \
  --apiKey <KeyID> --apiIssuer <IssuerID>
xcrun altool --upload-app  -f dist/SuperMD-mas-<version>.pkg -t macos \
  --apiKey <KeyID> --apiIssuer <IssuerID>
```

Bump `version` in `Cargo.toml` and the lockfile, and commit, **before**
tagging — cargo-deb reads it and the DMG takes the tag, which would mask the
mistake.

The API key must be at `~/.appstoreconnect/private_keys/AuthKey_<KeyID>.p8`.
`altool` takes `--apiKey <KeyID>` and searches for that exact filename; it
does not accept a path, and a wrong name reports a missing key rather than
the real problem.

## Traps

Each of these cost real time, and none is obvious from Apple's docs.

**The developer name is set once, permanently.** It is the `Company Name`
field in the New App dialog, captured the first time *any* app is added to
the account. It cannot be edited afterwards and is not displayed on the App
Information page, so you cannot even verify it later. Never create a
throwaway app record to "try things out".

**`--validate-app` does not inspect extended attributes.** A provisioning
profile downloaded through a browser carries `com.apple.quarantine`, `cp`
preserves it into the bundle, validation passes clean, and processing then
rejects the build with ITMS-91109 *after* upload. `bundle_mas.sh` runs
`xattr -cr` before signing; the order matters, because changing extended
attributes after signing invalidates the signature.

**The App Store Connect version must equal `CFBundleShortVersionString`.** A
new app record defaults to `1.0`. A build declaring anything else uploads and
processes successfully but never attaches to the version, and TestFlight
files it under a separate group. Edit the version field to match.

**`CFBundleVersion` must strictly increase per upload** and is independent of
the marketing version. `bundle_mas.sh` derives it from the clock unless
`BUILD_NUMBER` is set.

**`LSApplicationCategoryType` is required** for the Mac App Store, and must
agree with the category chosen in App Store Connect. Validation is where its
absence surfaces.

**Give `codesign` the full identity string including the team id.** With
certificates from more than one team in the keychain, `--sign "Apple
Distribution"` fails as ambiguous.

**The provisioning profile type is "Mac App Store Connect."** There is no
plain "Mac App Store" entry; do not pick "App Store Connect" (that is iOS) or
"Developer ID" (that is the direct-download channel).

## Certificates

Two are needed, and they must not share a private key:

| Certificate | Signs |
| --- | --- |
| Apple Distribution | the `.app` |
| Mac Installer Distribution | the `.pkg` — appears as "3rd Party Mac Developer Installer" |

Generate a keypair and CSR for each, upload each CSR in the portal, download
the `.cer`, then **verify each certificate matches its key before trusting
it** — a mispaired upload yields a certificate whose private key you do not
hold, and that only fails later at `codesign`:

```sh
openssl x509 -inform DER -in distribution.cer -noout -pubkey | openssl md5
openssl pkey -in distribution.key -pubout | openssl md5      # must match
```

Bundle each key and certificate into a `.p12` — that is both what the
keychain imports and what CI wants as a secret — then import and delete the
loose key files.

Certificates last one year. Renewal repeats this section and needs a fresh
provisioning profile against the new distribution certificate.

## App Review

The plugin system is the most likely rejection, under guideline 2.4.5(iv),
which prohibits an app from being a storefront for other people's code. The
App Review notes should explain the design up front: plugins are WebAssembly
components sandboxed with no file or network access by default, all
first-party plugins ship inside the bundle, there is no in-app catalog, and
adding one requires either importing a file the user already downloaded or
confirming a `supermd://` link that names a single plugin.

If it is rejected on that ground anyway, the prepared fallback is to set
`install_ui::catalog_browsable()` and the `supermd://` handler to `false`
under `mas`. Import Plugin… survives, and third-party plugins keep working.

Answer **No** to the age rating's "unrestricted web access" question. SuperMD
has no browser and no web view; a plugin can reach only a site the user
approved by name, and the result is data rendered into a document. Answering
Yes forces a 17+ rating.

## Release automation

The `mas` job in `.github/workflows/release.yml` stays inert until the
signing secrets exist; tagging then builds the DMG and silently skips the
App Store package. The `.pkg` is deliberately excluded from GitHub Releases —
it goes to App Store Connect only.

Screenshots have their own pitfalls; see [screenshots.md](screenshots.md).

## Known limits

- **arm64 only.** Intel Macs cannot run the App Store build.
- **No GraphQL highlighting** under `mas`: grammar plugins load through
  tree-sitter's wasmtime 24, which JIT-compiles. Linking a grammar statically
  is blocked too — `tree-sitter-graphql` 0.2.1 emits grammar ABI 15 while
  tree-sitter 0.23 accepts 13–14. See `BACKLOG.md`.
- **Pulley is roughly 30x slower** than the compiled backend. The only
  user-visible cost measured was ~260 ms to format a 1 MB document.
