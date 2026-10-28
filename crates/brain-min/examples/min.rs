//! `cargo run -p brain-min --example min -- <in.js> <out.js> [--mangle] [--drop-log]`: minifies one file by hand.
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let src = std::fs::read_to_string(&a[1]).expect("read");
    let o = brain_min::Options { mangle: a.iter().any(|x| x == "--mangle"), drop_log: a.iter().any(|x| x == "--drop-log"), ..Default::default() };
    let out = brain_min::minify(&src, &o).unwrap_or_else(|e| panic!("{}: {e}", a[1]));
    std::fs::write(&a[2], out).expect("write");
}
