# nrzMonitor – Architektur

Überwachung wie Netdata/Grafana, komplett in Rust: liest jede Sekunde den Host, jeden Docker-Container
und das Netzwerk aus (dazu DNS über nrzNetwork, Cloudflare, NetBird), speichert sparsam in DuckDB und
zeigt alles live in einer Oberfläche im nrz-Design. Läuft als Docker-Container im Host-Modus.

Dieses Dokument ist die **verbindliche Absprache** zwischen den Bereichen. Wer etwas baut, liest es
ganz und hält sich an Besitz, Schnittstellen und Regeln – dann passen die Teile ohne Rückfrage zusammen.

## 1. Aufbau

```
shared/     nrzmon-shared   Typen für Server UND Oberfläche: API, Metrik-Modell, Katalog (nur serde)
server/     nrzmon-server   Programm `nrzmonitor`: Sammler, DuckDB-Speicher, HTTP-API, liefert die Oberfläche aus
web/        nrzmon-web      Oberfläche: Dioxus 0.7 (WASM, nur Client), nrzUI-Bausteine, eigener Cargo-Workspace
web/style/  nrzmon-style    Stylesheet: erzeugt beim Bauen das CSS aus den Klassen im Quelltext (ersetzt Tailwind)
xtask/      Befehle (`cargo xtask`): build-web, dev, run-linux, shot, docker-image, umbrel-release
umbrel/     Paket für den nrzStore (docker-compose im Host-Modus, Manifest, hooks/pre-start)
```

Datenfluss:

```
Sammler ──Frame──▶ Core.submit ──▶ Ring (RAM, volle Auflösung, 15 min)  ──▶ Live-Kanal (WebSocket) ──▶ Browser
                              └──▶ Store.ingest (DuckDB, gestuft, dauerhaft)
Sammler ──▶ Core.snapshot(key, daten)   (Listen/Zustände: Container, Prozesse, Peers …)
Sammler ──▶ Core.event(ereignis)        (Container gestartet, Alarm, Peer offline …)
Browser ──▶ POST /api/v1/query          (Zeitreihen: RAM oder DuckDB, je nach Zeitraum)
```

## 2. Bereiche und Besitz

Jeder Bereich besitzt **seine** Dateien und ändert nur diese. Alles andere ist tabu – auch
„nur kurz formatieren“. Braucht ein Bereich etwas vom Kern, das fehlt: in der eigenen Datei lösen
und im Abschlussbericht nennen.

| Bereich | Server | Shared | Web |
|---|---|---|---|
| **Kern** (steht, nicht ändern) | `main.rs`, `core.rs`, `config.rs`, `registry.rs`, `frame.rs`, `ring.rs`, `live.rs`, `snaps.rs`, `settings.rs`, `auth.rs`, `web.rs`, `db.rs`, `util.rs`, `api/mod.rs`, `api/base.rs`, `collectors/mod.rs`, `collectors/monitor.rs` | `lib.rs`, `model.rs`, `query.rs`, `events.rs`, `api.rs`, `catalog/mod.rs`, `catalog/monitor.rs` | `main.rs`, `api.rs`, `live.rs`, `state.rs`, `fmt.rs`, `components/ui/**`, `components/hooks/**`, `components/lazy.rs`, `pages/mod.rs`, `pages/login.rs` |
| **1 Speicher & Analyse** | `store/**`, `api/query.rs`, `api/storage.rs` | `storage.rs` | `pages/explore.rs`, `pages/events.rs`, `pages/storage.rs` |
| **2 System** | `collectors/system/**`, `api/system.rs` | `system.rs`, `catalog/system.rs` | `pages/system/**` |
| **3 Docker & Alarme** | `collectors/docker/**`, `alerts/**`, `api/docker.rs`, `api/alerts.rs` | `docker.rs`, `alerts.rs`, `catalog/docker.rs` | `pages/docker/**`, `pages/alerts.rs` |
| **4 Netzwerk-Integrationen** | `collectors/{dns,cloudflare,netbird,probes}/**`, `api/dns.rs`, `api/integrations.rs` | `dns.rs`, `cloudflare.rs`, `netbird.rs`, `probes.rs`, `catalog/{dns,cloudflare,netbird,probes}.rs` | `pages/dns/**`, `pages/cloudflare.rs`, `pages/netbird.rs`, `pages/probes.rs`, `pages/integrations.rs` |
| **MCP** | `mcp/**`, `api/mcp.rs` | `mcp.rs` | `pages/mcp.rs` |
| **Strom** | `collectors/power.rs`, `api/power.rs` | `power.rs`, `catalog/power.rs` | `pages/power.rs` |
| **5 Oberfläche** | – | – | `components/chart/**`, `components/widgets.rs`, `components/layout.rs`, `pages/overview.rs`, `pages/metrics.rs`, `pages/settings.rs`, `style/**` (CSS-Erzeuger, nrz-Thema), `route.rs` (Aufbau) |

