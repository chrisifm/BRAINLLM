//! `cargo run --release -p brain-pack --example gz -- <files…>`: own gzip (lazy and optimal parse) vs file sizes.
fn main() {
    for f in std::env::args().skip(1) {
        let d = std::fs::read(&f).unwrap();
        let t = std::time::Instant::now();
        let lazy = brain_pack::gzip::gzip(&d, 0).len();
        let t0 = t.elapsed().as_secs_f64() * 1000.0;
        let t = std::time::Instant::now();
        let opt = brain_pack::gzip::gzip(&d, 15).len();
        let t1 = t.elapsed().as_secs_f64() * 1000.0;
        println!("{}\t{}\t{lazy}\t{opt}\t{t0:.1}ms\t{t1:.1}ms", f.rsplit('/').next().unwrap(), d.len());
    }
}
