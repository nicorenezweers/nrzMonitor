#!/usr/bin/env bash
# Baut nrzMonitor fuer umbrelOS (Linux amd64 + arm64), legt die fertigen Dateien auf den Zweig
# `umbrel` und pusht den Tag `umbrel-<version>`. nrzGit traegt die Version dann per nrzStore.json
# selbst im nrzStore ein (RELEASE in hooks/pre-start, version, releaseNotes, Store-Version).
#
#   ./scripts/umbrel-release.sh 1.0.1 "Kurze Beschreibung"
#
# Voraussetzungen: dx (dioxus-cli 0.7), zig + cargo-zigbuild, Rust-Targets x86_64/aarch64-unknown-linux-gnu.
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"
VERSION="${1:?Version fehlt, z.B. 1.0.1}"
NOTES="${2:-nrzMonitor ${VERSION}}"
TAG="umbrel-${VERSION}"
cd "${ROOT}"

[ -z "$(git status --porcelain)" ] || { echo "Arbeitsverzeichnis nicht sauber - erst committen." >&2; exit 1; }
# Version in umbrel/ muss zum Release passen (umbrel-app.yml und RELEASE in hooks/pre-start).
grep -q "^version: \"${VERSION}\"" umbrel/umbrel-app.yml || { echo "umbrel/umbrel-app.yml hat nicht version: \"${VERSION}\"" >&2; exit 1; }
grep -q "^RELEASE=\"${TAG}\"" umbrel/hooks/pre-start || { echo "umbrel/hooks/pre-start hat nicht RELEASE=\"${TAG}\"" >&2; exit 1; }
! git rev-parse -q --verify "refs/tags/${TAG}" >/dev/null || { echo "Tag ${TAG} gibt es schon." >&2; exit 1; }

# Oberflaeche einmal bauen; sie wird in beide Programme eingebettet.
scripts/build-web.sh

OUT="$(mktemp -d)"
trap 'rm -rf "${OUT}"' EXIT
for ARCH in amd64 arm64; do
  TRIPLE="$(triple_for "${ARCH}")"
  echo "==> Baue ${ARCH} (${TRIPLE})"
  cargo zigbuild --release -p nrzmon-server --target "${TRIPLE}.2.31"
  BIN="target/${TRIPLE}/release/nrzmonitor"
  LIB="$(find target -path "*duckdb-download/${TRIPLE}/*" -name 'libduckdb.so' | head -1)"
  [ -f "${LIB}" ] || { echo "libduckdb.so fuer ${TRIPLE} fehlt" >&2; exit 1; }
  # Oberflaeche muss eingebaut sein (sonst zeigt der Server nur die Hinweisseite).
  strings "${BIN}" | grep -c -F "nrzmon-web_bg" >/dev/null || { echo "Oberflaeche nicht eingebettet (${ARCH})" >&2; exit 1; }
  gzip -9 -c "${BIN}" > "${OUT}/nrzmonitor-${ARCH}.gz"
  gzip -9 -c "${LIB}" > "${OUT}/libduckdb-${ARCH}.so.gz"
done
(cd "${OUT}" && shasum -a 256 nrzmonitor-*.gz libduckdb-*.gz > SHA256SUMS)
ls -la "${OUT}"

WT="$(mktemp -d)"
git fetch origin umbrel 2>/dev/null || true
if git rev-parse -q --verify origin/umbrel >/dev/null; then
  git worktree add -q "${WT}" -B umbrel origin/umbrel
else
  git worktree add -q --detach "${WT}"
  (cd "${WT}" && git checkout -q --orphan umbrel && git rm -rqf . 2>/dev/null || true)
fi
cp "${OUT}"/* "${WT}/"
cat > "${WT}/README.md" <<'MD'
# nrzMonitor - Umbrel-Paket

Nur fertige Dateien fuer die App `nrz-monitor` im nrzStore (erstellt mit `scripts/umbrel-release.sh`).
`hooks/pre-start` im Store laedt sie per Tag `umbrel-<version>`.

| Datei | Inhalt |
|-------|--------|
| `nrzmonitor-amd64.gz`, `nrzmonitor-arm64.gz` | Server mit eingebauter Oberflaeche (Linux, glibc 2.31) |
| `libduckdb-amd64.so.gz`, `libduckdb-arm64.so.gz` | DuckDB 2.0 (liegt neben dem Programm) |
| `SHA256SUMS` | Pruefsummen |
MD
(
  cd "${WT}"
  git add -A
  git commit -qm "nrzMonitor ${VERSION}: Umbrel-Paket"
  git tag -a "${TAG}" -m "${NOTES}"
  git push -q origin umbrel
  git push -q origin "${TAG}"
)
git worktree remove --force "${WT}"
# Erst jetzt main pushen: kaeme umbrel/ (neue Version) vor dem Tag in den Store, haelt nrzGit den
# Download-Tag bis zum Tag auf dem alten Stand - umbrelOS koennte genau diesen Zwischenstand holen
# und die neue Version mit dem alten Programm installieren.
git push -q origin HEAD:main
echo "==> ${TAG} veroeffentlicht und main gepusht. nrzGit aktualisiert den nrzStore."
