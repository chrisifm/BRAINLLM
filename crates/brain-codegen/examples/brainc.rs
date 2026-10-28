//! `cargo run -p brain-codegen --example brainc -- <root> <rel.brain> [--dev] [--suffix=.js] [--runtime=<specifier>] [--out=<dir>]`
//! prints the JS and CSS, or writes `<out>/<rel>.js` and `<out>/<rel>.css`.
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let root = std::path::PathBuf::from(&a[1]);
    let rel = std::path::PathBuf::from(&a[2]);
    let src = std::fs::read_to_string(root.join(&rel)).unwrap();
    let flag = |k: &str| a.iter().find_map(|x| x.strip_prefix(k).map(str::to_string));
    let opts = brain_codegen::Options {
        dev: a.iter().any(|x| x == "--dev"),
        root: Some(root),
        import_suffix: flag("--suffix=").unwrap_or_default(),
        runtime: flag("--runtime=").unwrap_or_else(|| brain_codegen::Options::default().runtime),
        ..Default::default()
    };
    match brain_codegen::compile_with(&src, &rel, &opts) {
        Ok(o) => match flag("--out=") {
            Some(out) => {
                let js = std::path::Path::new(&out).join(format!("{}.js", rel.display()));
                std::fs::create_dir_all(js.parent().unwrap()).unwrap();
                std::fs::write(&js, o.js).unwrap();
                std::fs::write(js.with_extension("css"), o.css).unwrap();
            }
            None => print!("{}\n/* css */\n{}", o.js, o.css),
        },
        Err(d) => {
            for x in d {
                eprintln!("{x}");
            }
            std::process::exit(1);
        }
    }
}
