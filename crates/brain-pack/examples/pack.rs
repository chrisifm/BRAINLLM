//! `cargo run -p brain-pack --example pack -- <app> <out> [--no-min]`: bundles an app without the CLI (for manual
//! checks and size measurements).
use std::path::{Path, PathBuf};

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let (root, out) = (PathBuf::from(&a[1]).canonicalize().unwrap(), PathBuf::from(&a[2]));
    let minify = !a.iter().any(|x| x == "--no-min");
    let rt = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../runtime");
    let core = std::fs::read_to_string(rt.join("brain.js")).unwrap();
    let core = if minify { brain_min::minify_runtime_core(&core).unwrap() } else { core };
    let siblings: Vec<(String, String)> = ["brain-wc.js", "brain-form.js", "brain-ch.js", "brain-data.js", "brain-err.js"]
        .iter()
        .filter_map(|n| std::fs::read_to_string(rt.join(n)).ok().map(|s| (n.to_string(), s)))
        .collect();
    let r2 = root.clone();
    let compile = move |src: &str, rel: &Path| -> Result<brain_pack::Unit, String> {
        let opts = brain_codegen::Options { dev: false, root: Some(r2.clone()), ..Default::default() };
        let o = brain_codegen::compile_with(src, rel, &opts).map_err(|d| format!("{}: {}", rel.display(), d.iter().map(|x| x.msg.clone()).collect::<Vec<_>>().join("; ")))?;
        Ok(brain_pack::Unit { js: o.js, css: o.css, server_js: o.server_js, server_fns: o.server_fns })
    };
    let input = brain_pack::Input {
        root: root.clone(),
        compile: &compile,
        runtime_core: core,
        runtime_siblings: siblings,
        layer_order: format!("<style>{}</style>", brain_css::LAYER_ORDER),
        minify,
        site: None,
        shim: std::fs::read_to_string(rt.join("shim/dom.js")).ok(),
        generator: "BRAINLLM".into(),
        server_module: None,
        cache: Default::default(),
        server_mjs: None,
        progress: None,
        post: None,
    };
    let mut views = Vec::new();
    for e in walk(&root.join("views")) {
        let rel = e.strip_prefix(&root).unwrap().to_string_lossy().replace('\\', "/");
        if rel.contains("/_") || rel.ends_with(".test.brain") {
            continue;
        }
        let route = rel.trim_start_matches("views/").trim_end_matches(".brain").trim_end_matches("/index");
        let route = if route == "home" || route == "index" { "/".to_string() } else { format!("/{route}") };
        views.push((route, rel));
    }
    let o = brain_pack::build(&input, &views, &|_| vec![], &|_| vec![]).unwrap_or_else(|e| panic!("{e}"));
    for (p, c) in &o.files {
        let f = out.join(p);
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(&f, c).unwrap();
        println!("{:>7} B  {p}", c.len());
    }
    for w in o.warnings {
        eprintln!("warning: {w}");
    }
}

fn walk(d: &Path) -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Ok(rd) = std::fs::read_dir(d) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() { v.extend(walk(&p)) } else if p.extension().is_some_and(|x| x == "brain") { v.push(p) }
        }
    }
    v.sort();
    v
}