Gemeinsam genutzte Dateien mit **markierten Abschnitten** (nur im eigenen Abschnitt ergänzen, nichts umsortieren):

- `server/Cargo.toml`, `web/Cargo.toml`: neue Abhängigkeiten nur unter dem eigenen `# --- Abschnitt …`.
  Sparsam: lieber `std`, `tokio`, `reqwest`, `serde_json`, `libc` nutzen, die schon da sind.
- `web/src/route.rs`: neue Routen nur im eigenen `// --- …`-Block; die `use`-Zeile oben ergänzen.
- `Cargo.lock`-Konflikte löst der Zusammenbau, nicht der Bereich.

Bereich 5 darf die **öffentlichen Namen und Props** der Diagramm-Bausteine (Abschnitt 7) erweitern,
aber nicht umbenennen oder entfernen – die anderen Bereiche bauen gleichzeitig darauf.

## 3. Kern des Servers

Alles hängt am `Core` (billig klonbar, `server/src/core.rs`):

```rust
core.cfg                    // Config: proc, sys, root, docker_sock, interval, demo, data_dir …
core.chart(ChartDef) -> ChartHandle        // Diagramm anmelden (beliebig oft, billig wenn unverändert)
core.submit(frame)                         // Werte eines Takts abgeben
core.snapshot("docker.containers", &daten) // Liste/Zustand ablegen (serde), geht an Abonnenten
core.event(Event::new("docker", "container.start", Severity::Info, name, "Container gestartet"))
core.settings.get::<T>("dns") / .set("dns", &t) / .subscribe()   // Einstellungen je Bereich
core.db.conn()?             // eigene DuckDB-Verbindung (blockiert → spawn_blocking / eigener Thread)
core.http                   // reqwest::Client (Zeitlimit 20 s)
core.registry / core.ring / core.store     // Katalog, RAM-Puffer, Speicher
```

### Diagramme anmelden

```rust
use shared::catalog::system::*;
let cpu  = core.chart(ChartDef::tpl(&SYSTEM_CPU));                       // ID = Kontext
let disk = core.chart(ChartDef::tpl(&DISK_IO).instance("nvme0n1"));      // ID = disk.io.nvme0n1, Label device=nvme0n1
let cores = core.chart(ChartDef::tpl(&CPU_CORES).dims((0..n).map(|i| (format!("cpu{i}"), format!("Kern {i}")))));
let c = core.chart(ChartDef::tpl(&DOCKER_CPU).instance(&name).label("app", app).label("image", image));
core.registry.retire("docker.cpu.alt");   // Container weg: Diagramm liefert nichts mehr, alte Daten bleiben
```

- Vorlagen stehen im **Katalog** (`shared/src/catalog/*.rs`). Die dort vorhandenen Kontexte, ihre
  Dimensionen und Einheiten sind fest (die Oberfläche baut darauf). Neue Vorlagen darf der Besitzer
  der Katalogdatei ergänzen (auch in `ALL` eintragen); bestehende nicht umbenennen.
- Dimensionen zur Laufzeit (`dims: &[]` im Katalog): stabile Reihenfolge halten, neue **anhängen**.
  Bei „je Container/Programm“-Summen höchstens ~12 Dimensionen: die grössten einzeln, Rest als `other`.
