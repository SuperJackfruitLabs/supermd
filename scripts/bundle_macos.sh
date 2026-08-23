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

rm -rf "$DIST"
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
    <key>NSHighResolutionCapable</key><true/>
    <key>NSSupportsAutomaticGraphicsSwitching</key><true/>
</dict>
</plist>
PLIST

# Sign with a Developer ID identity when provided (hardened runtime,
# notarization-ready); otherwise ad-hoc (right-click → Open on first
# launch).
if [ -n "${SIGN_IDENTITY:-}" ]; then
    codesign --force --deep --options runtime --timestamp \
        --sign "$SIGN_IDENTITY" "$APP"
else
    codesign --force --deep --sign - "$APP"
fi

echo "creating dmg…"
hdiutil create -volname supermd -srcfolder "$APP" -ov -format UDZO \
    "$DIST/supermd-${VERSION}.dmg" > /dev/null

echo "done:"
ls -lh "$DIST"
