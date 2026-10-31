// Embeds in the binary everything served under `/@brain/*`: the `runtime/*.js` modules (brain.js, brain-dev.js,
// brain-wc.js, brain-form.js…) and the `runtime/ui/*.brain` kit. That way `brain dev`/`brain test` work outside the
// repo. Without runtime/brain.js a fallback mount is used.
use std::fmt::Write;
use std::path::Path;

fn main() {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let rt = Path::new(&manifest).join("../../runtime");
    println!("cargo:rerun-if-changed={}", rt.display());
    println!("cargo:rerun-if-changed={}", rt.join("ui").display());
    println!("cargo:rerun-if-changed={}", Path::new(&manifest).join("../../SPEC.md").display());
    let out_dir = Path::new(&std::env::var("OUT_DIR").unwrap()).to_path_buf();
    check_planned(&Path::new(&manifest).join("../../SPEC.md"));
    let mut files: Vec<(String, std::path::PathBuf)> = Vec::new();
    // shim/dom.js: the DOM shim used for build-time prerender (never served to browsers)
    // sw/brain-sw.js: service worker template (§35); server/server.mjs: production server copied into dist (#51)
    println!("cargo:rerun-if-changed={}", rt.join("sw").display());
    println!("cargo:rerun-if-changed={}", rt.join("server").display());
    println!("cargo:rerun-if-changed={}", rt.join("themes").display());
    for (dir, prefix, ext) in [
        (rt.clone(), "", "js"),
        (rt.join("ui"), "ui/", "brain"),
        (rt.join("shim"), "shim/", "js"),
        (rt.join("sw"), "sw/", "js"),
        (rt.join("server"), "server/", "mjs"),
        // theme presets for `brain theme <preset>` (§27)
        (rt.join("themes"), "themes/", "cssbrain"),
    ] {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_file() && p.extension().is_some_and(|x| x == ext) {
                println!("cargo:rerun-if-changed={}", p.display());
                files.push((format!("{prefix}{}", p.file_name().unwrap().to_string_lossy()), p));
            }
        }
    }
    files.sort();
    if !files.iter().any(|(n, _)| n == "brain.js") {
        let fb = Path::new(&manifest).join("src/templates/runtime-fallback.js");
        files.push(("brain.js".into(), fb));
    }
    let mut code = String::from("pub const RUNTIME: &[(&str, &str)] = &[\n");
    for (name, src) in &files {
        let dst = out_dir.join(format!("rt_{}", name.replace('/', "__")));
        std::fs::copy(src, &dst).unwrap();
        writeln!(code, "    ({name:?}, include_str!({:?})),", dst.display().to_string()).unwrap();
    }
    code.push_str("];\n");
    // the debugger app (§9.1, devtools/): its sources, served by `brain run` on :P+1 (no dist/, no mock/)
    let dt = Path::new(&manifest).join("../../devtools");
    println!("cargo:rerun-if-changed={}", dt.display());
    let mut dfiles: Vec<(String, std::path::PathBuf)> = Vec::new();
    fn walk(base: &Path, dir: &Path, out: &mut Vec<(String, std::path::PathBuf)>) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            let name = p.file_name().unwrap().to_string_lossy().to_string();
            if name.starts_with('.') || name == "dist" || name == "mock" || name == "node_modules" {
                continue;
            }
            if p.is_dir() {
                println!("cargo:rerun-if-changed={}", p.display());
                walk(base, &p, out);
            } else if p.extension().is_some_and(|x| x == "brain" || x == "jsbrain" || x == "cssbrain" || x == "json") {
                println!("cargo:rerun-if-changed={}", p.display());
                out.push((p.strip_prefix(base).unwrap().to_string_lossy().replace('\\', "/"), p));
            }
        }
    }
    walk(&dt, &dt, &mut dfiles);
    dfiles.sort();
    code.push_str("pub const DEVTOOLS: &[(&str, &str)] = &[\n");
    for (name, src) in &dfiles {
        let dst = out_dir.join(format!("dt_{}", name.replace('/', "__")));
        std::fs::copy(src, &dst).unwrap();
        writeln!(code, "    ({name:?}, include_str!({:?})),", dst.display().to_string()).unwrap();
    }
    code.push_str("];\n");
    std::fs::write(out_dir.join("runtime_files.rs"), code).unwrap();
    embed_releases(&Path::new(&manifest).join("releases"), &out_dir);
}

/// Versioned change manifests (`releases/<version>/<entry>.json`, see `src/releases.rs`): every file is embedded,
/// so adding an entry is one new file and the npm package ships them inside the binary.
fn embed_releases(root: &Path, out_dir: &Path) {
    println!("cargo:rerun-if-changed={}", root.display());
    let mut files = vec![];
    for v in std::fs::read_dir(root).into_iter().flatten().flatten().filter(|e| e.path().is_dir()) {
        println!("cargo:rerun-if-changed={}", v.path().display());
        for f in std::fs::read_dir(v.path()).into_iter().flatten().flatten() {
            let p = f.path();
            if p.extension().is_some_and(|x| x == "json") {
                println!("cargo:rerun-if-changed={}", p.display());
                files.push((v.file_name().to_string_lossy().into_owned(), f.file_name().to_string_lossy().into_owned(), p));
            }
        }
    }
    files.sort();
    let mut code = String::from("pub(crate) const RELEASE_FILES: &[(&str, &str, &str)] = &[\n");
    for (version, name, path) in files {
        writeln!(code, "    ({version:?}, {name:?}, include_str!({:?})),", path.display().to_string()).unwrap();
    }
    code.push_str("];\n");
    std::fs::write(out_dir.join("releases.rs"), code).unwrap();
}

/// Fails the build if SPEC.md has a `<!--planned-->` marker that is not alone on its line, an unclosed block, an
/// orphan or nested close: `brain spec` would serve half the spec without warning (it happened on 2026-10-13: Sonnet 0/20).
/// Same rule as `spec::strip_planned`.
fn check_planned(spec: &Path) {
    let Ok(src) = std::fs::read_to_string(spec) else { return };
    let mut open: Option<usize> = None;
    for (n, line) in src.lines().enumerate() {
        let n = n + 1;
        let t = line.trim();
        let (o, c) = (t.contains("<!--planned"), t.contains("<!--/planned-->"));
        if !o && !c {
            continue;
        }
        let alone = (o && !c && t.starts_with("<!--planned") && t.ends_with("-->") && t.matches("-->").count() == 1) || (c && !o && t == "<!--/planned-->");
        assert!(alone, "SPEC.md:{n}: the planned marker must be alone on its line: `{t}`");
        if o {
            assert!(open.is_none(), "SPEC.md:{n}: nested planned block (opened on line {})", open.unwrap_or(0));
            open = Some(n);
        } else {
            assert!(open.take().is_some(), "SPEC.md:{n}: `<!--/planned-->` without an opening");
        }
    }
    assert!(open.is_none(), "SPEC.md:{}: planned block without `<!--/planned-->`", open.unwrap_or(0));
}