- Labels machen Diagramme auffindbar (`container`, `app`, `image`, `device`, `iface`, `mount` …).

### Werte abgeben

Lokale Quellen im Takt (eigener Thread, genau auf der Sekunde):

```rust
struct Cpu { chart: ChartHandle, prev: Counter, … }
impl Collector for Cpu {
    fn name(&self) -> &'static str { "cpu" }
    fn collect(&mut self, t: &mut Tick<'_>) -> anyhow::Result<()> {
        t.put(&self.chart, &[user, nice, system, …]);      // Reihenfolge der Dimensionen, NaN = kein Wert
        if t.due(10) { t.put(&self.space, &[used, avail, reserved]); }   // seltener: passend zu Tpl::every(10)
        Ok(())
    }
}
collectors::spawn(core, Cpu { … });
```

APIs und Ereignisströme frei: `tokio::spawn(async move { loop { let mut f = Frame::new(ts); f.put(&h, &[…]); core.submit(f); … } })`.

Regeln für Werte:

- **Zähler nie roh**, immer als Rate je Sekunde (`util::Counter::rate(wert, t.dt)`; erster Wert und
  Rücksetzer liefern `None` → `util::nan(..)`).
- **Vor dem Abgeben runden** (`util::round`): Prozent 2 Stellen, Temperatur 1, Bytes/Raten 0, Millisekunden 2.
  Gerundete Werte komprimieren um ein Vielfaches besser – das ist Teil des sparsamen Speicherns.
- Einheiten wie im Katalog: Bytes (nicht KiB), Bit/s für Netzwerk, Prozent 0–100 (CPU je Container:
  100 % = ein Kern), Millisekunden, Sekunden.
- `frame.ts` darf bis zu 15 Minuten zurückliegen (Cloudflare liefert verzögert) – jede Minute nur einmal.
- Ein Sammler darf nie den Takt blockieren: kein Netzwerk, kein `docker`-Aufruf im Takt-Thread;
  teure Dinge seltener (`t.due(n)`) oder in eine eigene Aufgabe.
- Fehler einzelner Quellen (Datei fehlt, Recht fehlt) still überspringen – nicht jedes System hat alles.

### Beispieldaten (`core.cfg.demo`)

Auf dem Mac (und mit `NRZMON_DEMO=1`) gibt es kein `/proc`. **Jeder Bereich liefert darum in
`start()` bei `core.cfg.demo` glaubwürdige Beispieldaten für alle seine Diagramme, Momentaufnahmen
und ein paar Ereignisse** (`util::Walk` hilft). Damit lässt sich die Oberfläche überall entwickeln
und fotografieren. Bereich 4: Beispieldaten nur, solange keine echten Zugangsdaten hinterlegt sind –
die HTTP-Sammler laufen überall echt.

### Momentaufnahmen

Für alles, was keine Zeitreihe ist. Schlüssel `bereich.name`, Typ in `shared/src/<bereich>.rs`
(mit `#[serde(default)]` an Strukturen, damit Felder dazukommen können). Fest vereinbart:

| Schlüssel | Typ | Besitzer |
|---|---|---|
| `system.info` | `shared::system::HostInfo` | 2 |
| `alerts.active` | `Vec<shared::alerts::ActiveAlert>` | 3 |

Weitere Schlüssel legt jeder Bereich selbst fest (z. B. `system.processes`, `docker.containers`,
`netbird.peers`) und nutzt sie in seinen eigenen Seiten.

### HTTP-API

Jeder Bereich hat `server/src/api/<bereich>.rs` mit `pub fn routes() -> Router<Core>` (schon in
`api/mod.rs` eingehängt). Pfade unter `/api/v1/<bereich>/…`, JSON, Typen aus `shared`.
Fehler: `ApiErr::bad("…")`, `ApiErr::not_found("…")`, `?` auf `anyhow::Result` (→ 500). Rückgabe `ApiResult<T>`.
Die Anmeldung (falls `NRZMON_PASSWORD` gesetzt) schützt alle Routen von selbst. Geheimnisse (Tokens)
nie im Klartext zurückgeben – nur `gesetzt: true` oder maskiert.

