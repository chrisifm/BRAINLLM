//! Large apps by components (INTERNALS §43): `brain card <comp>` returns only a component's contract (props, emits,
//! slots, what it reads and writes, children, size, check status), and `brain split <comp> "<selector>" <Name>`
//! extracts a subtree into a new component. A split that does not pass `brain check` is reverted and refused.

use brain_mcp::Json;
use std::path::{Path, PathBuf};

/// A component by file (`comp/cart.brain`) or by name (`Cart`, `cart`), as `brain map` lists it.
pub fn resolve(root: &Path, comp: &str) -> Result<String, String> {
    if comp.len() > 240 {
        return Err("component name/path exceeds 240 bytes".into());
    }
    let as_file = comp.strip_prefix("./").unwrap_or(comp);
    if as_file.ends_with(".brain") || comp.contains('/') || comp.contains('\\') || Path::new(comp).is_absolute() {
        let path = crate::changes::source_path(root, as_file)?;
        if !as_file.ends_with(".brain") || !std::fs::symlink_metadata(&path).is_ok_and(|m| m.is_file() && !m.file_type().is_symlink()) {
            return Err("card requires an existing app-relative .brain component file".into());
        }
        crate::map::read_source(root, as_file)?;
        return Ok(as_file.to_owned());
    }
    let m = crate::map::build(root);
    let comps = match m.get("components") {
        Some(Json::Arr(a)) => a.clone(),
        _ => vec![],
    };
    let file = |c: &Json| c.get("file").and_then(Json::as_str).unwrap_or("").to_string();
    let stem = |f: &str| {
        Path::new(f)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default()
    };
    let hits: Vec<String> = comps
        .iter()
        .filter(|c| {
            c.get("name").and_then(Json::as_str).is_some_and(|n| n.eq_ignore_ascii_case(comp)) || stem(&file(c)).eq_ignore_ascii_case(comp)
        })
        .map(file)
        .collect();
    match hits.as_slice() {
        [one] => {
            crate::changes::source_path(root, one)?;
            Ok(one.clone())
        }
        [] => Err(format!(
            "no component `{comp}`; fix: run `brain map` and use a name or file it lists"
        )),
        many => Err(format!("`{comp}` matches {}; fix: pass the file", many.join(", "))),
    }
}

fn parse(root: &Path, rel: &str) -> Result<brain_parse::ast::Component, String> {
    let src = crate::map::read_source(root, rel).map_err(|e| format!("{rel}: {e}"))?;
    brain_parse::parse(&src, Path::new(rel)).map_err(|d| {
        format!(
            "{rel} does not parse ({}); fix: run `brain check {rel}`",
            d.iter().map(|x| format!("{} {}", x.code, x.msg)).collect::<Vec<_>>().join("; ")
        )
    })
}

/// (errors, warnings) of `brain check` on one file.
fn status(root: &Path, rel: &str) -> Result<(usize, usize, Vec<crate::compiler::Diag>), String> {
    let target = crate::changes::source_path(root, rel)?;
    crate::map::read_source(root, rel)?;
    let (_, diags, _) = crate::run_check_base(Some(target))?;
    let w = diags.iter().filter(|d| d.is_warning()).count();
    Ok((diags.len() - w, w, diags))
}

