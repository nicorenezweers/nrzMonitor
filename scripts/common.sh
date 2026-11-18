# Gemeinsame Helfer der Skripte (wird mit `source` eingebunden).

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# dioxus-cli finden (`dx` im PATH kann auch Denos dx sein).
find_dx() {
  local c
  for c in "${DX:-}" "${HOME}/.cargo/bin/dx" "$(command -v dx || true)"; do
    if [ -n "${c}" ] && "${c}" --version 2>/dev/null | grep -qi dioxus; then
      echo "${c}"
      return 0
    fi
  done
  echo "dioxus-cli (dx) nicht gefunden – cargo binstall dioxus-cli@0.7.10" >&2
  return 1
}

# Rust-Ziel fuer eine Architektur (amd64 | arm64).
triple_for() {
  case "$1" in
    amd64 | x86_64) echo "x86_64-unknown-linux-gnu" ;;
    arm64 | aarch64) echo "aarch64-unknown-linux-gnu" ;;
    *) echo "unbekannte Architektur $1" >&2; return 1 ;;
  esac
}