Vorhanden: `GET /info`, `GET /charts?section=&context=&retired=`, `POST /query`, `GET /events`,
`GET /snap`, `GET /snap/{key}`, `GET /live` (WebSocket, `shared::query::{LiveCmd, LiveMsg}`), `POST /login`, `/logout`.

### Einstellungen

`core.settings` speichert je Bereich ein JSON-Objekt in `<data>/settings.json` (Schlüssel = Bereich:
`storage`, `alerts`, `dns`, `cloudflare`, `netbird`, `probes`). Der Typ steht in `shared`. Sammler
lesen beim Start und nach `core.settings.subscribe()` neu – Änderungen wirken ohne Neustart.
Umgebungsvariablen (`NRZMON_…`) dürfen Standardwerte vorgeben.

## 4. Speicher (Bereich 1)

Ziel: **alles behalten, wenig belegen**. Vorgabe:

- **RAM** (`ring.rs`, steht): letzte 15 min in voller Auflösung → Live-Ansicht ohne Datenträger.
- **Stufe 0 – Sekunden**: jeder Wert; Standard 24 h.
- **Stufe 1 – Minuten** (Mittel, Minimum, Maximum je Reihe): Standard 30 Tage.
- **Stufe 2 – Stunden** (Mittel, Minimum, Maximum): Standard 2 Jahre.
- Aufbewahrung je Stufe und eine **Obergrenze für den Speicherplatz** einstellbar (Seite „Speicher“);
  wird die Grenze erreicht, fällt zuerst das Älteste der feinsten Stufe weg.
- `Store::ingest` kehrt sofort zurück (nur puffern); geschrieben wird gebündelt in einem eigenen
  Thread (Appender). Frisch gepufferte Werte in DuckDB-Tabellen, abgeschlossene Abschnitte sortiert
  nach `(series, ts)` als **Parquet mit ZSTD** unter `<data>/…` (ein Abschnitt = eine Datei → Löschen
  ist Datei löschen, die Datenbank wächst nicht). Die mitgelieferte DuckDB-Bibliothek kann Parquet/JSON.
- Abfragen wählen die Stufe nach Zeitraum und Punktzahl und mischen RAM + Datenträger auf ein Raster.
- Gemessen wird in **Bytes je Wert** – das Ziel ist ≤ 1 Byte im Sekundenarchiv. Zahlen auf der Seite „Speicher“.
- DuckDB bleibt klein eingestellt (`NRZMON_DB_MEMORY`, Standard 256 MB; 2 Threads).

Die Schnittstelle `Store::{open, ingest, query, add_event, events, shutdown, samples_total}` bleibt –
`core.rs`, `api/base.rs` und die Oberfläche hängen daran. Ereignisse gehören dauerhaft in DuckDB.

## 5. Laufzeit-Umgebung

Im Container (Host-Modus): `/proc` → `/host/proc`, `/sys` → `/host/sys`, `/` → `/host/root` (alles nur
lesend), Docker-Socket `/var/run/docker.sock`, `pid: host`. Die Pfade stehen in `core.cfg.{proc,sys,root,docker_sock}` –
**nie** `/proc` oder `/sys` fest verdrahten. Zielgerät: umbrelOS auf x86_64 (Intel i5-6500T, 16 GB,
NVMe), ~80 Container; zusätzlich arm64.

