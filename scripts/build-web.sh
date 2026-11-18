#!/usr/bin/env bash
# Oberflaeche fuer die Auslieferung bauen: Release-Bundle nach web/dist, jede Datei zusaetzlich als .gz.
# Der Server bettet web/dist beim Bauen ein (rust-embed) – also zuerst dieses Skript, dann cargo build.
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"
DX="$(find_dx)"
cd "${ROOT}/web"
"${DX}" bundle --platform web --release
OUT="target/dx/nrzmon-web/release/web/public"
[ -f "${OUT}/index.html" ] || { echo "Bundle fehlt: ${OUT}" >&2; exit 1; }
find dist -mindepth 1 ! -name .gitkeep -delete
cp -R "${OUT}/." dist/
# Vorkomprimieren: der Server liefert .gz aus, wenn der Browser es annimmt.
find dist -type f ! -name '*.gz' ! -name .gitkeep ! -name '*.png' ! -name '*.woff2' -size +1k -exec gzip -9 -k -f {} \;
du -sh dist | sed 's/^/web\/dist: /'
