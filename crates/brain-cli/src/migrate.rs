//! `brain upgrade [--from <v>] [--to <v>] [--dry-run|--apply] [--json] [--no-doctor]`: reads the change manifests
//! (`releases.rs`) between the version the app's source was last migrated to and the installed compiler, runs the safe
//! codemods (`--apply`), runs the gates and returns a migration report an LLM agent can follow step by step: codemods,
//! manual steps with `file:line`, the capabilities the app does not use yet with the snippet that starts each, and the
//! gates. `--dry-run` (the default) writes nothing. `brain upgrade <exact-version>` keeps its package-pin preview.
use crate::releases::{self, Entry, Kind};
use serde_json::{Value, json};
use std::cmp::Ordering;
use std::fs;
use std::path::Path;

const PACKAGE: &str = "@christianirack/brainllm";

/// The compiler running this command.
pub fn installed() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Default target: the installed compiler; a prerelease build (`0.5.1-dev.0`) carries the `[Unreleased]` entries, so
/// it targets the newest manifest version above it.
pub fn default_target() -> String {
    let me = installed();
    if me.contains('-')
        && let Some(latest) = releases::latest().filter(|l| releases::cmp(l, me) == Ordering::Greater)
    {
        return latest;
    }
    me.to_string()
}

fn read_json(path: &Path) -> Option<Value> {
    let meta = fs::symlink_metadata(path).ok()?;
    if !meta.is_file() || meta.len() > 8 * 1024 * 1024 {
        return None;
    }
    serde_json::from_str(&fs::read_to_string(path).ok()?).ok()
}

fn exact(v: &str) -> Option<String> {
    let v = v.trim().trim_start_matches(['^', '~', '=', 'v']);
    crate::upgrade::parse_version(v).ok().map(|_| v.to_string())
}

/// The version the app's source targets and where it was read: brain.json `brainVersion` (written by the scaffold and
/// by `brain upgrade --apply`), else the package.json pin, else the lockfile.
pub fn recorded(root: &Path) -> Option<(String, &'static str)> {
    if let Some(v) = read_json(&root.join("brain.json")).and_then(|j| j.get("brainVersion").and_then(Value::as_str).and_then(exact)) {
        return Some((v, "brain.json brainVersion"));
    }
    if let Some(pkg) = read_json(&root.join("package.json")) {
        for s in ["dependencies", "devDependencies", "optionalDependencies"] {
            if let Some(v) = pkg.get(s).and_then(|d| d.get(PACKAGE)).and_then(Value::as_str).and_then(exact) {
                return Some((v, "package.json pin"));
            }
        }
    }
    read_json(&root.join("package-lock.json"))
        .and_then(|l| l.pointer(&format!("/packages/node_modules~1{}", PACKAGE.replace('/', "~1"))).and_then(|p| p.get("version")).and_then(Value::as_str).and_then(exact))
        .map(|v| (v, "package-lock.json"))
}

pub struct Options {
    pub from: Option<String>,
    pub to: Option<String>,
    pub apply: bool,
    pub doctor: bool,
}

/// `brain.json` text with `"brainVersion": to`, keeping every other byte: replace the value in place, or insert the key
/// first (on its own line with the file's indentation, or inline for a one-line object).
pub fn with_brain_version(src: &str, to: &str) -> Result<String, String> {
    let parsed: Value = serde_json::from_str(src).map_err(|_| "brain.json is invalid JSON")?;
    if !parsed.is_object() {
        return Err("brain.json must contain an object".into());
    }
    let out = if parsed.get("brainVersion").is_some() {
        let at = src.find("\"brainVersion\"").ok_or("brain.json brainVersion must be written as a plain key")?;
        let colon = src[at..].find(':').map(|c| at + c).ok_or("brain.json brainVersion has no value")?;
        let open = colon + 1 + src[colon + 1..].len() - src[colon + 1..].trim_start().len();
        if !src[open..].starts_with('"') {
            return Err("brain.json brainVersion must be a string; fix: set it to the exact version the app targets".into());
        }
        let close = src[open + 1..].find('"').map(|q| open + 1 + q).ok_or("brain.json brainVersion must be a string")?;
        format!("{}\"{to}\"{}", &src[..open], &src[close + 1..])
    } else {
        let brace = src.find('{').ok_or("brain.json must contain an object")?;
        let (head, rest) = (&src[..=brace], &src[brace + 1..]);
        let body = rest.trim_start();
        if body.starts_with('}') {
            format!("{head}\"brainVersion\": \"{to}\"{body}")
        } else if rest.starts_with(['\n', '\r']) {
            let first = rest.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
            let indent = &first[..first.len() - first.trim_start().len()];
            let nl = if rest.starts_with("\r\n") { "\r\n" } else { "\n" };
            format!("{head}{nl}{indent}\"brainVersion\": \"{to}\",{rest}")
        } else {
            format!("{head}\"brainVersion\": \"{to}\", {rest}")
        }
    };
    let check: Value = serde_json::from_str(&out).map_err(|_| "could not write brainVersion into brain.json")?;
    if check.get("brainVersion").and_then(Value::as_str) != Some(to) {
        return Err("could not write brainVersion into brain.json".into());
    }
    Ok(out)
}

