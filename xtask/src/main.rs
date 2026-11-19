//! Entwicklungs- und Release-Befehle von nrzMonitor (cargo-xtask-Muster): `cargo xtask <befehl>`.
//! Ersetzt die früheren Shell-Skripte unter `scripts/` und das `Dockerfile`.

use std::env;
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow, bail};
use flate2::Compression;
use flate2::write::GzEncoder;
use sha2::{Digest, Sha256};

const HILFE: &str = "\
nrzMonitor – Befehle: cargo xtask <befehl>

  build-web                       Oberfläche (Release) nach web/dist bauen, Dateien zusätzlich als .gz
  dev [--no-web]                  Oberfläche (Debug) bauen und Server mit Beispieldaten starten
                                  (NRZMON_PORT, Standard 8571)
  run-linux [--demo]              Linux-Build im Container im Host-Modus (echte Sammler)
  shot <route> <datei> [breite] [hoehe] [port]
                                  Bildschirmfoto mit Headless-Chrome im Docker (1600x1100, Port 8571)
  docker-image <version>          Multi-Arch-Image bauen und in die Registry pushen (NRZMON_IMAGE)
  umbrel-release <version> [notizen]
                                  umbrelOS-Release: Zweig `umbrel`, Tag `umbrel-<version>`, dann main pushen
";

