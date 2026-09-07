#!/bin/bash
# Fail if a binary references private Apple APIs.
#
# App Review rejects these on sight, and its scan is static — a linked
# symbol is enough, whether or not the code can run. altool
# --validate-app does NOT check, so without this the failure only appears
# after upload. See vendor/gpui/PATCH.md.
#
# Usage: scripts/check_private_apis.sh <binary>
set -uo pipefail
BIN="${1:?usage: check_private_apis.sh <binary>}"
fail=0

if [ ! -e "$BIN" ]; then
    echo "error: $BIN does not exist" >&2
    exit 1
fi

# nm must actually be able to parse this as an object/executable, or the
# rest of this script is scanning nothing and a bad path would silently
# report "clean". Capture stdout+stderr together so a parse failure is
# shown to the caller instead of being discarded.
nm_out=$(nm -u "$BIN" 2>&1)
nm_status=$?
if [ "$nm_status" -ne 0 ]; then
    echo "error: nm could not read $BIN as an object/executable:" >&2
    echo "$nm_out" | sed 's/^/  /' >&2
    exit 1
fi

# Private CoreGraphics / SkyLight, as undefined symbols.
syms=$(echo "$nm_out" | sed 's/^ *//' | grep -E '^_(CGS|SLS)' | sort -u)
if [ -n "$syms" ]; then
    echo "private symbols:"; echo "$syms" | sed 's/^/  /'; fail=1
fi

# Private selectors are dispatched at runtime and never appear as
# undefined symbols — only a string scan finds them.
sels=$(strings -a "$BIN" 2>/dev/null | grep -E '^_(windowResize[A-Za-z]*Cursor|updateProxyLayer|setCornerMask)$' | sort -u)
if [ -n "$sels" ]; then
    echo "private selectors:"; echo "$sels" | sed 's/^/  /'; fail=1
fi

if [ "$fail" = 0 ]; then
    echo "clean: no private Apple APIs in $BIN"
fi
exit $fail
