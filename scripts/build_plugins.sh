#!/bin/bash
# Build first-party plugins (default) or test fixtures (--fixtures)
# to wasm components. Requires: rustup target add wasm32-wasip2
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TARGET=wasm32-wasip2

if [ "${1:-}" = "--fixtures" ]; then
    OUT="$ROOT/tests/fixtures/plugins"
    CRATES="echo panic hang"
    BASE="$ROOT/plugins/fixtures"
else
    OUT="$ROOT/dist/plugins"
    CRATES="dot toc"
    BASE="$ROOT/plugins"
fi

rustup target add $TARGET 2>/dev/null || true
for name in $CRATES; do
    echo "building ${name}..."
    (cd "$BASE/$name" && cargo build --release --target $TARGET)
    mkdir -p "$OUT/$name"
    cp "$BASE/$name"/target/$TARGET/release/*.wasm "$OUT/$name/plugin.wasm"
    cp "$BASE/$name/plugin.toml" "$OUT/$name/plugin.toml"
done
echo "built: $CRATES -> $OUT"
