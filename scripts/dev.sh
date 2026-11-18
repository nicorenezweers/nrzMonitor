#!/usr/bin/env bash
# Entwicklung: Oberflaeche bauen (Debug) und den Server starten, der sie ausliefert.
# Auf dem Mac laufen Beispieldaten (NRZMON_DEMO), echte Sammler brauchen Linux: scripts/run-linux.sh.
#
#   scripts/dev.sh                 # http://127.0.0.1:8571
#   NRZMON_PORT=8601 scripts/dev.sh
#   scripts/dev.sh --no-web        # Oberflaeche nicht neu bauen
#
# Nach Aenderungen an der Oberflaeche: `(cd web && dx build --platform web)` und Seite neu laden –
# der Server muss dafuer nicht neu starten.
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"
cd "${ROOT}"
PORT="${NRZMON_PORT:-8571}"
if [ "${1:-}" != "--no-web" ]; then
  DX="$(find_dx)"
  (cd web && "${DX}" build --platform web)
fi
export NRZMON_BIND="${NRZMON_BIND:-127.0.0.1:${PORT}}"
export NRZMON_DATA_DIR="${NRZMON_DATA_DIR:-/tmp/nrzmon-dev-${PORT}}"
export NRZMON_WEB_DIR="${ROOT}/web/target/dx/nrzmon-web/debug/web/public"
exec cargo run -p nrzmon-server
