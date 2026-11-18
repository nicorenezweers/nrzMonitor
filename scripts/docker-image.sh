#!/usr/bin/env bash
# Baut das Multi-Arch-Image (amd64 + arm64) und pusht es in die Registry von nrzGit.
#   scripts/docker-image.sh 1.0.0
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"
VERSION="${1:?Version fehlt}"
IMAGE="${NRZMON_IMAGE:-100.78.107.162:8390/nicorenezweers/nrz-monitor}"
cd "${ROOT}"
scripts/build-web.sh
rm -rf dist && mkdir -p dist
for ARCH in amd64 arm64; do
  TRIPLE="$(triple_for "${ARCH}")"
  cargo zigbuild --release -p nrzmon-server --target "${TRIPLE}.2.31"
  mkdir -p "dist/${ARCH}"
  cp "target/${TRIPLE}/release/nrzmonitor" "dist/${ARCH}/"
  cp "$(find target -path "*duckdb-download/${TRIPLE}/*" -name 'libduckdb.so' | head -1)" "dist/${ARCH}/"
done
docker buildx build --platform linux/amd64,linux/arm64 -t "${IMAGE}:${VERSION}" --push .
echo "==> ${IMAGE}:${VERSION}"
