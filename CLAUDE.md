# nrzMonitor

Überwachung wie Netdata/Grafana in Rust: `shared/` (Typen, Katalog), `server/` (Sammler, DuckDB, API),
`web/` (Dioxus 0.7 WASM, nrzUI). **Vor jeder Änderung `ARCHITECTURE.md` lesen** – dort stehen Besitz der
Dateien, Schnittstellen und Regeln.

- Oberfläche und Kommentare deutsch, Schweizer Schreibweise (ss statt ß). Commit-Nachrichten deutsch ohne Umlaute.
- nrz-Design: nur Theme-Klassen, keine festen Farben; Reihenfarben nur über `chart::color(slot)`.
- `dx` = dioxus-cli unter `~/.cargo/bin/dx` (das `dx` im PATH ist Deno).
- DuckDB wird **nicht** aus dem Quelltext gebaut: kein `bundled`-, `parquet`- oder `json`-Feature setzen.
- Zähler als Rate speichern, Werte vor dem Abgeben runden (`util::round`) – siehe ARCHITECTURE.md, Abschnitt 3.
- Entwickeln: `scripts/dev.sh` (Mac, Beispieldaten), `scripts/run-linux.sh` (echte Sammler im Container),
  `scripts/shot.sh` (Bildschirmfoto). Tests: `cargo test -p nrzmon-server -p nrzmon-shared`.
- Clippy nur für berührte Crates: `cargo clippy -p <crate> --no-deps`.
- Release: Version in `Cargo.toml`, `web/Cargo.toml`, `umbrel/umbrel-app.yml`, `RELEASE=` in `umbrel/hooks/pre-start`
  und `releaseNotes` in `nrzStore.json` anheben, **lokal committen, nicht pushen**, dann
  `scripts/umbrel-release.sh <version> "Notizen"` – das Skript pusht erst den Tag, dann `main`. Wird `main`
  vorher gepusht, zeigt der Store kurz die neue Version mit altem Download-Tag (umbrelOS installiert dann das alte Programm).
