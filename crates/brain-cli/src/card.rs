//! Large apps by components (INTERNALS §43): `brain card <comp>` returns only a component's contract (props, emits,
//! slots, what it reads and writes, children, size, check status), and `brain split <comp> "<selector>" <Name>`
//! extracts a subtree into a new component. A split that does not pass `brain check` is reverted and refused.

use brain_mcp::Json;
use std::path::{Path, PathBuf};

/// A component by file (`comp/cart.brain`) or by name (`Cart`, `cart`), as `brain map` lists it.
pub fn resolve(root: &Path, comp: &str) -> Result<String, String> {
    let as_file = comp.trim_start_matches("./");
    if as_file.ends_with(".brain") && root.join(as_file).is_file() {
        return Ok(as_file.to_string());
    }
    let m = crate::map::build(root);
    let comps = match m.get("components") {
        Some(Json::Arr(a)) => a.clone(),
        _ => vec![],
    };
    let file = |c: &Json| c.get("file").and_then(Json::as_str).unwrap_or("").to_string();
    let stem = |f: &str| Path::new(f).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let hits: Vec<String> = comps
        .iter()
        .filter(|c| c.get("name").and_then(Json::as_str).is_some_and(|n| n.eq_ignore_ascii_case(comp)) || stem(&file(c)).eq_ignore_ascii_case(comp))
        .map(file)
        .collect();
    match hits.as_slice() {
        [one] => Ok(one.clone()),
        [] => Err(format!("no component `{comp}`; fix: run `brain map` and use a name or file it lists")),
        many => Err(format!("`{comp}` matches {}; fix: pass the file", many.join(", "))),
    }
}

fn parse(root: &Path, rel: &str) -> Result<brain_parse::ast::Component, String> {
    let src = std::fs::read_to_string(root.join(rel)).map_err(|e| format!("{rel}: {e}"))?;
    brain_parse::parse(&src, Path::new(rel)).map_err(|d| format!("{rel} does not parse ({}); fix: run `brain check {rel}`", d.iter().map(|x| format!("{} {}", x.code, x.msg)).collect::<Vec<_>>().join("; ")))
}

/// (errors, warnings) of `brain check` on one file.
fn status(root: &Path, rel: &str) -> Result<(usize, usize, Vec<crate::compiler::Diag>), String> {
    let (_, diags, _) = crate::run_check_base(Some(root.join(rel)))?;
    let w = diags.iter().filter(|d| d.is_warning()).count();
    Ok((diags.len() - w, w, diags))
}

/// The card of `comp`; with `suggest` when the component is over the size limit (B1010).
pub fn card(root: &Path, comp: &str) -> Result<Json, String> {
    let rel = resolve(root, comp)?;
    let c = parse(root, &rel)?;
    let (e, w, _) = status(root, &rel)?;
    let mut j = brain_check::contract::card(&c, Some((e, w)));
    let (lines, bytes) = brain_check::contract::size(&c);
    let over = lines > brain_check::split::MAX_LINES || bytes > brain_check::split::MAX_BYTES;
    if let (Json::Obj(kv), true, Some(s)) = (&mut j, over, brain_check::split::suggest(&c)) {
        let v = match s {
            brain_check::split::Suggestion::Split { selector, name, line } => {
                Json::obj([("split", Json::str(format!("brain split {rel} \"{selector}\" {name}"))), ("line", Json::Num(line as f64))])
            }
            brain_check::split::Suggestion::Repeated { line, count } => Json::obj([("repeated", Json::Num(count as f64)), ("line", Json::Num(line as f64))]),
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
pub fn split(root: &Path, comp: &str, selector: &str, name: &str, before_write: &mut dyn FnMut(&Path, Option<&str>)) -> Result<Json, String> {
    let rel = resolve(root, comp)?;
    let c = parse(root, &rel)?;
    let dir = if rel.starts_with("ui/") { "ui" } else { "comp" };
    let new_rel = format!("{dir}/{}.brain", kebab(name));
    let new_path: PathBuf = root.join(&new_rel);
    if new_path.exists() {
        return Err(format!("{new_rel} already exists; fix: pick another name"));
    }
    let s = brain_check::split::split(&c, selector, name, &new_rel)?;
    let orig_path = root.join(&rel);
    let before = std::fs::read_to_string(&orig_path).map_err(|e| e.to_string())?;
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
        return Err(format!("the split does not pass `brain check`, so nothing changed: {}", crate::compiler::Diag::list_to_json(&errors)));
    }
    Ok(Json::obj([
        ("original", Json::str(&rel)),
        ("created", Json::str(&new_rel)),
        ("props", Json::Arr(s.props.iter().map(Json::str).collect())),
        ("emits", Json::Arr(s.emits.iter().map(Json::str).collect())),
    ]))
}
