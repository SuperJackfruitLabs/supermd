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
    <!-- Purpose strings. SuperMD is a file editor: the user picks a folder
         and the sidebar lists what is inside it. Opening the Home folder
         therefore reaches macOS's protected locations, each of which prompts.
         App Review rejected 0.0.14 (202609012108) under 5.1.1(ii) because
         these prompts appeared with no explanation. Each string says what is
         accessed and gives an example, as the guideline requires. -->
    <key>NSDocumentsFolderUsageDescription</key>
    <string>SuperMD needs access to your Documents folder to open and save Markdown files stored there — for example, if you open a notes folder inside Documents, it reads those files to show them and writes your edits back.</string>
    <key>NSDesktopFolderUsageDescription</key>
    <string>SuperMD needs access to your Desktop to open and save Markdown files stored there — for example, if you open a project folder on your Desktop, it reads those files to show them and writes your edits back.</string>
    <key>NSDownloadsFolderUsageDescription</key>
    <string>SuperMD needs access to your Downloads folder to open Markdown files stored there — for example, a set of notes you downloaded, or a plugin archive you chose to import.</string>
    <key>NSAppleMusicUsageDescription</key>
    <string>SuperMD does not play or use music. It asks only because your Music folder appears when you open your Home folder in the file sidebar, and it must list that folder's contents to show it to you.</string>
    <key>NSRemovableVolumesUsageDescription</key>
    <string>SuperMD needs access to removable volumes to open and save Markdown files stored on them — for example, a folder of notes on a USB drive that you choose to open.</string>
    <key>NSNetworkVolumesUsageDescription</key>
    <string>SuperMD needs access to network volumes to open and save Markdown files stored on them — for example, a shared folder of documents on a file server that you choose to open.</string>
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