/// Image für `docker-image` (früher das `Dockerfile`). Programm und DuckDB-Bibliothek liegen vorher
/// für beide Architekturen in dist/<arch>/; der Inhalt geht per stdin an `docker buildx build -f -`.
const DOCKERFILE: &str = r#"# nrzMonitor als eigenes Image (ausserhalb von umbrelOS), gebaut mit `cargo xtask docker-image`.
FROM debian:trixie-slim
ARG TARGETARCH
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates tzdata \
 && rm -rf /var/lib/apt/lists/*
COPY dist/${TARGETARCH}/nrzmonitor dist/${TARGETARCH}/libduckdb.so /app/
ENV NRZMON_BIND=0.0.0.0:8571 NRZMON_DATA_DIR=/data
VOLUME /data
EXPOSE 8571
HEALTHCHECK --interval=30s --timeout=5s CMD ["/app/nrzmonitor", "healthcheck"]
ENTRYPOINT ["/app/nrzmonitor"]
"#;

/// Bundle-Ordner von `dx bundle --release` und `dx build` (relativ zu web/).
const WEB_RELEASE: &str = "target/dx/nrzmon-web/release/web/public";
const WEB_DEBUG: &str = "target/dx/nrzmon-web/debug/web/public";

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let befehl = args.first().map(String::as_str).unwrap_or("");
    let rest = args.get(1..).unwrap_or(&[]);
    let ergebnis = match befehl {
        "build-web" => build_web(),
        "dev" => dev(rest),
        "run-linux" => run_linux(rest),
        "shot" => shot(rest),
        "docker-image" => docker_image(rest),
        "umbrel-release" => umbrel_release(rest),
        "" | "help" | "hilfe" | "-h" | "--help" => {
            print!("{HILFE}");
            return if befehl.is_empty() { ExitCode::from(2) } else { ExitCode::SUCCESS };
        }
        _ => {
            eprintln!("Unbekannter Befehl: {befehl}\n");
            eprint!("{HILFE}");
            return ExitCode::from(2);
        }
    };
    match ergebnis {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("Fehler: {e:#}");
            ExitCode::FAILURE
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Befehle
// ---------------------------------------------------------------------------------------------

/// Oberfläche für die Auslieferung bauen: Release-Bundle nach web/dist, Dateien zusätzlich als .gz.
/// Der Server bettet web/dist beim Bauen ein (rust-embed) – also zuerst dies, dann cargo build.
fn build_web() -> Result<()> {
    let web = root()?.join("web");
    let dx = find_dx()?;
    // dx räumt den Ausgabeordner nicht auf: alte Dateien früherer Bundles nicht mit einbetten
    let out = web.join(WEB_RELEASE);
    if out.exists() {
        fs::remove_dir_all(&out)?;
    }
    run(Command::new(&dx).args(["bundle", "--platform", "web", "--release"]).current_dir(&web))?;
    if !out.join("index.html").is_file() {
        bail!("Bundle fehlt: {}", out.display());
    }
    let dist = web.join("dist");
    fs::create_dir_all(&dist)?;
    for eintrag in fs::read_dir(&dist)? {
        let pfad = eintrag?.path();
        if pfad.file_name() == Some(OsStr::new(".gitkeep")) {
            continue;
        }
        if pfad.is_dir() {
            fs::remove_dir_all(&pfad)?;
        } else {
            fs::remove_file(&pfad)?;
        }
    }
    copy_dir(&out, &dist)?;
    // Vorkomprimieren: der Server liefert .gz aus, wenn der Browser es annimmt.
    for datei in files(&dist)? {
        let name = datei.file_name().and_then(OsStr::to_str).unwrap_or("");
        let auslassen = name == ".gitkeep"
            || [".gz", ".png", ".woff2"].iter().any(|endung| name.ends_with(endung));
        if !auslassen && fs::metadata(&datei)?.len() > 1024 {
            let mut gz = datei.clone().into_os_string();
            gz.push(".gz");
            gzip_file(&datei, Path::new(&gz))?;
        }
    }
    let summe: u64 = files(&dist)?.iter().map(|f| fs::metadata(f).map(|m| m.len()).unwrap_or(0)).sum();
    println!("web/dist: {}", human(summe));
    Ok(())
}

/// Entwicklung: Oberfläche bauen (Debug) und den Server starten, der sie ausliefert.
/// Auf dem Mac laufen Beispieldaten, echte Sammler brauchen Linux: `cargo xtask run-linux`.
/// Nach Änderungen an der Oberfläche: `(cd web && dx build --platform web)` und Seite neu laden.
fn dev(args: &[String]) -> Result<()> {
    let root = root()?;
    let port = env_or("NRZMON_PORT", "8571");
    if args.first().map(String::as_str) != Some("--no-web") {
        let dx = find_dx()?;
        run(Command::new(&dx).args(["build", "--platform", "web"]).current_dir(root.join("web")))?;
    }
    let mut cmd = Command::new("cargo");
    cmd.args(["run", "-p", "nrzmon-server"])
        .current_dir(&root)
        .env("NRZMON_BIND", env_or("NRZMON_BIND", &format!("127.0.0.1:{port}")))
        .env("NRZMON_DATA_DIR", env_or("NRZMON_DATA_DIR", &format!("/tmp/nrzmon-dev-{port}")))
        .env("NRZMON_WEB_DIR", root.join("web").join(WEB_DEBUG));
    exec(cmd)
}

/// Echte Sammler testen: Server für Linux bauen (zig) und in einem Container im Host-Modus starten.
/// Auf dem Mac ist der "Host" die Docker-VM (colima). Die Oberfläche kommt aus dem Debug-Build,
/// sonst aus web/dist.
fn run_linux(args: &[String]) -> Result<()> {
    let root = root()?;
    let port = env_or("NRZMON_PORT", "8571");
    let arch = Command::new("docker")
        .args(["info", "--format", "{{.Architecture}}"])
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|a| !a.is_empty())
        .unwrap_or_else(|| env::consts::ARCH.to_string());
    let triple = triple_for(&arch)?;
    run(Command::new("cargo")
        .args(["zigbuild", "-p", "nrzmon-server", "--target", &format!("{triple}.2.31")])
        .current_dir(&root))?;
    let bin = root.join("target").join(triple).join("debug/nrzmonitor");
    let lib = find_libduckdb(&root, triple)
        .ok_or_else(|| anyhow!("libduckdb.so für {triple} nicht gefunden"))?;
    let stage = root.join(format!("target/run-linux-{port}"));
    if stage.exists() {
        fs::remove_dir_all(&stage)?;
    }
    fs::create_dir_all(stage.join("app"))?;
    fs::create_dir_all(stage.join("data"))?;
    copy_file(&bin, &stage.join("app/nrzmonitor"))?;
    copy_file(&lib, &stage.join("app/libduckdb.so"))?;
    let mut web = root.join("web").join(WEB_DEBUG);
    if !web.join("index.html").is_file() {
        web = root.join("web/dist");
    }
    copy_dir(&web, &stage.join("app/web"))?;
    let demo = if args.first().map(String::as_str) == Some("--demo") { "1" } else { "0" };
    let name = format!("nrzmon-dev-{port}");
    let _ = Command::new("docker")
        .args(["rm", "-f", &name])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    println!("==> http://127.0.0.1:{port}  (Container {name}, Host-Netzwerk der Docker-VM)");
    let mut cmd = Command::new("docker");
    cmd.args(["run", "--rm", "--name", &name, "--init", "--network", "host", "--pid", "host"])
        .arg("-v")
        .arg(format!("{}:/app:ro", stage.join("app").display()))
        .args(["-v", "/proc:/host/proc:ro", "-v", "/sys:/host/sys:ro", "-v", "/:/host/root:ro"])
        .args(["-v", "/var/run/docker.sock:/var/run/docker.sock:ro"])
        .args(["-e", &format!("NRZMON_BIND=0.0.0.0:{port}")])
        .args(["-e", "NRZMON_DATA_DIR=/data", "-e", "NRZMON_WEB_DIR=/app/web"])
        .args(["-e", &format!("NRZMON_DEMO={demo}")])
        .args(["-e", &format!("RUST_LOG={}", env_or("RUST_LOG", "info"))])
        .args(["--tmpfs", "/data:size=512m", "debian:trixie-slim", "/app/nrzmonitor"]);
    exec(cmd)
}

/// Bildschirmfoto der Oberfläche mit Headless-Chrome im Docker. Der Server muss laufen.
fn shot(args: &[String]) -> Result<()> {
    let arg = |i: usize, standard: &str| args.get(i).cloned().unwrap_or_else(|| standard.to_string());
    let route = arg(0, "/");
    let out = arg(1, "shot.png");
    let breite = arg(2, "1600");
    let hoehe = arg(3, "1100");
    let port = args.get(4).cloned().unwrap_or_else(|| env_or("NRZMON_PORT", "8571"));
    let name = format!("nrzmon-shot-{}", std::process::id());
    // Chrome endet oft mit Fehlercode, obwohl das Bild da ist – wie bisher nicht prüfen.
    let _ = Command::new("docker")
        .args(["run", "--name", &name, "--add-host=host.docker.internal:host-gateway"])
        .args(["zenika/alpine-chrome", "--no-sandbox", "--headless", "--disable-gpu", "--hide-scrollbars"])
        .arg(format!("--window-size={breite},{hoehe}"))
        .args(["--virtual-time-budget=9000", "--screenshot=/tmp/shot.png"])
        .arg(format!("http://host.docker.internal:{port}{route}"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let kopiert = run(Command::new("docker")
        .args(["cp", &format!("{name}:/tmp/shot.png"), &out])
        .stdout(Stdio::null()));
    // Container immer entfernen, auch wenn das Kopieren scheiterte.
    let entfernt = run(Command::new("docker").args(["rm", "-f", &name]).stdout(Stdio::null()));
    kopiert?;
    entfernt?;
    println!("{out}");
    Ok(())
}

/// Baut das Multi-Arch-Image (amd64 + arm64) und pusht es in die Registry von nrzGit.
fn docker_image(args: &[String]) -> Result<()> {
    let version = args.first().ok_or_else(|| anyhow!("Version fehlt, z.B. 1.0.0"))?;
    let image = env_or("NRZMON_IMAGE", "100.78.107.162:8390/nicorenezweers/nrz-monitor");
    let root = root()?;
    build_web()?;
    let dist = root.join("dist");
    if dist.exists() {
        fs::remove_dir_all(&dist)?;
    }
    for arch in ["amd64", "arm64"] {
        let triple = triple_for(arch)?;
        zigbuild_release(&root, triple)?;
        let ziel = dist.join(arch);
        fs::create_dir_all(&ziel)?;
        copy_file(&root.join("target").join(triple).join("release/nrzmonitor"), &ziel.join("nrzmonitor"))?;
        let lib = find_libduckdb(&root, triple)
            .ok_or_else(|| anyhow!("libduckdb.so für {triple} nicht gefunden"))?;
        copy_file(&lib, &ziel.join("libduckdb.so"))?;
    }
    let tag = format!("{image}:{version}");
    let mut kind = Command::new("docker")
        .args(["buildx", "build", "--platform", "linux/amd64,linux/arm64", "-t", &tag, "--push", "-f", "-", "."])
        .current_dir(&root)
        .stdin(Stdio::piped())
        .spawn()
        .context("docker konnte nicht gestartet werden")?;
    kind.stdin.take().context("stdin von docker fehlt")?.write_all(DOCKERFILE.as_bytes())?;
    let status = kind.wait()?;
    if !status.success() {
        bail!("docker buildx build fehlgeschlagen ({status})");
    }
    println!("==> {tag}");
    Ok(())
}

/// Baut nrzMonitor für umbrelOS (Linux amd64 + arm64), legt die fertigen Dateien auf den Zweig
/// `umbrel` und pusht den Tag `umbrel-<version>`, zuletzt `main`. nrzGit trägt die Version dann per
/// nrzStore.json selbst im nrzStore ein.
/// Voraussetzungen: dx (dioxus-cli 0.7), zig + cargo-zigbuild, Rust-Targets x86_64/aarch64-unknown-linux-gnu.
fn umbrel_release(args: &[String]) -> Result<()> {
    let version = args.first().ok_or_else(|| anyhow!("Version fehlt, z.B. 1.0.1"))?;
    let notizen = args.get(1).cloned().unwrap_or_else(|| format!("nrzMonitor {version}"));
    let tag = format!("umbrel-{version}");
    let root = root()?;

    if !git_output(&root, &["status", "--porcelain"])?.trim().is_empty() {
        bail!("Arbeitsverzeichnis nicht sauber - erst committen.");
    }
    // Version in umbrel/ muss zum Release passen (umbrel-app.yml und RELEASE in hooks/pre-start).
    if !has_line_prefix(&root.join("umbrel/umbrel-app.yml"), &format!("version: \"{version}\""))? {
        bail!("umbrel/umbrel-app.yml hat nicht version: \"{version}\"");
    }
    if !has_line_prefix(&root.join("umbrel/hooks/pre-start"), &format!("RELEASE=\"{tag}\""))? {
        bail!("umbrel/hooks/pre-start hat nicht RELEASE=\"{tag}\"");
    }
    if git_ok(&root, &["rev-parse", "-q", "--verify", &format!("refs/tags/{tag}")]) {
        bail!("Tag {tag} gibt es schon.");
    }

    // Oberfläche einmal bauen; sie wird in beide Programme eingebettet.
    build_web()?;

    let out = TempDir::new("nrzmon-release")?;
    for arch in ["amd64", "arm64"] {
        let triple = triple_for(arch)?;
        println!("==> Baue {arch} ({triple})");
        zigbuild_release(&root, triple)?;
        let bin = root.join("target").join(triple).join("release/nrzmonitor");
        let lib = find_libduckdb(&root, triple)
            .ok_or_else(|| anyhow!("libduckdb.so für {triple} fehlt"))?;
        // Oberfläche muss eingebaut sein (sonst zeigt der Server nur die Hinweisseite).
        let inhalt = fs::read(&bin).with_context(|| format!("{} nicht lesbar", bin.display()))?;
        if !contains(&inhalt, b"nrzmon-web_bg") {
            bail!("Oberflaeche nicht eingebettet ({arch})");
        }
        gzip_file(&bin, &out.path.join(format!("nrzmonitor-{arch}.gz")))?;
        gzip_file(&lib, &out.path.join(format!("libduckdb-{arch}.so.gz")))?;
    }
    let namen = ["nrzmonitor-amd64.gz", "nrzmonitor-arm64.gz", "libduckdb-amd64.so.gz", "libduckdb-arm64.so.gz"];
    fs::write(out.path.join("SHA256SUMS"), sha256sums(&out.path, &namen)?)?;
    for eintrag in fs::read_dir(&out.path)? {
        let eintrag = eintrag?;
        println!("{:>12}  {}", eintrag.metadata()?.len(), eintrag.file_name().to_string_lossy());
    }

    let wt = Worktree::new(&root)?;
    let _ = Command::new("git")
        .args(["fetch", "origin", "umbrel"])
        .current_dir(&root)
        .stderr(Stdio::null())
        .status();
    let wt_pfad = wt.dir.path.to_string_lossy().to_string();
    if git_ok(&root, &["rev-parse", "-q", "--verify", "origin/umbrel"]) {
        git(&root, &["worktree", "add", "-q", &wt_pfad, "-B", "umbrel", "origin/umbrel"])?;
    } else {
        git(&root, &["worktree", "add", "-q", "--detach", &wt_pfad])?;
        git(&wt.dir.path, &["checkout", "-q", "--orphan", "umbrel"])?;
        let _ = Command::new("git")
            .args(["rm", "-rqf", "."])
            .current_dir(&wt.dir.path)
            .stderr(Stdio::null())
            .status();
    }
    for name in namen.iter().chain(&["SHA256SUMS"]) {
        fs::copy(out.path.join(name), wt.dir.path.join(name))?;
    }
    fs::write(wt.dir.path.join("README.md"), UMBREL_README)?;
    git(&wt.dir.path, &["add", "-A"])?;
    git(&wt.dir.path, &["commit", "-qm", &format!("nrzMonitor {version}: Umbrel-Paket")])?;
    git(&wt.dir.path, &["tag", "-a", &tag, "-m", &notizen])?;
    git(&wt.dir.path, &["push", "-q", "origin", "umbrel"])?;
    git(&wt.dir.path, &["push", "-q", "origin", &tag])?;
    drop(wt);
    // Erst jetzt main pushen: käme umbrel/ (neue Version) vor dem Tag in den Store, hielte nrzGit den
    // Download-Tag bis zum Tag auf dem alten Stand – umbrelOS könnte genau diesen Zwischenstand holen
    // und die neue Version mit dem alten Programm installieren.
    git(&root, &["push", "-q", "origin", "HEAD:main"])?;
    println!("==> {tag} veroeffentlicht und main gepusht. nrzGit aktualisiert den nrzStore.");
    Ok(())
}

const UMBREL_README: &str = "\
# nrzMonitor - Umbrel-Paket

Nur fertige Dateien fuer die App `nrz-monitor` im nrzStore (erstellt mit `cargo xtask umbrel-release`).
`hooks/pre-start` im Store laedt sie per Tag `umbrel-<version>`.

| Datei | Inhalt |
|-------|--------|
| `nrzmonitor-amd64.gz`, `nrzmonitor-arm64.gz` | Server mit eingebauter Oberflaeche (Linux, glibc 2.31) |
| `libduckdb-amd64.so.gz`, `libduckdb-arm64.so.gz` | DuckDB 2.0 (liegt neben dem Programm) |
| `SHA256SUMS` | Pruefsummen |
";

// ---------------------------------------------------------------------------------------------
// Helfer
// ---------------------------------------------------------------------------------------------

/// Repo-Wurzel (Elternordner von xtask/).
fn root() -> Result<PathBuf> {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .canonicalize()
        .context("Repo-Wurzel nicht gefunden")
}

fn env_or(name: &str, standard: &str) -> String {
    env::var(name).ok().filter(|v| !v.is_empty()).unwrap_or_else(|| standard.to_string())
}

/// Programm ausführen; bricht ab, wenn es fehlt oder mit Fehlercode endet.
fn run(cmd: &mut Command) -> Result<()> {
    let prog = cmd.get_program().to_string_lossy().to_string();
    let status = cmd.status().with_context(|| format!("{prog} konnte nicht gestartet werden"))?;
    if !status.success() {
        let args: Vec<_> = cmd.get_args().map(|a| a.to_string_lossy().to_string()).collect();
        bail!("`{prog} {}` fehlgeschlagen ({status})", args.join(" "));
    }
    Ok(())
}

/// Prozess durch das Programm ersetzen (Strg+C erreicht es direkt).
fn exec(mut cmd: Command) -> Result<()> {
    let prog = cmd.get_program().to_string_lossy().to_string();
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let fehler = cmd.exec();
        Err(anyhow!("{prog} konnte nicht gestartet werden: {fehler}"))
    }
    #[cfg(not(unix))]
    {
        let status = cmd.status().with_context(|| format!("{prog} konnte nicht gestartet werden"))?;
        std::process::exit(status.code().unwrap_or(1));
    }
}

fn git(dir: &Path, args: &[&str]) -> Result<()> {
    run(Command::new("git").args(args).current_dir(dir))
}

fn git_ok(dir: &Path, args: &[&str]) -> bool {
    Command::new("git")
        .args(args)
        .current_dir(dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

fn git_output(dir: &Path, args: &[&str]) -> Result<String> {
    let out = Command::new("git").args(args).current_dir(dir).output().context("git konnte nicht gestartet werden")?;
    if !out.status.success() {
        bail!("`git {}` fehlgeschlagen: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

fn zigbuild_release(root: &Path, triple: &str) -> Result<()> {
    run(Command::new("cargo")
        .args(["zigbuild", "--release", "-p", "nrzmon-server", "--target", &format!("{triple}.2.31")])
        .current_dir(root))
}

/// dioxus-cli finden: `$DX`, `$HOME/.cargo/bin/dx`, dann jedes `dx` im PATH (dort liegt oft zuerst
/// Denos dx). Gültig ist, wer bei `--version` „dioxus“ meldet.
fn find_dx() -> Result<PathBuf> {
    let mut kandidaten: Vec<PathBuf> = Vec::new();
    if let Some(dx) = env::var_os("DX").filter(|v| !v.is_empty()) {
        kandidaten.push(dx.into());
    }
    if let Some(home) = env::var_os("HOME").filter(|v| !v.is_empty()) {
        kandidaten.push(Path::new(&home).join(".cargo/bin/dx"));
    }
    if let Some(path) = env::var_os("PATH") {
        kandidaten.extend(env::split_paths(&path).map(|d| d.join("dx")).filter(|p| p.is_file()));
    }
    kandidaten
        .into_iter()
        .find(|c| {
            Command::new(c)
                .arg("--version")
                .stdin(Stdio::null())
                .output()
                .is_ok_and(|o| {
                    let text = format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
                    text.to_lowercase().contains("dioxus")
                })
        })
        .ok_or_else(|| anyhow!("dioxus-cli (dx) nicht gefunden – cargo binstall dioxus-cli@0.7.10"))
}

/// Rust-Ziel für eine Architektur (amd64 | arm64).
fn triple_for(arch: &str) -> Result<&'static str> {
    match arch {
        "amd64" | "x86_64" => Ok("x86_64-unknown-linux-gnu"),
        "arm64" | "aarch64" => Ok("aarch64-unknown-linux-gnu"),
        _ => bail!("unbekannte Architektur {arch}"),
    }
}

/// libduckdb.so des duckdb-Downloads für ein Ziel unter target/ suchen.
fn find_libduckdb(root: &Path, triple: &str) -> Option<PathBuf> {
    let marke = format!("duckdb-download/{triple}/");
    fn suche(dir: &Path, marke: &str) -> Option<PathBuf> {
        for eintrag in fs::read_dir(dir).ok()?.flatten() {
            let pfad = eintrag.path();
            let typ = eintrag.file_type().ok()?;
            if typ.is_dir() {
                if let Some(treffer) = suche(&pfad, marke) {
                    return Some(treffer);
                }
            } else if pfad.file_name() == Some(OsStr::new("libduckdb.so"))
                && pfad.to_string_lossy().contains(marke)
            {
                return Some(pfad);
            }
        }
        None
    }
    suche(&root.join("target"), &marke)
}

fn has_line_prefix(datei: &Path, prefix: &str) -> Result<bool> {
    let text = fs::read_to_string(datei).with_context(|| format!("{} nicht lesbar", datei.display()))?;
    Ok(text.lines().any(|l| l.starts_with(prefix)))
}

fn contains(heu: &[u8], nadel: &[u8]) -> bool {
    heu.windows(nadel.len()).any(|w| w == nadel)
}

/// Datei mit gzip Stufe 9 komprimieren (wie `gzip -9`, Original bleibt).
fn gzip_file(src: &Path, dst: &Path) -> Result<()> {
    let mut ein = BufReader::new(File::open(src).with_context(|| format!("{} nicht lesbar", src.display()))?);
    let aus = BufWriter::new(File::create(dst).with_context(|| format!("{} nicht schreibbar", dst.display()))?);
    let mut enc = GzEncoder::new(aus, Compression::best());
    io::copy(&mut ein, &mut enc)?;
    enc.finish()?.flush()?;
    Ok(())
}

fn sha256_hex(daten: &[u8]) -> String {
    Sha256::digest(daten).iter().map(|b| format!("{b:02x}")).collect()
}

/// Inhalt einer SHA256SUMS-Datei wie `shasum -a 256`: `<hex>  <name>` je Zeile.
fn sha256sums(dir: &Path, namen: &[&str]) -> Result<String> {
    let mut text = String::new();
    for name in namen {
        let daten = fs::read(dir.join(name)).with_context(|| format!("{name} nicht lesbar"))?;
        text.push_str(&format!("{}  {name}\n", sha256_hex(&daten)));
    }
    Ok(text)
}

/// Alle Dateien unter `dir` (rekursiv).
fn files(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut liste = Vec::new();
    for eintrag in fs::read_dir(dir)? {
        let eintrag = eintrag?;
        if eintrag.file_type()?.is_dir() {
            liste.extend(files(&eintrag.path())?);
        } else {
            liste.push(eintrag.path());
        }
    }
    Ok(liste)
}

/// Ordnerinhalt rekursiv kopieren (wie `cp -R src/. dst/`).
fn copy_dir(src: &Path, dst: &Path) -> Result<()> {
    fs::create_dir_all(dst)?;
    for eintrag in fs::read_dir(src).with_context(|| format!("{} nicht lesbar", src.display()))? {
        let eintrag = eintrag?;
        let ziel = dst.join(eintrag.file_name());
        if eintrag.file_type()?.is_dir() {
            copy_dir(&eintrag.path(), &ziel)?;
        } else {
            copy_file(&eintrag.path(), &ziel)?;
        }
    }
    Ok(())
}

fn copy_file(src: &Path, dst: &Path) -> Result<()> {
    fs::copy(src, dst).with_context(|| format!("{} → {} nicht kopierbar", src.display(), dst.display()))?;
    Ok(())
}

fn human(bytes: u64) -> String {
    let mut wert = bytes as f64;
    for einheit in ["B", "K", "M", "G"] {
        if wert < 1024.0 {
            return format!("{wert:.1}{einheit}");
        }
        wert /= 1024.0;
    }
    format!("{wert:.1}T")
}

/// Temporärer Ordner, wird beim Verlassen (auch bei Fehler) gelöscht.
struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(praefix: &str) -> Result<Self> {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        let path = env::temp_dir().join(format!("{praefix}-{}-{nanos}", std::process::id()));
        fs::create_dir(&path).with_context(|| format!("{} nicht anlegbar", path.display()))?;
        Ok(Self { path })
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

/// Git-Worktree in einem temporären Ordner; wird beim Verlassen (auch bei Fehler) abgemeldet.
struct Worktree {
    root: PathBuf,
    dir: TempDir,
}

impl Worktree {
    fn new(root: &Path) -> Result<Self> {
        Ok(Self { root: root.to_path_buf(), dir: TempDir::new("nrzmon-umbrel")? })
    }
}

impl Drop for Worktree {
    fn drop(&mut self) {
        let pfad = self.dir.path.to_string_lossy().to_string();
        if self.dir.path.join(".git").exists() {
            let _ = git(&self.root, &["worktree", "remove", "--force", &pfad]);
        }
        let _ = fs::remove_dir_all(&self.dir.path);
        let _ = Command::new("git").args(["worktree", "prune"]).current_dir(&self.root).status();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::read::GzDecoder;
    use std::io::Read;

    #[test]
    fn sha256_bekannter_wert() {
        assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }

    #[test]
    fn sha256sums_format_und_gzip_rundlauf() {
        let dir = TempDir::new("xtask-test").unwrap();
        let daten: Vec<u8> = (0..50_000u32).flat_map(|i| (i % 251).to_le_bytes()).collect();
        fs::write(dir.path.join("roh"), &daten).unwrap();
        gzip_file(&dir.path.join("roh"), &dir.path.join("a.gz")).unwrap();
        fs::write(dir.path.join("b.gz"), b"abc").unwrap();

        let mut zurueck = Vec::new();
        GzDecoder::new(File::open(dir.path.join("a.gz")).unwrap()).read_to_end(&mut zurueck).unwrap();
        assert_eq!(zurueck, daten);

        let text = sha256sums(&dir.path, &["b.gz", "a.gz"]).unwrap();
        let zeilen: Vec<&str> = text.lines().collect();
        assert_eq!(zeilen[0], "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad  b.gz");
        assert!(zeilen[1].ends_with("  a.gz") && zeilen[1].len() == 64 + 2 + 4);
        assert!(text.ends_with('\n'));
    }

    #[test]
    fn architekturen() {
        assert_eq!(triple_for("arm64").unwrap(), "aarch64-unknown-linux-gnu");
        assert_eq!(triple_for("x86_64").unwrap(), "x86_64-unknown-linux-gnu");
        assert!(triple_for("riscv").is_err());
    }
}