/// The card of `comp`; with `suggest` when the component is over the size limit (B1010).
pub fn card(root: &Path, comp: &str) -> Result<Json, String> {
    if comp.starts_with("resource:") || comp.starts_with("job:") {
        return crate::backend::card(root, Some(comp)).and_then(|j| Json::parse(&j.to_string()));
    }
    let rel = resolve(root, comp)?;
    let c = parse(root, &rel)?;
    let discovery = crate::map::build(root);
    let complete = discovery.get("coverage").and_then(|c| c.get("checkerContextSafe")) == Some(&Json::Bool(true));
    // Status loads checker context. An incomplete source inventory cannot safely
    // establish that its dependency readers stay within the same app bounds.
    let mut safe_imports = true;
    for s in &c.script {
        if let brain_parse::ast::Stmt::Import(i) = s {
            if !i.source.starts_with("brain:") {
                let base = Path::new(&rel).parent().unwrap_or(Path::new(""));
                let mut parts = vec![];
                for part in base.join(&i.source).components() {
                    match part {
                        std::path::Component::Normal(p) => parts.push(p.to_string_lossy().into_owned()),
                        std::path::Component::CurDir => {}
                        std::path::Component::ParentDir => {
                            if parts.pop().is_none() {
                                safe_imports = false;
                            }
                        }
                        _ => safe_imports = false,
                    }
                }
                safe_imports &= crate::map::read_source(root, &parts.join("/")).is_ok();
            }
        }
    }
    let checked = if complete && safe_imports {
        let (e, w, _) = status(root, &rel)?;
        Some((e, w))
    } else {
        None
    };
    let mut j = brain_check::contract::card(&c, checked);
    if checked.is_none() {
        if let Json::Obj(kv) = &mut j {
            kv.push((
                "status".into(),
                Json::obj([
                    ("state", Json::str("unknown")),
                    ("reason", Json::str("Source discovery or dependency boundaries are incomplete")),
                ]),
            ));
        }
    }
    if let (Json::Obj(kv), Some(coverage)) = (&mut j, discovery.get("coverage")) {
        kv.push(("coverage".into(), coverage.clone()));
    }
    let (lines, bytes) = brain_check::contract::size(&c);
    let over = lines > brain_check::split::MAX_LINES || bytes > brain_check::split::MAX_BYTES;
    if let (Json::Obj(kv), true, Some(s)) = (&mut j, over, brain_check::split::suggest(&c)) {
        let v = match s {
            brain_check::split::Suggestion::Split { selector, name, line } => Json::obj([
                ("split", Json::str(format!("brain split {rel} \"{selector}\" {name}"))),
                ("line", Json::Num(line as f64)),
            ]),
            brain_check::split::Suggestion::Repeated { line, count } => {
                Json::obj([("repeated", Json::Num(count as f64)), ("line", Json::Num(line as f64))])
            }
        };
        kv.push(("suggest".into(), v));
    }
    Ok(j)
}

/// `TodoRow` → `todo-row`.
fn kebab(name: &str) -> String {
    let mut o = String::new();
    for (i, ch) in name.chars().enumerate() {
        if ch.is_ascii_uppercase() {
            if i > 0 {
                o.push('-');
            }
            o.push(ch.to_ascii_lowercase());
        } else {
            o.push(ch);
        }
    }
    o
}

/// Extracts the subtree `selector` of `comp` into `<dir>/<kebab name>.brain` (ui/ components stay in ui/, the rest go
/// to comp/), writes both files and checks them; any error reverts both and returns the diagnostics.
/// `before_write(path, previous)` lets the MCP journal record each write for `brain_undo`.
pub fn split(
    root: &Path,
    comp: &str,
    selector: &str,
    name: &str,
    before_write: &mut dyn FnMut(&Path, Option<&str>),
) -> Result<Json, String> {
    let _writer = crate::edit::writer_lock(root)?;
    let rel = resolve(root, comp)?;
    let c = parse(root, &rel)?;
    let dir = if rel.starts_with("ui/") { "ui" } else { "comp" };
    let new_rel = format!("{dir}/{}.brain", kebab(name));
    let new_path: PathBuf = crate::changes::source_path(root, &new_rel)?;
    if new_path.exists() {
        return Err(format!("{new_rel} already exists; fix: pick another name"));
    }
    let s = brain_check::split::split(&c, selector, name, &new_rel)?;
    let orig_path = root.join(&rel);
    let before = crate::map::read_source(root, &rel)?;
    before_write(&orig_path, Some(&before));
    before_write(&new_path, None);
    std::fs::create_dir_all(new_path.parent().unwrap()).map_err(|e| e.to_string())?;
    std::fs::write(&new_path, &s.extracted).map_err(|e| e.to_string())?;
    std::fs::write(&orig_path, &s.original).map_err(|e| e.to_string())?;
    let mut errors = Vec::new();
    for f in [&rel, &new_rel] {
        let (_, _, diags) = status(root, f)?;
        errors.extend(diags.into_iter().filter(|d| !d.is_warning()));
    }
    if !errors.is_empty() {
        // refused: both files back as they were
        let _ = std::fs::write(&orig_path, &before);
        let _ = std::fs::remove_file(&new_path);
        return Err(format!(
            "the split does not pass `brain check`, so nothing changed: {}",
            crate::compiler::Diag::list_to_json(&errors)
        ));
    }
    Ok(Json::obj([
        ("original", Json::str(&rel)),
        ("created", Json::str(&new_rel)),
        ("props", Json::Arr(s.props.iter().map(Json::str).collect())),
        ("emits", Json::Arr(s.emits.iter().map(Json::str).collect())),
    ]))
}

