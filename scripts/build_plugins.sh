#!/bin/bash
# Build first-party plugins (default) or test fixtures (--fixtures)
# to wasm components. Requires: rustup target add wasm32-wasip2
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TARGET=wasm32-wasip2

if [ "${1:-}" = "--fixtures" ]; then
    OUT="$ROOT/tests/fixtures/plugins"
    CRATES="echo panic hang reader fetcher"
    BASE="$ROOT/plugins/fixtures"
else
    OUT="$ROOT/dist/plugins"
    CRATES="dot toc emoji tidy todo-marks url-title html-export"
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

# Grammar plugins ship committed artifacts — plain copy, no build.
if [ "${1:-}" != "--fixtures" ]; then
    for name in graphql; do
        mkdir -p "$OUT/$name"
        cp "$ROOT/plugins/$name/plugin.toml" "$ROOT/plugins/$name/grammar.wasm" \
           "$ROOT/plugins/$name/highlights.scm" "$OUT/$name/"
    done
fi

# nofetch: the fetcher binary with a capability-free manifest — proves
# net enforcement keys off the declaration, not the wasm.
if [ "${1:-}" = "--fixtures" ]; then
    mkdir -p "$OUT/nofetch"
    cp "$OUT/fetcher/plugin.wasm" "$OUT/nofetch/plugin.wasm"
    cat > "$OUT/nofetch/plugin.toml" <<'EOF'
name = "nofetch"
version = "0.1.0"
formats = true
EOF
fi
echo "built: $CRATES -> $OUT"
