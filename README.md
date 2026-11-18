# nrzMonitor - Umbrel-Paket

Nur fertige Dateien fuer die App `nrz-monitor` im nrzStore (erstellt mit `scripts/umbrel-release.sh`).
`hooks/pre-start` im Store laedt sie per Tag `umbrel-<version>`.

| Datei | Inhalt |
|-------|--------|
| `nrzmonitor-amd64.gz`, `nrzmonitor-arm64.gz` | Server mit eingebauter Oberflaeche (Linux, glibc 2.31) |
| `libduckdb-amd64.so.gz`, `libduckdb-arm64.so.gz` | DuckDB 2.0 (liegt neben dem Programm) |
| `SHA256SUMS` | Pruefsummen |
