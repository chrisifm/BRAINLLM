//! `cargo run -p brain-check --example doctor -- <app root>`: prints the app-wide B10xx findings (§18).
fn main() {
    let root = std::env::args().nth(1).unwrap_or_else(|| ".".into());
    for d in brain_check::doctor::doctor(std::path::Path::new(&root)) {
        println!("{} {}:{}:{} {}", d.code, d.file, d.line, d.col, d.msg);
    }
}
