#!/usr/bin/env bash
# Echte Sammler testen: Server fuer Linux bauen (zig) und in einem Container im Host-Modus starten –
# genau wie spaeter auf dem Geraet. Auf dem Mac ist der "Host" die Docker-VM (colima): ein echtes
# Linux mit /proc, /sys, cgroups und laufenden Containern.
#
#   scripts/run-linux.sh                # Port 8571, im Vordergrund (Strg+C beendet)
#   NRZMON_PORT=8602 scripts/run-linux.sh
#   scripts/run-linux.sh --demo         # Beispieldaten statt echter Sammler
#
# Voraussetzungen: zig, cargo-zigbuild, Docker. Die Oberflaeche kommt aus web/dist (scripts/build-web.sh)
# oder – wenn vorhanden – aus dem Debug-Build (web/target/dx/.../public).
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"
cd "${ROOT}"
PORT="${NRZMON_PORT:-8571}"
ARCH="$(docker info --format '{{.Architecture}}' 2>/dev/null || uname -m)"
TRIPLE="$(triple_for "${ARCH}")"
cargo zigbuild -p nrzmon-server --target "${TRIPLE}.2.31"
BIN="target/${TRIPLE}/debug/nrzmonitor"
LIB="$(find target -path "*duckdb-download/${TRIPLE}/*" -name 'libduckdb.so' | head -1)"
[ -f "${LIB}" ] || { echo "libduckdb.so fuer ${TRIPLE} nicht gefunden" >&2; exit 1; }
STAGE="target/run-linux-${PORT}"
rm -rf "${STAGE}" && mkdir -p "${STAGE}/app" "${STAGE}/data"
cp "${BIN}" "${LIB}" "${STAGE}/app/"
WEB="web/target/dx/nrzmon-web/debug/web/public"
[ -f "${WEB}/index.html" ] || WEB="web/dist"
cp -R "${WEB}" "${STAGE}/app/web"
DEMO=0
[ "${1:-}" = "--demo" ] && DEMO=1
NAME="nrzmon-dev-${PORT}"
docker rm -f "${NAME}" >/dev/null 2>&1 || true
echo "==> http://127.0.0.1:${PORT}  (Container ${NAME}, Host-Netzwerk der Docker-VM)"
exec docker run --rm --name "${NAME}" --init \
  --network host --pid host \
  -v "${ROOT}/${STAGE}/app:/app:ro" \
  -v /proc:/host/proc:ro -v /sys:/host/sys:ro -v /:/host/root:ro \
  -v /var/run/docker.sock:/var/run/docker.sock:ro \
  -e NRZMON_BIND="0.0.0.0:${PORT}" -e NRZMON_DATA_DIR=/data -e NRZMON_WEB_DIR=/app/web \
  -e NRZMON_DEMO="${DEMO}" -e RUST_LOG="${RUST_LOG:-info}" \
  --tmpfs /data:size=512m \
  debian:trixie-slim /app/nrzmonitor