fn loc(file: &str, line: usize) -> Value {
    if line == 0 { json!({"file": file}) } else { json!({"file": file, "line": line}) }
}

/// The migration report (and, with `apply`, the codemods it ran).
pub fn run(root: &Path, opts: &Options) -> Result<Value, String> {
    run_with(root, opts, &mut |_, _| {})
}

/// [`run`] with a hook called before each file write with the path and its previous content (MCP journals it so
/// `brain_undo` can revert an applied upgrade).
pub fn run_with(root: &Path, opts: &Options, before_write: &mut dyn FnMut(&Path, &str)) -> Result<Value, String> {
    let root = root.canonicalize().map_err(|_| "app root is unavailable")?;
    let (from, from_source) = match &opts.from {
        Some(f) => (exact(f).ok_or("--from must be an exact semantic version")?, "--from"),
        None => recorded(&root).ok_or("cannot tell which version this app targets: no brainVersion in brain.json, no @christianirack/brainllm pin in package.json and no lockfile entry; fix: pass --from <version>")?,
    };
    let to = match &opts.to {
        Some(t) => exact(t).ok_or("--to must be an exact semantic version")?,
        None => default_target(),
    };
    if releases::cmp(&to, &from) == Ordering::Less {
        return Err(format!("target {to} is older than the app's version {from}; a downgrade needs a separate reviewed plan"));
    }
    let span = releases::between(&from, &to)?;
    let entries: Vec<(&str, &Entry)> = span.iter().flat_map(|r| r.entries.iter().map(move |e| (r.version.as_str(), e))).collect();
    let files = releases::app_files(&root);

    // ---- codemods ----
    let mut codemods = vec![];
    let fix_codes: Vec<&str> = entries.iter().filter(|(_, e)| e.codemod.as_deref() == Some("brain-fix")).flat_map(|(_, e)| e.codes.iter().map(String::as_str)).collect();
    let pin = crate::upgrade::preview(&root, &to).ok();
    let pin_changes = pin.as_ref().and_then(|p| p["operations"].as_array().cloned()).unwrap_or_default();
    let mut pin_entry = json!({"id": "package-pin", "description": releases::CODEMODS[0].1, "files": pin_changes.iter().map(|o| o["path"].clone()).collect::<Vec<_>>()});
    pin_entry["status"] = json!(if pin.is_none() { "skipped" } else if pin_changes.is_empty() { "up-to-date" } else if opts.apply { "applied" } else { "planned" });
    if opts.apply {
        for op in &pin_changes {
            let path = root.join(op["path"].as_str().unwrap_or("package.json"));
            let now = fs::read(&path).map_err(|e| e.to_string())?;
            if crate::manifests::hash(&now) != op["expectedRevision"].as_str().unwrap_or("") {
                return Err("package.json changed while the upgrade ran; fix: run brain upgrade again".into());
            }
            before_write(&path, &String::from_utf8_lossy(&now));
            fs::write(&path, op["content"].as_str().unwrap_or("")).map_err(|e| e.to_string())?;
        }
    }
    if pin.is_some() && !pin_changes.is_empty() {
        pin_entry["next"] = json!("install the new pin with the app's package manager (for example `npm install`) and commit the lockfile");
    }
    codemods.push(pin_entry);

    let bj = root.join("brain.json");
    let record = match fs::read_to_string(&bj) {
        Ok(src) => {
            let next = with_brain_version(&src, &to)?;
            let status = if next == src { "up-to-date" } else if opts.apply { before_write(&bj, &src); fs::write(&bj, &next).map_err(|e| e.to_string())?; "applied" } else { "planned" };
            json!({"id": "record-version", "description": releases::CODEMODS[1].1, "status": status, "files": ["brain.json"]})
        }
        Err(_) => json!({"id": "record-version", "description": releases::CODEMODS[1].1, "status": "skipped", "reason": "no brain.json"}),
    };
    codemods.push(record);

    let mut fixed = json!(null);
    if !fix_codes.is_empty() {
        if opts.apply {
            fixed = crate::run_fix_only(Some(root.clone()), Some(&fix_codes), before_write)
                .map(|j| Value::from_str_lossy(&j.stringify()))
                .unwrap_or_else(|e| json!({"error": e}));
        }
        codemods.push(json!({"id": "brain-fix", "description": releases::CODEMODS[2].1, "codes": fix_codes, "status": if opts.apply { "applied" } else { "planned" }, "result": fixed}));
    }

    // ---- check gate (after codemods when applying) ----
    let check = crate::run_check(Some(root.clone()));
    let diags = check.as_ref().map(|(_, d)| d.clone()).unwrap_or_default();
    let errors = diags.iter().filter(|d| !d.is_warning()).count();

    // ---- manual steps, adoption, other changes ----
    let (mut manual, mut adopt, mut using, mut changes) = (vec![], vec![], vec![], vec![]);
    for (version, e) in &entries {
        let mut locations: Vec<Value> = e.finds.iter().flat_map(|p| releases::probe(&root, &files, p)).map(|(f, l)| loc(&f, l)).collect();
        for d in diags.iter().filter(|d| e.codes.iter().any(|c| c == d.code)) {
            locations.push(json!({"file": d.file, "line": d.line, "code": d.code, "msg": d.msg, "fix": d.fix}));
        }
        locations.dedup();
        match e.kind {
            Kind::Capability => {
                let hits: Vec<Value> = e.uses.iter().flat_map(|p| releases::probe(&root, &files, p)).take(3).map(|(f, l)| loc(&f, l)).collect();
                if hits.is_empty() {
                    let a = e.adopt.as_ref();
                    adopt.push(json!({"entry": e.id, "version": version, "title": e.title, "why": e.why, "file": a.map(|a| &a.file), "command": a.and_then(|a| a.command.as_ref()), "snippet": a.map(|a| &a.snippet), "guide": e.guide, "apis": e.apis}));
                } else {
                    using.push(json!({"entry": e.id, "version": version, "title": e.title, "at": hits}));
                }
            }
            Kind::Breaking | Kind::Deprecation => {
                if !locations.is_empty() {
                    manual.push(json!({"entry": e.id, "version": version, "kind": e.kind, "title": e.title, "why": e.why, "steps": e.steps, "guide": e.guide, "locations": locations}));
                } else {
                    changes.push(json!({"entry": e.id, "version": version, "kind": e.kind, "title": e.title, "status": "not found in this app"}));
                }
            }
            Kind::Change if !e.steps.is_empty() && !locations.is_empty() => {
                manual.push(json!({"entry": e.id, "version": version, "kind": e.kind, "title": e.title, "why": e.why, "steps": e.steps, "guide": e.guide, "locations": locations}));
            }
            Kind::Internal => {}
            _ => changes.push(json!({"entry": e.id, "version": version, "kind": e.kind, "title": e.title, "why": e.why, "steps": e.steps})),
        }
    }

    // ---- gates ----
    let mut gates = vec![match &check {
        Ok((n, _)) => json!({"name": "check", "command": "brain check --json", "status": "ran", "ok": errors == 0, "files": n, "errors": errors, "warnings": diags.len() - errors}),
        Err(e) => json!({"name": "check", "command": "brain check --json", "status": "failed-to-run", "ok": false, "error": e}),
    }];
    if opts.apply && opts.doctor && errors == 0 {
        match crate::run_doctor(Some(root.clone())) {
            Ok((ok, report)) => {
                let r = Value::from_str_lossy(&report.stringify());
                gates.push(json!({"name": "doctor", "command": "brain doctor", "status": "ran", "ok": ok, "tests": r["test"], "build": {"ok": r["build"]["ok"], "skipped": r["build"]["skipped"]}, "seo": {"skipped": r["seo"]["skipped"]}}));
            }
            Err(e) => gates.push(json!({"name": "doctor", "command": "brain doctor", "status": "failed-to-run", "ok": false, "error": e})),
        }
    } else {
        gates.push(json!({"name": "doctor", "command": "brain doctor", "status": "pending", "reason": if opts.apply { if errors > 0 { "check has errors; fix them first" } else { "--no-doctor" } } else { "dry run: doctor builds and runs the tests; run it after --apply" }}));
    }
    let ok = gates.iter().all(|g| g["ok"] != json!(false)) && manual.is_empty();
    let mut next = vec![];
    if !opts.apply && codemods.iter().any(|c| c["status"] == "planned") {
        next.push("Run `brain upgrade --apply` to run the planned codemods.".to_string());
    }
    if codemods.iter().any(|c| c.get("next").is_some()) {
        next.push("Install the new pin (`npm install`) and commit the lockfile.".into());
    }
    if !manual.is_empty() {
        next.push(format!("Work through the {} manual step group(s) below, one location at a time; re-run `brain check --json` after each.", manual.len()));
    }
    if !adopt.is_empty() {
        next.push("Adopt the new capabilities that fit this app: read each guide, start from the snippet, add a test.".into());
    }
    next.push("Finish with `brain doctor` exit 0, then `brain build`.".into());
    Ok(json!({
        "schemaVersion": 1, "mode": if opts.apply { "apply" } else { "dry-run" },
        "from": from, "fromSource": from_source, "to": to, "installed": installed(),
        "releases": span.iter().map(|r| json!({"version": r.version, "entries": r.entries.len()})).collect::<Vec<_>>(),
        "codemods": codemods, "manual": manual, "adopt": adopt, "alreadyUsing": using, "changes": changes, "gates": gates,
        "ok": ok, "next": next,
        "packagePin": pin.map(|p| json!({"operations": p["operations"].as_array().map_or(0, Vec::len), "requiresHumanDecisions": p["requiresHumanDecisions"]})),
    }))
}

