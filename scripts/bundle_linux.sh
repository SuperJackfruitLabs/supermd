#!/bin/bash
# Build supermd-linux-<arch>.tar.gz into dist/.
# Usage: scripts/bundle_linux.sh [version]
set -euo pipefail

VERSION="${1:-0.0.0-dev}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
ARCH="$(uname -m)"
DIST="$ROOT/dist"
STAGE="$DIST/supermd-linux"

cargo build --release --manifest-path "$ROOT/Cargo.toml"

rm -rf "$STAGE"
mkdir -p "$STAGE"
cp "$ROOT/target/release/supermd" "$STAGE/"
cp "$ROOT/assets/linux/supermd.desktop" "$STAGE/"
cp "$ROOT/assets/linux/supermd-128.png" "$STAGE/"
cp "$ROOT/assets/linux/supermd-512.png" "$STAGE/"
if [ -d "$ROOT/dist/default-plugins" ]; then
    cp -R "$ROOT/dist/default-plugins" "$STAGE/plugins"
fi

cat > "$STAGE/install.sh" <<'INSTALL'
#!/bin/sh
# Install SuperMD for the current user (no root needed).
set -e
HERE="$(cd "$(dirname "$0")" && pwd)"
BIN="$HOME/.local/bin"
APPS="$HOME/.local/share/applications"
ICONS="$HOME/.local/share/icons/hicolor"
mkdir -p "$BIN" "$APPS" "$ICONS/128x128/apps" "$ICONS/512x512/apps"
install -m 755 "$HERE/supermd" "$BIN/supermd"
install -m 644 "$HERE/supermd.desktop" "$APPS/supermd.desktop"
install -m 644 "$HERE/supermd-128.png" "$ICONS/128x128/apps/supermd.png"
install -m 644 "$HERE/supermd-512.png" "$ICONS/512x512/apps/supermd.png"
command -v update-desktop-database >/dev/null 2>&1 && \
    update-desktop-database "$APPS" || true
echo "SuperMD installed. Make sure $BIN is on your PATH."
INSTALL
chmod +x "$STAGE/install.sh"

tar -C "$DIST" -czf "$DIST/supermd-linux-${ARCH}.tar.gz" supermd-linux
rm -rf "$STAGE"
echo "done:"
ls -lh "$DIST"/supermd-linux-*.tar.gz