| Variable | Standard | Zweck |
|---|---|---|
| `NRZMON_BIND` | `0.0.0.0:8571` | Adresse(n) von Oberfläche und API |
| `NRZMON_DATA_DIR` | `data` | DuckDB, Parquet, `settings.json` |
| `NRZMON_HOST_PROC` / `_SYS` / `_ROOT` | `/host/…` falls vorhanden | Dateisysteme des Hosts |
| `NRZMON_DOCKER_SOCK` | `/var/run/docker.sock` | Docker-API |
| `NRZMON_INTERVAL` | `1` | Sekunden zwischen Messungen |
| `NRZMON_LIVE_WINDOW` | `900` | Sekunden im RAM |
| `NRZMON_DEMO` | aus (an ohne Linux) | Beispieldaten |
| `NRZMON_PASSWORD` | – | Anmeldung verlangen |
| `NRZMON_WEB_DIR` | – | Oberfläche aus Ordner statt eingebaut |
| `NRZMON_DB_MEMORY` / `_THREADS` | `256MB` / `2` | DuckDB klein halten |
| `NRZMON_MCP_TOKEN` | erzeugt | Schlüssel für `/mcp` (sonst in den Einstellungen) |
| `NRZMON_MCP_WRITE` | aus | MCP darf auch ändern |
| `NRZMON_MCP_URL` | aus `NRZMON_BIND` | Ziel der stdio-Brücke `nrzmonitor mcp` |

### MCP und Dashboards

`server/src/mcp/` bietet alles, was die API kann, nach aussen an – mit eigenem Schlüssel (`settings.json`,
Schlüssel `mcp`, beim ersten Start erzeugt; `NRZMON_MCP_TOKEN` hat Vorrang). Das Passwort der Oberfläche
ersetzt ihn nur, wenn `NRZMON_PASSWORD` gesetzt ist. Unter umbrelOS lässt der App-Proxy `/mcp*` ohne
umbrelOS-Anmeldung durch (`PROXY_AUTH_WHITELIST`); andere Container erreichen `http://10.21.0.1:8571/mcp`.

| Pfad | Zweck |
|---|---|
| `POST/GET/DELETE /mcp` | MCP „Streamable HTTP“ (2025-06-18, auch 2025-03-26/2024-11-05): Werkzeuge, Ressourcen mit Abo, Prompts, Logging, Vervollständigung |
| `GET /mcp/stream` | Server-Sent Events für Dashboards: `hello`, `latest`, `tick`, `snap`, `event`, `catalog` |
| `/mcp/api/v1/…` | die ganze REST-API mit dem Schlüssel (ändernd nur mit `allow_write`) |
| `nrzmonitor mcp` | Brücke stdin/stdout → `/mcp` (`docker exec -i … nrzmonitor mcp`) |

Werkzeuge rufen intern dieselben Routen auf (`api::routes()` ohne Anmeldung) – neue Endpunkte eines Bereichs
sind damit über `api_request` und `/mcp/api/v1` sofort erreichbar. Eigene Werkzeuge: `mcp/tools.rs` (`defs()` + `run()`).

## 6. Oberfläche: Regeln

- **nrz-Design** (Quelle: nrzUI `DESIGN.md`): dunkel, nur Grautöne, ruhig und dicht. Nur Theme-Klassen
  (`bg-background`, `bg-card`, `bg-muted`, `text-muted-foreground`, `border`, `bg-accent` …), **keine festen
  Farben** (`bg-neutral-900`, `#141414`, `text-red-400`). Farbe nur für Zustände: `success`, `warning`,
  `destructive`, `info` – und für Datenreihen (siehe unten).
- Texte **Deutsch**, Schweizer Schreibweise (ss statt ß), kurz und konkret. Code-Kommentare deutsch.
- **Alles Rust**: keine CSS-, JavaScript- oder Shell-Dateien und kein Dockerfile anlegen (Ausnahme:
  `umbrel/hooks/pre-start`, den umbrelOS als Shell-Skript verlangt). Klassen schreibt man wie bei
  Tailwind v4 (`flex`, `gap-4`, `hover:bg-accent`, `sm:grid-cols-2`, `[&_svg]:size-4` …); das CSS dazu erzeugt
  `web/style` beim Bauen (`build.rs`) aus allen Klassen in `web/src`. Klassen immer vollständig hinschreiben,
  nicht zusammensetzen (`format!("p-{n}")` findet der Erzeuger nicht). Kennt er eine Klasse in `class: "…"`
  nicht, schlägt `(cd web && cargo test -p nrzmon-style)` fehl („Klassen ohne CSS“, `cargo check` warnt) –
  dann in `web/style/src/utilities.rs` ergänzen. Farben, Radius und Schrift des Themas stehen in
  `web/style/src/theme.rs`.