trait FromStrLossy {
    fn from_str_lossy(s: &str) -> Value;
}
impl FromStrLossy for Value {
    fn from_str_lossy(s: &str) -> Value {
        serde_json::from_str(s).unwrap_or(Value::Null)
    }
}

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or("")
}

/// The report as a markdown checklist an agent follows top to bottom.
pub fn markdown(r: &Value) -> String {
    let mut m = format!("# BRAINLLM upgrade {} → {} ({})\n\n", s(&r["from"]), s(&r["to"]), s(&r["mode"]));
    m.push_str(&format!("Version source: {}. Installed compiler: {}.\n\n## Next\n", s(&r["fromSource"]), s(&r["installed"])));
    for n in r["next"].as_array().into_iter().flatten() {
        m.push_str(&format!("- [ ] {}\n", s(n)));
    }
    m.push_str("\n## Codemods\n");
    for c in r["codemods"].as_array().into_iter().flatten() {
        let mark = if matches!(s(&c["status"]), "applied" | "up-to-date") { "x" } else { " " };
        m.push_str(&format!("- [{mark}] `{}` ({}): {}\n", s(&c["id"]), s(&c["status"]), s(&c["description"])));
    }
    m.push_str("\n## Manual steps\n");
    let manual = r["manual"].as_array().cloned().unwrap_or_default();
    if manual.is_empty() {
        m.push_str("None found in this app.\n");
    }
    for g in &manual {
        m.push_str(&format!("### {} ({} {})\n{}\n", s(&g["title"]), s(&g["kind"]), s(&g["version"]), s(&g["why"])));
        for st in g["steps"].as_array().into_iter().flatten() {
            m.push_str(&format!("- [ ] {}\n", s(st)));
        }
        for l in g["locations"].as_array().into_iter().flatten() {
            let line = l["line"].as_u64().map(|n| format!(":{n}")).unwrap_or_default();
            let code = l["code"].as_str().map(|c| format!(" {c}: {}", s(&l["msg"]))).unwrap_or_default();
            m.push_str(&format!("  - [ ] `{}{line}`{code}\n", s(&l["file"])));
        }
        if let Some(g) = g["guide"].as_str() {
            m.push_str(&format!("  Read: {g}\n"));
        }
    }
    m.push_str("\n## Adopt (new capabilities this app does not use yet)\n");
    for a in r["adopt"].as_array().into_iter().flatten() {
        m.push_str(&format!("### {} ({})\n{}\nRead: {}\n", s(&a["title"]), s(&a["version"]), s(&a["why"]), s(&a["guide"])));
        if let Some(c) = a["command"].as_str() {
            m.push_str(&format!("Start with: `{c}`\n"));
        }
        m.push_str(&format!("In `{}`:\n```\n{}\n```\n", s(&a["file"]), s(&a["snippet"])));
    }
    let using = r["alreadyUsing"].as_array().map_or(0, Vec::len);
    if using > 0 {
        m.push_str(&format!("\nAlready used by this app: {using} capability(ies) (see `alreadyUsing` in --json).\n"));
    }
    m.push_str("\n## Other changes\n");
    for c in r["changes"].as_array().into_iter().flatten() {
        m.push_str(&format!("- {} ({} {}){}\n", s(&c["title"]), s(&c["kind"]), s(&c["version"]), c["status"].as_str().map(|x| format!(": {x}")).unwrap_or_default()));
    }
    m.push_str("\n## Gates\n");
    for g in r["gates"].as_array().into_iter().flatten() {
        let mark = if g["ok"] == json!(true) { "x" } else { " " };
        m.push_str(&format!("- [{mark}] `{}` ({})\n", s(&g["command"]), s(&g["status"])));
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn app(name: &str, version: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("brain-migrate-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("views")).unwrap();
        fs::write(d.join("package.json"), format!("{{\"devDependencies\":{{\"{PACKAGE}\":\"{version}\"}}}}")).unwrap();
        fs::write(d.join("brain.json"), "{\n  \"name\": \"Shop\",\n  \"docs\": true\n}\n").unwrap();
        fs::write(d.join("views/_layout.brain"), "<script>\n  export async function load() { return {} }\n</script>\n<slot />\n").unwrap();
        fs::write(d.join("views/index.brain"), "<p>{post.owner}</p>\n").unwrap();
        d
    }

    #[test]
    fn brain_version_is_inserted_or_replaced_without_touching_other_bytes() {
        // inserted first, with the file's indentation; every other byte kept
        let src = "{\n  \"$schema\": \"x\",\n  \"name\": \"Shop\"\n}\n";
        let out = with_brain_version(src, "0.6.0").unwrap();
        assert_eq!(out, "{\n  \"brainVersion\": \"0.6.0\",\n  \"$schema\": \"x\",\n  \"name\": \"Shop\"\n}\n");
        // replaced in place: only the value changes, spacing around the colon included
        assert_eq!(with_brain_version(&out, "0.7.0").unwrap(), out.replace("0.6.0", "0.7.0"));
        let spaced = "{\n\t\"name\": \"Shop\",\n\t\"brainVersion\" :  \"0.5.0\",\n\t\"docs\": true\n}";
        assert_eq!(with_brain_version(spaced, "0.6.1").unwrap(), spaced.replace("0.5.0", "0.6.1"));
        // the same version is a no-op
        assert_eq!(with_brain_version(&out, "0.6.0").unwrap(), out);
        // tabs, CRLF and one-line objects keep their style
        let tabs = "{\n\t\"name\": \"Shop\"\n}";
        assert_eq!(with_brain_version(tabs, "0.6.0").unwrap(), "{\n\t\"brainVersion\": \"0.6.0\",\n\t\"name\": \"Shop\"\n}");
        let crlf = "{\r\n  \"name\": \"Shop\"\r\n}\r\n";
        assert_eq!(with_brain_version(crlf, "0.6.0").unwrap(), "{\r\n  \"brainVersion\": \"0.6.0\",\r\n  \"name\": \"Shop\"\r\n}\r\n");
        assert_eq!(with_brain_version("{\"name\":\"Shop\"}", "0.6.0").unwrap(), "{\"brainVersion\": \"0.6.0\", \"name\":\"Shop\"}");
        assert_eq!(with_brain_version("{}", "0.6.0").unwrap(), "{\"brainVersion\": \"0.6.0\"}");
        assert_eq!(with_brain_version("{\n}\n", "0.6.0").unwrap(), "{\"brainVersion\": \"0.6.0\"}\n");
        // a nested key with the same name is not the top-level one
        let nested = "{\n  \"x\": { \"brainVersion\": \"0.1.0\" }\n}";
        let v: Value = serde_json::from_str(&with_brain_version(nested, "0.6.0").unwrap()).unwrap();
        assert_eq!((v["brainVersion"].as_str(), v["x"]["brainVersion"].as_str()), (Some("0.6.0"), Some("0.1.0")));
        // a nested one written before the top-level key is never rewritten in its place: refused, not corrupted
        assert!(with_brain_version("{\"x\": {\"brainVersion\": \"0.1.0\"}, \"brainVersion\": \"0.5.0\"}", "0.6.0").is_err());
        // refused: not an object, invalid JSON, a non-string value
        assert!(with_brain_version("[]", "0.6.0").is_err());
        assert!(with_brain_version("{", "0.6.0").is_err());
        assert!(with_brain_version("{\"brainVersion\": 5}", "0.6.0").unwrap_err().contains("must be a string"));
    }

    #[test]
    fn recorded_version_prefers_brain_json_then_the_pin() {
        let d = app("recorded", "^0.5.0");
        assert_eq!(recorded(&d).unwrap(), ("0.5.0".to_string(), "package.json pin"));
        fs::write(d.join("brain.json"), "{\"brainVersion\":\"0.4.0\"}").unwrap();
        assert_eq!(recorded(&d).unwrap().0, "0.4.0");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn dry_run_lists_manual_steps_with_file_line_and_adoptable_capabilities_and_writes_nothing() {
        let d = app("dry", "0.5.0");
        let before = fs::read_to_string(d.join("brain.json")).unwrap();
        let r = run(&d, &Options { from: None, to: Some("0.6.0".into()), apply: false, doctor: false }).unwrap();
        assert_eq!(fs::read_to_string(d.join("brain.json")).unwrap(), before);
        assert_eq!((r["from"].as_str(), r["to"].as_str(), r["mode"].as_str()), (Some("0.5.0"), Some("0.6.0"), Some("dry-run")));
        let manual = r["manual"].as_array().unwrap();
        let layout = manual.iter().find(|g| g["entry"] == "layout-load-b0117").expect("layout load step");
        assert!(layout["locations"].as_array().unwrap().iter().any(|l| l["file"] == "views/_layout.brain" && l["line"] == 2), "{layout}");
        assert!(manual.iter().any(|g| g["entry"] == "backend-owner-hidden"));
        let adopt: Vec<&str> = r["adopt"].as_array().unwrap().iter().map(|a| a["entry"].as_str().unwrap()).collect();
        assert!(adopt.contains(&"webmcp-tools") && adopt.contains(&"file-buckets"), "{adopt:?}");
        assert!(!adopt.contains(&"app-docs"), "brain.json has docs: true, so it is already used");
        assert!(r["codemods"].as_array().unwrap().iter().any(|c| c["id"] == "record-version" && c["status"] == "planned"));
        let md = markdown(&r);
        assert!(md.contains("`views/_layout.brain:2`") && md.contains("## Adopt") && md.contains("- [ ] "), "{md}");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn apply_runs_safe_codemods_and_records_the_version() {
        let d = app("apply", "0.5.0");
        let r = run(&d, &Options { from: None, to: Some("0.6.0".into()), apply: true, doctor: false }).unwrap();
        assert_eq!(r["mode"], "apply");
        let pkg: Value = serde_json::from_str(&fs::read_to_string(d.join("package.json")).unwrap()).unwrap();
        assert_eq!(pkg["devDependencies"][PACKAGE], "0.6.0");
        assert_eq!(recorded(&d).unwrap(), ("0.6.0".to_string(), "brain.json brainVersion"));
        assert_eq!(r["gates"][0]["name"], "check");
        let again = run(&d, &Options { from: None, to: Some("0.6.0".into()), apply: false, doctor: false }).unwrap();
        assert_eq!(again["releases"], json!([]), "nothing left between 0.6.0 and 0.6.0");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn downgrades_and_unknown_versions_are_refused() {
        let d = app("down", "0.6.0");
        assert!(run(&d, &Options { from: None, to: Some("0.5.0".into()), apply: false, doctor: false }).unwrap_err().contains("older"));
        assert!(run(&d, &Options { from: Some("latest".into()), to: None, apply: false, doctor: false }).is_err());
        let _ = fs::remove_dir_all(&d);
    }
}
