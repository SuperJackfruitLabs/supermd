#!/bin/bash
# Post-release: fill plugins/catalog.json sha256 fields from the
# published per-plugin zips of a release, then review + commit.
#   scripts/update_catalog_hashes.sh v0.0.9
set -euo pipefail
TAG="$1"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CATALOG="$ROOT/plugins/catalog.json"
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

python3 - "$CATALOG" "$TAG" "$TMP" <<'PY'
import json, subprocess, sys, hashlib, urllib.request
catalog_path, tag, tmp = sys.argv[1:4]
data = json.load(open(catalog_path))
for entry in data["plugins"]:
    url = f"https://github.com/SuperJackfruitLabs/supermd/releases/download/{tag}/plugin-{entry['name']}.zip"
    dest = f"{tmp}/{entry['name']}.zip"
    urllib.request.urlretrieve(url, dest)
    entry["sha256"] = hashlib.sha256(open(dest, "rb").read()).hexdigest()
    entry["download"] = url
json.dump(data, open(catalog_path, "w"), indent=2)
open(catalog_path, "a").write("\n")
print("catalog updated for", tag)
PY
echo "review with: git diff plugins/catalog.json"