- Seitenaufbau immer mit den nrzUI-Bausteinen aus `components/ui/app_shell.rs`: `PageContainer`
  (`width: PageWidth::Full` für Diagrammseiten), `PageHeader`, `PageSection`, `SettingsGroup`,
  `SettingsRow`, `EmptyState`, `LoadingState`. Weitere Bausteine in `components/ui/` (Button, Card, Badge,
  Input, Select, Switch, Tabs, Table, Tooltip, Dialog, Sheet, Popover, DropdownMenu, Command, Toast,
  Skeleton, Progress, Checkbox, Pagination …) – benutzen statt nachbauen, nicht verändern.
  Ihr Verhalten (Öffnen, Schliessen, Tastatur) richten sie mit den Helfern in `components/ui/dom.rs` ein,
  feste Regeln dazu stehen in `web/style/src/base.rs` (`NRZ_UI`). Meldungen mit `toast::show_toast` – der
  `ToastToaster` steht einmal in `layout.rs`.
- Grössen: Seitentitel `text-2xl font-semibold`, Text `text-sm`, Hinweise `text-xs text-muted-foreground`,
  Zahlen `tabular-nums`, Karten `rounded-xl border bg-card shadow-sm`, Abstände `gap-4`/`gap-6`.
- Jede Seite muss am Handy (375 px) und am Desktop (1600 px) gut aussehen.
- Zahlen formatieren mit `shared::Unit::format` (z. B. `Unit::Bytes.format(v)`), Zeiten mit `crate::fmt`.

### Diagramme richtig benutzen

- **Form zuerst**: Verlauf → Diagramm; ein einzelner Stand → Kennzahl (`StatTile`); Anteil an einer
  Grenze → `Meter`; Rangliste → Balkenliste/Tabelle. Nie zwei y-Achsen in einem Diagramm.
- **Farben der Reihen**: acht feste Töne `var(--series-1)` … `var(--series-8)` in fester Reihenfolge
  (`chart::color(slot)`), geprüft farbenblind-sicher. Die Farbe gehört zur Reihe (Platz in der Liste),
  nie zum Rang. Mehr als acht Reihen: Rest neutral (`--series-other`) oder als „Andere“ zusammenfassen.
- Text trägt nie die Reihenfarbe – eine Farbmarke steht daneben. Zustandsfarben (`success`/`warning`/
  `destructive`) nie als Reihenfarbe und immer mit Text oder Symbol.
- Linien 1.75 px, Flächen als zarte Tönung, Hilfslinien haarfein. Ab zwei Reihen eine Legende.
- Der **Zeitraum** gilt für die ganze Seite (Leiste oben, `use_app().range`); keine eigenen
  Zeitraum-Schalter je Diagramm. Beim Neuladen bleibt das alte Bild stehen (kein Flackern).

## 7. Oberfläche: Bausteine

```rust
use crate::components::chart::{ChartCard, SectionCharts, ContextCharts, Plot, PlotData, PlotSeries, use_chart, color};
use crate::components::chart::section::ChartGrid;
use crate::components::widgets::{StatTile, Meter, StatusDot, Tone, Sparkline, use_latest, show};
use crate::components::lazy::Lazy;
use crate::{api, fmt, live::{use_snap, use_tick, use_events}, state::{use_app, TimeRange}};
use shared::catalog::system::*;   // Kontexte als Konstanten, z. B. SYSTEM_CPU.context
```

