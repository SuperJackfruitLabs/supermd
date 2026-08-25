#!/bin/bash
# Build supermd.app and supermd.dmg into dist/.
# Usage: scripts/bundle_macos.sh [version]   (default: 0.0.0-dev)
set -euo pipefail

VERSION="${1:-0.0.0-dev}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DIST="$ROOT/dist"
APP="$DIST/supermd.app"

echo "building release binary…"
cargo build --release --manifest-path "$ROOT/Cargo.toml"

# Clear only our own outputs — dist/ may hold the staged default
# plugins from build_plugins.sh, which ride inside the bundle.
rm -rf "$APP" "$DIST"/supermd-*.dmg "$DIST/dmg-staging"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"

cp "$ROOT/target/release/supermd" "$APP/Contents/MacOS/supermd"
cp "$ROOT/assets/icon.icns" "$APP/Contents/Resources/icon.icns"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>SuperMD</string>
    <key>CFBundleDisplayName</key><string>SuperMD</string>
    <key>CFBundleIdentifier</key><string>com.superjackfruitlabs.supermd</string>
    <key>CFBundleVersion</key><string>${VERSION}</string>
    <key>CFBundleShortVersionString</key><string>${VERSION}</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleExecutable</key><string>supermd</string>
    <key>CFBundleIconFile</key><string>icon</string>
    <key>LSMinimumSystemVersion</key><string>12.0</string>
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
    <key>NSHighResolutionCapable</key><true/>
    <key>NSSupportsAutomaticGraphicsSwitching</key><true/>
</dict>
</plist>
PLIST

# Sign with a Developer ID identity when provided (hardened runtime,
# notarization-ready); otherwise ad-hoc (right-click → Open on first
# launch).
# Default plugins ride inside the bundle (seeded on first run); they
# must be in place before signing so the signature covers them.
if [ -d "$ROOT/dist/default-plugins" ]; then
    mkdir -p "$APP/Contents/Resources/plugins"
    cp -R "$ROOT/dist/default-plugins/." "$APP/Contents/Resources/plugins/"
fi

# Entitlements: the wasm plugin runtime needs JIT under the hardened
# runtime (see assets/entitlements.plist) — without them the notarized
# app is SIGKILLed on the first plugin execution.
if [ -n "${SIGN_IDENTITY:-}" ]; then
    codesign --force --deep --options runtime --timestamp \
        --entitlements "$ROOT/assets/entitlements.plist" \
        --sign "$SIGN_IDENTITY" "$APP"
else
    codesign --force --deep --entitlements "$ROOT/assets/entitlements.plist" --sign - "$APP"
fi

echo "creating dmg…"
DMG="$DIST/supermd-${VERSION}.dmg"
# Dressed DMG: display-named app, /Applications symlink, background,
# and the committed Finder layout (.DS_Store).
STAGING="$DIST/dmg-staging"
rm -rf "$STAGING"
mkdir -p "$STAGING/.background"
ditto "$APP" "$STAGING/SuperMD.app"   # ditto preserves signatures
ln -s /Applications "$STAGING/Applications"
cp "$ROOT/assets/dmg/bg@2x.png" "$STAGING/.background/bg@2x.png"
if [ -f "$ROOT/assets/dmg/DS_Store" ]; then
    cp "$ROOT/assets/dmg/DS_Store" "$STAGING/.DS_Store"
fi
hdiutil create -volname SuperMD -srcfolder "$STAGING" -ov -format UDZO \
    "$DMG" > /dev/null
rm -rf "$STAGING"

# The DMG needs its own signature too — spctl assesses the image's
# primary signature, not just the app inside it.
if [ -n "${SIGN_IDENTITY:-}" ]; then
    codesign --force --timestamp --sign "$SIGN_IDENTITY" "$DMG"
fi

echo "done:"
ls -lh "$DIST"