#[cfg(test)]
mod boundary_tests {
    use super::*;
    struct App(PathBuf);
    impl App {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let dir = std::env::temp_dir().join(format!(
                "brain-card-bounds-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            std::fs::create_dir_all(dir.join("comp")).unwrap();
            std::fs::write(dir.join("comp/counter.brain"), "<p>Counter</p>").unwrap();
            Self(dir)
        }
    }
    impl Drop for App {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn component_names_and_relative_files_work_but_outside_and_traversal_do_not() {
        let app = App::new();
        let outside = App::new();
        assert_eq!(resolve(&app.0, "counter").unwrap(), "comp/counter.brain");
        assert_eq!(resolve(&app.0, "Counter").unwrap(), "comp/counter.brain");
        assert_eq!(resolve(&app.0, "./comp/counter.brain").unwrap(), "comp/counter.brain");
        assert!(resolve(&app.0, outside.0.join("comp/counter.brain").to_str().unwrap()).is_err());
        assert!(resolve(&app.0, "../comp/counter.brain").is_err());
        assert!(resolve(&app.0, "comp/../../outside.brain").is_err());
        assert!(resolve(&app.0, "comp\\counter.brain").is_err());
    }
    #[test]
    fn card_large_files_and_symlink_sources_fail_without_reading_targets() {
        let app = App::new();
        let outside = App::new();
        std::fs::write(app.0.join("comp/large.brain"), vec![b'x'; 2 * 1024 * 1024 + 1]).unwrap();
        assert!(resolve(&app.0, "comp/large.brain").unwrap_err().contains("2 MiB"));
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(outside.0.join("comp/counter.brain"), app.0.join("comp/alias.brain")).unwrap();
            assert!(resolve(&app.0, "comp/alias.brain").is_err());
            assert!(parse(&app.0, "comp/alias.brain").is_err());
            std::os::unix::fs::symlink(&app.0, app.0.join("loop")).unwrap();
            let j = card(&app.0, "counter").unwrap();
            assert_eq!(j.get("status").unwrap().get("state").and_then(Json::as_str), Some("unknown"));
        }
    }
    #[test]
    fn active_lease_blocks_legacy_split_before_resolve_or_mutation() {
        let app = App::new();
        let source = std::fs::read(app.0.join("comp/counter.brain")).unwrap();
        crate::changes::lease(
            &app.0,
            &serde_json::json!({"owner":"split-agent","paths":["comp/counter.brain"],"ttlSeconds":30}),
        )
        .unwrap();
        let mut journal_calls = 0;
        let error = split(&app.0, "counter", "p", "CounterLine", &mut |_, _| {
            journal_calls += 1;
        })
        .unwrap_err();
        assert!(error.contains("active file leases"), "{error}");
        assert_eq!(journal_calls, 0);
        assert_eq!(std::fs::read(app.0.join("comp/counter.brain")).unwrap(), source);
        assert!(!app.0.join("comp/counter-line.brain").exists());
    }
}
