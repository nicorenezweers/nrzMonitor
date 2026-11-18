// Die Oberflaeche (web/dist) wird per rust-embed eingebettet: bei jeder Aenderung neu bauen.
fn main() {
    println!("cargo:rerun-if-changed=../web/dist");
    if let Ok(entries) = std::fs::read_dir("../web/dist") {
        for e in entries.flatten() {
            println!("cargo:rerun-if-changed={}", e.path().display());
        }
    }
    if let Ok(entries) = std::fs::read_dir("../web/dist/assets") {
        for e in entries.flatten() {
            println!("cargo:rerun-if-changed={}", e.path().display());
        }
    }
}
