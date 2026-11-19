# nrzMonitor

Überwachung wie Netdata/Grafana in Rust: `shared/` (Typen, Katalog), `server/` (Sammler, DuckDB, API),
`web/` (Dioxus 0.7 WASM, nrzUI). **Vor jeder Änderung `ARCHITECTURE.md` lesen** – dort stehen Besitz der
Dateien, Schnittstellen und Regeln.

- Oberfläche und Kommentare deutsch, Schweizer Schreibweise (ss statt ß). Commit-Nachrichten deutsch ohne Umlaute.
- nrz-Design: nur Theme-Klassen, keine festen Farben; Reihenfarben nur über `chart::color(slot)`.
- 100 % Rust: keine CSS-, JS- oder Shell-Dateien und kein Dockerfile (einzige Ausnahme: `umbrel/hooks/pre-start`,
  umbrelOS verlangt ein Shell-Skript). CSS erzeugt `web/style` beim Bauen aus den Klassen (Tailwind-v4-Schreibweise);
  fehlt eine Klasse, schlägt `(cd web && cargo test -p nrzmon-style)` fehl – dann dort ergänzen. Hilfsbefehle: `cargo xtask`.
- `dx` = dioxus-cli unter `~/.cargo/bin/dx` (das `dx` im PATH ist Deno).
- DuckDB wird **nicht** aus dem Quelltext gebaut: kein `bundled`-, `parquet`- oder `json`-Feature setzen.
- Zähler als Rate speichern, Werte vor dem Abgeben runden (`util::round`) – siehe ARCHITECTURE.md, Abschnitt 3.
- Entwickeln: `cargo xtask dev` (Mac, Beispieldaten), `cargo xtask run-linux` (echte Sammler im Container),
  `cargo xtask shot` (Bildschirmfoto); `cargo xtask` zeigt alle Befehle. Tests: `cargo test -p nrzmon-server -p nrzmon-shared`.
- Clippy nur für berührte Crates: `cargo clippy -p <crate> --no-deps`.
- Release: Version in `Cargo.toml`, `web/Cargo.toml`, `umbrel/umbrel-app.yml`, `RELEASE=` in `umbrel/hooks/pre-start`
  und `releaseNotes` in `nrzStore.json` anheben, **lokal committen, nicht pushen**, dann
  `cargo xtask umbrel-release <version> "Notizen"` – der Befehl pusht erst den Tag, dann `main`. Wird `main`
  vorher gepusht, zeigt der Store kurz die neue Version mit altem Download-Tag (umbrelOS installiert dann das alte Programm).
