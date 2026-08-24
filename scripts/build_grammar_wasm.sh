#!/bin/bash
# Build a tree-sitter grammar to wasm for SuperMD grammar plugins.
# One-time developer tool — the built artifact is committed; users and
# CI never need this. Requires: tree-sitter-cli 0.23.x (ABI 14) and
# emscripten (emcc) on PATH.
#   scripts/build_grammar_wasm.sh <grammar-src-dir> <out-dir>
# Regenerates parser.c at the CLI's ABI, then builds the wasm module.
set -euo pipefail
SRC="$1"; OUT="$2"
(cd "$SRC" && tree-sitter generate && tree-sitter build --wasm -o "$OUT/grammar.wasm")
echo "built: $OUT/grammar.wasm"
