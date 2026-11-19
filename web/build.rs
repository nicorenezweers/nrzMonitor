//! Erzeugt das Stylesheet der Oberfläche aus den Klassen im Quelltext (Crate `style/`, ersetzt
//! Tailwind). Ergebnis: `$OUT_DIR/nrz.css`, eingebunden in `src/main.rs`.

fn main() {
    println!("cargo:rerun-if-changed=src");
    let out = nrzmon_style::build(std::path::Path::new("src")).expect("Quelltexte unter src/ nicht lesbar");
    for class in &out.unknown {
        println!("cargo:warning=Klasse ohne CSS: `{class}` – Schreibweise prüfen oder in style/src/utilities.rs ergänzen");
    }
    let path = std::path::PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR fehlt")).join("nrz.css");
    std::fs::write(path, out.css).expect("nrz.css nicht schreibbar");
}