| Baustein | Zweck |
|---|---|
| `ChartCard { id, height?, title?, compact? }` | Fertige Karte zu einem Diagramm (Katalog-ID): folgt dem Zeitraum, läuft live, Legende, Fadenkreuz über alle Diagramme, Ziehen = Zoomen. In Listen `key: "{id}"`. |
| `SectionCharts { section }` | Alle Diagramme eines Bereichs, nach Familie gruppiert, erst im Bild aufgebaut. |
| `ContextCharts { context }` | Alle Instanzen eines Kontexts (z. B. `net.traffic` je Schnittstelle). |
| `ChartGrid { … }` | Raster für Karten (1/2/3 Spalten). |
| `Plot { data: PlotData, kind, unit, height }` | Nur die Zeichenfläche für **eigene** Daten (z. B. aus einem eigenen Endpunkt). |
| `use_chart(id) -> Signal<Option<ChartData>>` | Daten eines Diagramms für eigene Darstellungen. |
| `use_latest(id) -> Memo<Option<Vec<Option<f64>>>>` | Neueste Werte eines Diagramms (Kennzahlen). |
| `use_snap::<T>(key) -> Memo<Option<T>>` | Momentaufnahme, immer aktuell (Live-Kanal). |
| `use_events() -> Signal<Vec<Event>>` | Laufende Ereignisse. |
| `use_app()` | `.charts` (Katalog), `.chart(id)`, `.by_context(ctx)`, `.by_section(s)`, `.by_label(ctx, key, wert)`, `.range`, `.zoom(a, b)`, `.info` |
| `api::get/post/put/delete`, `api::qs` | Aufrufe der eigenen Endpunkte (`/dns/queries` → `/api/v1/dns/queries`). |
| `Lazy { min_height, … }` | Inhalt erst im Bild aufbauen (für lange Listen von Diagrammen). |
| `StatTile`, `Meter`, `StatusDot`, `Sparkline` | Kennzahl, Füllstand, Zustand, Mini-Verlauf. |

Diagramme eines Objekts finden: `app.by_label(DOCKER_CPU.context, "container", &name)` oder die ID
direkt: `DOCKER_CPU.chart_id(Some(&name))`.

Seiten: jede Seite ist eine Komponente mit dem Namen der Route (`web/src/route.rs`). Die vorhandenen
Platzhalter zeigen schon alle Diagramme ihres Bereichs (`SectionPage`) – daraus werden die richtigen
Seiten: oben Kennzahlen, dann die wichtigsten Diagramme gezielt angeordnet, Tabellen aus
Momentaufnahmen, Details per Klick. Einstellungen eines Bereichs: als Komponente exportieren
(`IntegrationSettings`, `StorageSettings`, `AlertSettings` – schon in `pages/settings.rs` eingehängt).

## 8. Entwickeln und Prüfen

```bash
cargo xtask dev                         # Mac: Oberfläche bauen + Server mit Beispieldaten, http://127.0.0.1:8571
NRZMON_PORT=8602 cargo xtask dev        # eigener Port (jeder Bereich hat seinen, siehe Auftrag)
(cd web && ~/.cargo/bin/dx build --platform web)   # Oberfläche neu bauen, Seite neu laden
cargo xtask run-linux                   # echte Sammler: Linux-Build im Container im Host-Modus (Docker-VM)
cargo xtask shot /system/cpu /tmp/cpu.png 1600 1100 8602    # Bildschirmfoto (Headless-Chrome im Docker)
cargo xtask shot /system/cpu /tmp/cpu-m.png 390 844 8602    # Handy
cargo xtask build-web                   # Oberfläche für die Auslieferung nach web/dist (wird eingebettet)
cargo test -p nrzmon-server -p nrzmon-shared
(cd web && cargo test --target "$(rustc -vV | sed -n 's/host: //p')")   # Rechenteil der Oberfläche
(cd web && cargo test -p nrzmon-style)  # CSS-Erzeuger, prüft auch: jede Klasse in class: "…" ergibt CSS
cargo clippy -p nrzmon-server -p nrzmon-shared --no-deps
```

`dx` ist die dioxus-cli unter `~/.cargo/bin/dx` (das `dx` im PATH ist Deno). Fertig ist ein Bereich
erst, wenn Server **und** Oberfläche ohne Fehler bauen, die Tests laufen und die eigenen Seiten auf
Bildschirmfotos (Desktop und Handy) gut aussehen. Arbeit auf dem eigenen Zweig committen (kleine,
beschriebene Commits, Nachrichten deutsch ohne Umlaute).
