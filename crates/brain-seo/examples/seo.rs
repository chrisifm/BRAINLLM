//! `cargo run -p brain-seo --example seo -- <app root> [audit|links] [--json]`: the `brain seo` command before it is
//! wired into the CLI. `… <app root> finish [--site https://…]` applies the build step to an existing `dist/` (what
//! `brain build` will call between page assembly and precompression).

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let root = args
        .first()
        .map_or_else(|| std::path::PathBuf::from("."), std::path::PathBuf::from);
    let rest = args.get(1..).unwrap_or(&[]);
    if rest.first().is_some_and(|a| a == "finish") {
        if let Err(e) = finish(&root, rest) {
            eprintln!("{e}");
            std::process::exit(1);
        }
        return;
    }
    let (code, out) = brain_seo::cli(&root, rest);
    println!("{out}");
    std::process::exit(code);
}

fn finish(root: &std::path::Path, args: &[String]) -> Result<(), String> {
    let mut cfg = brain_seo::Config::load(root)?;
    if let Some(i) = args.iter().position(|a| a == "--site") {
        cfg.site = Some(brain_seo::config::origin(
            args.get(i + 1).ok_or("--site needs a value")?,
        )?);
    }
    let dist = root.join("dist");
    let (mut pages, assets) = brain_seo::read_dist(&dist)?;
    let built = brain_seo::finish(&cfg, root, &mut pages, &[], &assets)?;
    for p in &pages {
        std::fs::write(dist.join(&p.file), &p.html).map_err(|e| e.to_string())?;
    }
    for (path, content) in &built.files {
        std::fs::create_dir_all(dist.join(path).parent().unwrap()).map_err(|e| e.to_string())?;
        std::fs::write(dist.join(path), content).map_err(|e| e.to_string())?;
        println!("+ dist/{path}");
    }
    for w in &built.warnings {
        println!("warning: {w}");
    }
    print!("{}", built.report.text());
    Ok(())
}
