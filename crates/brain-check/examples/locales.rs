//! `cargo run -p brain-check --example locales -- <app root>`: the app-wide language findings (§37).
fn main() {
    let root = std::env::args().nth(1).unwrap_or_else(|| ".".into());
    match brain_check::i18n::load(std::path::Path::new(&root)) {
        Some(i) => {
            println!("default={} locales={:?} missing={:?}", i.default, i.locales, i.missing);
            for d in brain_check::i18n::check_locales(&i) {
                println!("{} {}:{} {}", d.code, d.file, d.line, d.msg);
            }
        }
        None => println!("no i18n"),
    }
}
