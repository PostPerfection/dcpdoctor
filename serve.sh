#!/usr/bin/env bash
# Serve the site locally exactly as .github/workflows/pages.yml assembles it,
# so /validate is never a stale copy of web/.
set -euo pipefail

cd "$(dirname "$0")"
out=$(mktemp -d)
trap 'rm -rf "$out"' EXIT

if [ ! -d web/pkg ]; then
    (cd rust/crates/dcpdoctor-wasm && wasm-pack build --target web --release --out-dir ../../../web/pkg)
fi

mkdir -p "$out/validate"
cp -r docs/* "$out/"
cp -r web/* "$out/validate/"

echo "http://localhost:8000"
python3 -m http.server 8000 --directory "$out"
