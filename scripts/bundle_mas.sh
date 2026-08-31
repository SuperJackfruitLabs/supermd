#!/bin/bash
# Build the sandboxed App Store package into dist/.
# Usage: scripts/bundle_mas.sh <version>
# Requires: APP_IDENTITY       ("Apple Distribution: ...")
#           INSTALLER_IDENTITY ("3rd Party Mac Developer Installer: ...")
#           PROFILE            (path to the .provisionprofile)
set -euo pipefail

VERSION="${1:?version required}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DIST="$ROOT/dist"
APP="$DIST/SuperMD.app"

: "${APP_IDENTITY:?APP_IDENTITY required}"
: "${INSTALLER_IDENTITY:?INSTALLER_IDENTITY required}"
: "${PROFILE:?PROFILE required}"

echo "building sandboxed release binary…"
# --no-default-features drops `grammars`, and with it tree-sitter's
# wasmtime 24 — the second JIT. `mas` swaps the plugin host to Pulley.
cargo build --release --no-default-features --features mas \
    --manifest-path "$ROOT/Cargo.toml"

rm -rf "$APP" "$DIST"/SuperMD-mas*.pkg
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$ROOT/target/release/supermd" "$APP/Contents/MacOS/supermd"
cp "$ROOT/assets/icon.icns" "$APP/Contents/Resources/icon.icns"

# CFBundleVersion must strictly increase per upload; the App Store
# rejects a re-upload that reuses one. Callers pass a build number.
BUILD="${BUILD_NUMBER:-$(date +%Y%m%d%H%M)}"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>SuperMD</string>
    <key>CFBundleDisplayName</key><string>SuperMD</string>
    <key>CFBundleIdentifier</key><string>com.superjackfruit.supermd</string>
    <key>CFBundleVersion</key><string>${BUILD}</string>
    <key>CFBundleShortVersionString</key><string>${VERSION}</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleExecutable</key><string>supermd</string>
    <key>CFBundleIconFile</key><string>icon</string>
    <key>LSMinimumSystemVersion</key><string>12.0</string>
    <!-- Required by the Mac App Store. Must match the category chosen in
         App Store Connect. -->
    <key>LSApplicationCategoryType</key><string>public.app-category.developer-tools</string>
    <key>CFBundleDocumentTypes</key>
    <array>
      <dict>
        <key>CFBundleTypeName</key><string>Markdown Document</string>
        <key>CFBundleTypeRole</key><string>Editor</string>
        <key>LSHandlerRank</key><string>Owner</string>
        <key>LSItemContentTypes</key>
        <array><string>net.daringfireball.markdown</string></array>
        <key>CFBundleTypeExtensions</key>
        <array><string>md</string><string>markdown</string><string>mdown</string><string>mdx</string></array>
      </dict>
      <dict>
        <key>CFBundleTypeName</key><string>Text Document</string>
        <key>CFBundleTypeRole</key><string>Editor</string>
        <key>LSHandlerRank</key><string>Alternate</string>
        <key>LSItemContentTypes</key>
        <array><string>public.plain-text</string><string>public.source-code</string></array>
      </dict>
      <dict>
        <key>CFBundleTypeName</key><string>Folder</string>
        <key>CFBundleTypeRole</key><string>Viewer</string>
        <key>LSHandlerRank</key><string>Alternate</string>
        <key>LSItemContentTypes</key>
        <array><string>public.folder</string></array>
      </dict>
    </array>
    <key>CFBundleURLTypes</key>
    <array>
      <dict>
        <key>CFBundleURLName</key><string>com.superjackfruit.supermd.install</string>
        <key>CFBundleURLSchemes</key><array><string>supermd</string></array>
      </dict>
    </array>
    <key>ITSAppUsesNonExemptEncryption</key><false/>
    <key>NSHighResolutionCapable</key><true/>
    <key>NSSupportsAutomaticGraphicsSwitching</key><true/>
</dict>
</plist>
PLIST

# All 14 plugins ride inside the bundle — the MAS build has no catalog
# browser, so anything not shipped is unreachable until the user imports it.
mkdir -p "$APP/Contents/Resources/plugins"
cp -R "$ROOT/dist/plugins/." "$APP/Contents/Resources/plugins/"

cp "$PROFILE" "$APP/Contents/embedded.provisionprofile"

# A profile downloaded through a browser carries com.apple.quarantine, and
# cp preserves it. App Store processing rejects any quarantined file with
# ITMS-91109 — after a clean --validate-app, so it only surfaces once the
# build is already uploaded. Strip before signing: changing xattrs after
# would invalidate the signature.
xattr -cr "$APP"

codesign --force --options runtime --timestamp \
    --entitlements "$ROOT/assets/mas.entitlements" \
    --sign "$APP_IDENTITY" "$APP"

productbuild --component "$APP" /Applications \
    --sign "$INSTALLER_IDENTITY" \
    "$DIST/SuperMD-mas-${VERSION}.pkg"

echo "done: $DIST/SuperMD-mas-${VERSION}.pkg"
