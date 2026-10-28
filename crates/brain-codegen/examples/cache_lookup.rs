//! Reproducible microbenchmark of persistent lookup, not a whole-build performance claim.
use brain_codegen::{blocks::{blocks, cache_key, source_cache_key}, Options};
use std::{hint::black_box, path::Path, time::Instant};

fn main() {
    let src = format!("<script>let n = sig(0)</script><section>{}</section>", "<p>{n}</p>".repeat(100));
    let path = Path::new("comp/bench.brain");
    let opts = Options::default();
    let iterations = 10_000;
    let start = Instant::now();
    for _ in 0..iterations { black_box(cache_key(&blocks(black_box(&src), path), path, &opts)); }
    let parsed = start.elapsed().as_secs_f64();
    let start = Instant::now();
    for _ in 0..iterations { black_box(source_cache_key(black_box(&src), path, &opts)); }
    let direct = start.elapsed().as_secs_f64();
    println!("{{\"iterations\":{iterations},\"bytes\":{},\"parsedSeconds\":{parsed},\"directSeconds\":{direct},\"speedup\":{}}}", src.len(), parsed / direct);
}
