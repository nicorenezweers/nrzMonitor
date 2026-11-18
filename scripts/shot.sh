#!/usr/bin/env bash
# Bildschirmfoto der Oberflaeche mit Headless-Chrome im Docker (kein Browser auf dem Rechner noetig).
#
#   scripts/shot.sh /system/cpu out.png [breite] [hoehe] [port]
#
# Der Server muss laufen (scripts/dev.sh). Standard: 1600x1100, Port 8571.
set -euo pipefail
ROUTE="${1:-/}"
OUT="${2:-shot.png}"
W="${3:-1600}"
H="${4:-1100}"
PORT="${5:-${NRZMON_PORT:-8571}}"
NAME="nrzmon-shot-$$"
docker run --name "${NAME}" --add-host=host.docker.internal:host-gateway zenika/alpine-chrome \
  --no-sandbox --headless --disable-gpu --hide-scrollbars \
  --window-size="${W},${H}" --virtual-time-budget=9000 \
  --screenshot=/tmp/shot.png "http://host.docker.internal:${PORT}${ROUTE}" >/dev/null 2>&1 || true
docker cp "${NAME}:/tmp/shot.png" "${OUT}" >/dev/null
docker rm -f "${NAME}" >/dev/null
echo "${OUT}"
