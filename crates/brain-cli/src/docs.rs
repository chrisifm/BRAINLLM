//! `brain docs [--json] [--out docs/app] [--check]`: deterministic Markdown documentation of an app, generated from its
//! source (routes, components, data, backend contracts, environment keys, SEO). Hand-written prose lives in
//! `<out>/notes/<page>.md` and is merged into the generated page, so the docs can be enriched without being overwritten.
//! Same source → same bytes: everything is sorted, no timestamps, no absolute paths, never an environment value.

use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};
use std::process::ExitCode;

pub const DEFAULT_OUT: &str = "docs/app";
/// (file stem, title, one-line summary) in link-table order.
const PAGES: [(&str, &str, &str); 7] = [
    ("index", "Overview", "app summary and the page index"),
    ("routes", "Routes", "every view: path, params, load, SEO metadata and guards"),
    ("components", "Components", "every comp/ and ui/ component: props, events, slots, stores and a usage example"),
    ("data", "Data", "stores with their readers and writers, server functions and channels"),
    ("backend", "Backend", "resources, jobs, roles, endpoints and every other backend contract key"),
    ("env", "Environment", "environment keys per environment (names only, never values)"),
    ("seo", "SEO", "indexable pages and the source of each sitemap entry"),
];

fn line_of(src: &str, offset: usize) -> usize {
    src.as_bytes()[..offset.min(src.len())].iter().filter(|b| **b == b'\n').count() + 1
}

fn cell(s: &str) -> String {
    s.replace('|', "\\|").replace('\n', " ")
}

fn code(s: &str) -> String {
    if s.is_empty() { String::new() } else { format!("`{}`", cell(s)) }
}

fn list(v: &Value) -> Vec<String> {
    v.as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_owned)).collect()).unwrap_or_default()
}

fn joined(items: &[String]) -> String {
    if items.is_empty() { "—".into() } else { items.iter().map(|s| code(s)).collect::<Vec<_>>().join(", ") }
}

/// `views/admin/users/[id].brain` → `views/_guard.jsbrain`, `views/admin/_guard.jsbrain` (outermost first).
fn guards(root: &Path, file: &str) -> Vec<String> {
    let mut out = vec![];
    let mut dir = PathBuf::new();
    let parts: Vec<&str> = file.split('/').collect();
    for part in &parts[..parts.len().saturating_sub(1)] {
        dir.push(part);
        let g = dir.join("_guard.jsbrain");
        if root.join(&g).is_file() {
            out.push(g.to_string_lossy().replace('\\', "/"));
        }
    }
    out
}

fn parse(root: &Path, rel: &str) -> Option<(String, brain_parse::ast::Component)> {
    let src = crate::map::read_source(root, rel).ok()?;
    let c = brain_parse::parse(&src, Path::new(rel)).ok()?;
    Some((src, c))
}

fn seo_of(c: &brain_parse::ast::Component) -> Value {
    let mut seo = serde_json::Map::new();
    for m in &c.meta {
        if matches!(m.name.as_str(), "title" | "keyword" | "description" | "robots" | "canonical" | "lastmod" | "titleTemplate") {
            let v = match (&m.value, &m.expr) {
                (_, Some(e)) => format!("{{{}}}", brain_parse::dump::expr(e)),
                (Some(v), None) => v.clone(),
                (None, None) => String::new(),
            };
            seo.insert(m.name.clone(), Value::String(v));
        }
    }
    Value::Object(seo)
}

/// `export async function name(a, b)` signatures of a BrainScript module, in source order.
fn signatures(src: &str) -> Vec<Value> {
    let mut out = vec![];
    for (i, line) in src.lines().enumerate() {
        let t = line.trim_start();
        let Some(rest) = t.strip_prefix("export ") else { continue };
        let rest = rest.strip_prefix("default ").unwrap_or(rest);
        let (is_async, rest) = match rest.strip_prefix("async ") {
            Some(r) => (true, r),
            None => (false, rest),
        };
        let Some(rest) = rest.strip_prefix("function ") else { continue };
        let Some(open) = rest.find('(') else { continue };
        let name = rest[..open].trim();
        let params = rest[open + 1..].split(')').next().unwrap_or("").trim();
        if !name.is_empty() {
            out.push(json!({"name": name, "params": params, "async": is_async, "line": i + 1}));
        }
    }
    out
}

/// Environment keys from the committed `.brain_env.example` (falls back to `.brain_env`): key → environments.
fn env_keys(root: &Path) -> (Option<&'static str>, BTreeMap<String, BTreeSet<String>>) {
    let mut keys: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for name in [crate::env::EXAMPLE, ".brain_env"] {
        let path = root.join(name);
        let regular = std::fs::symlink_metadata(&path).is_ok_and(|m| m.is_file() && m.len() <= 1 << 20);
        let Some(src) = regular.then(|| std::fs::read_to_string(&path).ok()).flatten() else { continue };
        let file = brain_check::env::parse(&src);
        let none = |_: &str| None;
        let shared: BTreeSet<String> = file.resolve("shared", &none).into_keys().collect();
        for e in std::iter::once("shared".to_string()).chain(file.environments()).collect::<BTreeSet<_>>() {
            for k in file.resolve(&e, &none).into_keys() {
                // a shared key applies everywhere: list it once as `shared`
                if e == "shared" || !shared.contains(&k) {
                    keys.entry(k).or_default().insert(e.clone());
                }
            }
        }
        return (Some(name), keys);
    }
    (None, keys)
}

/// The documentation model (also the `--json` output): plain data, sorted, without values of secrets.
pub fn model(root: &Path) -> Value {
    let map: Value = serde_json::from_str(&crate::map::build(root).stringify()).unwrap_or(Value::Null);
    let brain_json: Value = crate::map::read_source(root, "brain.json")
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(Value::Null);
    let name = brain_json["name"]
        .as_str()
        .map(str::to_owned)
        .or_else(|| root.canonicalize().ok()?.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "app".into());

    let mut routes = vec![];
    let mut route_list: Vec<&Value> = map["routes"].as_array().map(|a| a.iter().collect()).unwrap_or_default();
    route_list.sort_by(|a, b| a["path"].as_str().cmp(&b["path"].as_str()));
    for r in route_list {
        let file = r["file"].as_str().unwrap_or("");
        let (seo, paths, sitemap) = match parse(root, file) {
            Some((src, c)) => (seo_of(&c), src.contains("function paths"), src.contains("function sitemap")),
            None => (json!({}), false, false),
        };
        routes.push(json!({
            "path": r["path"], "file": file, "params": r["params"], "load": r["load"],
            "seo": seo, "guards": guards(root, file), "paths": paths, "sitemap": sitemap,
        }));
    }

    let mut components = vec![];
    let mut server_fns = vec![];
    let mut comp_list: Vec<&Value> = map["components"].as_array().map(|a| a.iter().collect()).unwrap_or_default();
    comp_list.sort_by(|a, b| a["file"].as_str().cmp(&b["file"].as_str()));
    for m in comp_list {
        let file = m["file"].as_str().unwrap_or("");
        let Some((src, c)) = parse(root, file) else { continue };
        for s in &c.script {
            if let brain_parse::ast::Stmt::Server { func, span } = s {
                let params: Vec<String> = func.params.iter().map(brain_parse::dump::pat).collect();
                server_fns.push(json!({
                    "name": func.name.as_ref().map(|n| n.name.clone()).unwrap_or_default(),
                    "params": params.join(", "), "async": func.is_async,
                    "file": file, "line": line_of(&src, span.start as usize),
                }));
            }
        }
        let level = m["level"].as_str().unwrap_or("comp");
        if level == "view" {
            continue;
        }
        let card: Value = serde_json::from_str(&brain_check::contract::card(&c, None).stringify()).unwrap_or(Value::Null);
        let line = c.props.first().map(|p| line_of(&src, p.span.start as usize)).unwrap_or(1);
        components.push(json!({
            "name": c.name, "file": file, "line": line, "level": level,
            "props": card["props"], "emits": card["emits"], "slots": card["slots"],
            "reads": card["reads"], "writes": card["writes"], "children": card["children"],
            "expose": m["expose"],
        }));
    }

    let mut stores: BTreeMap<String, (Vec<String>, Vec<String>)> = BTreeMap::new();
    for (side, key) in [(0, "reads"), (1, "writes")] {
        if let Some(o) = map["reverse"][key].as_object() {
            for (k, files) in o {
                let e = stores.entry(k.clone()).or_default();
                let mut f = list(files);
                f.sort();
                f.dedup();
                if side == 0 { e.0 = f } else { e.1 = f }
            }
        }
    }
    let stores: Vec<Value> = stores.into_iter().map(|(k, (r, w))| json!({"key": k, "readBy": r, "writtenBy": w})).collect();

    let mut modules = vec![];
    let mut mod_list: Vec<&Value> = map["modules"].as_array().map(|a| a.iter().collect()).unwrap_or_default();
    mod_list.sort_by(|a, b| a["file"].as_str().cmp(&b["file"].as_str()));
    for m in mod_list {
        let file = m["file"].as_str().unwrap_or("");
        if file.starts_with("views/") {
            continue; // route guards and helpers are documented with their routes
        }
        let kind = if file.starts_with("channels/") {
            "channel"
        } else if m["server"].as_bool() == Some(true) || file.starts_with("server/") {
            "server"
        } else {
            "module"
        };
        let sigs = crate::map::read_source(root, file).map(|s| signatures(&s)).unwrap_or_default();
        modules.push(json!({"file": file, "kind": kind, "exports": m["exports"], "functions": sigs}));
    }

    let backend = match crate::backend::inspect(root) {
        Ok(mut v) => {
            let mut endpoints = vec![];
            if let Some(paths) = v["openapi"]["paths"].as_object() {
                for (p, ops) in paths {
                    if let Some(ops) = ops.as_object() {
                        for (method, op) in ops.iter().filter(|(m, _)| matches!(m.as_str(), "get" | "post" | "put" | "patch" | "delete")) {
                            let summary = op["summary"].as_str().or(op["operationId"].as_str()).unwrap_or("");
                            endpoints.push(json!({"method": method.to_uppercase(), "path": p, "summary": summary}));
                        }
                    }
                }
            }
            if let Some(o) = v.as_object_mut() {
                for k in ["openapi", "schemaVersion"] {
                    o.remove(k);
                }
                o.insert("endpoints".into(), Value::Array(endpoints));
            }
            v
        }
        Err(e) => json!({"error": e}),
    };

    let (env_source, keys) = env_keys(root);
    let env: Vec<Value> = keys.into_iter().map(|(k, e)| json!({"key": k, "environments": e})).collect();

    let site = parse(root, "app.brain").map(|(_, c)| seo_of(&c)).unwrap_or(json!({}));
    let mut seo = vec![];
    for r in &routes {
        let path = r["path"].as_str().unwrap_or("");
        let robots = r["seo"]["robots"].as_str().unwrap_or("");
        let dynamic = !list(&r["params"]).is_empty();
        let file = r["file"].as_str().unwrap_or("");
        let private = file.rsplit('/').next().is_some_and(|f| f.starts_with('_'));
        let (indexable, source) = if !list(&r["guards"]).is_empty() {
            (false, "excluded: guarded")
        } else if robots.contains("noindex") {
            (false, "excluded: robots noindex")
        } else if private {
            (false, "excluded: underscore view")
        } else if !dynamic {
            (true, "static route")
        } else if r["paths"].as_bool() == Some(true) {
            (true, "paths() export (prerendered)")
        } else if r["sitemap"].as_bool() == Some(true) {
            (true, "sitemap() export (live)")
        } else {
            (false, "excluded: dynamic route without paths() or sitemap()")
        };
        seo.push(json!({"path": path, "indexable": indexable, "source": source, "keyword": r["seo"]["keyword"], "title": r["seo"]["title"]}));
    }

    json!({
        "schemaVersion": 1,
        "app": {"name": name, "seo": site, "settings": brain_json.get("seo").cloned().unwrap_or(Value::Null)},
        "routes": routes, "components": components,
        "data": {"stores": stores, "serverFunctions": server_fns, "modules": modules},
        "backend": backend,
        "env": {"source": env_source, "keys": env},
        "seo": seo,
    })
}

fn usage(c: &Value) -> String {
    let name = c["name"].as_str().unwrap_or("");
    let mut attrs = String::new();
    for p in c["props"].as_array().into_iter().flatten() {
        if p["optional"].as_bool() == Some(true) || p.get("default").is_some() {
            continue;
        }
        let n = p["name"].as_str().unwrap_or("");
        let v = match p["type"].as_str().unwrap_or("any") {
            "str" => "\"text\"".to_string(),
            "num" => "{0}".into(),
            "bool" => "{true}".into(),
            "list" => "{[]}".into(),
            "obj" => "{{}}".into(),
            "fn" => "{() => {}}".into(),
            _ => "{value}".into(),
        };
        if p["bindable"].as_bool() == Some(true) {
            attrs.push_str(&format!(" bind:{n}={{{n}}}"));
        } else {
            attrs.push_str(&format!(" {n}={v}"));
        }
    }
    for e in list(&c["emits"]) {
        attrs.push_str(&format!(" on:{e}={{handle}}"));
    }
    if list(&c["slots"]).is_empty() { format!("<{name}{attrs} />") } else { format!("<{name}{attrs}>…</{name}>") }
}

fn render_value(v: &Value) -> String {
    match v {
        Value::String(s) => code(s),
        Value::Null => "—".into(),
        Value::Array(a) if a.is_empty() => "—".into(),
        Value::Array(a) if a.iter().all(|x| x.is_string()) => joined(&list(v)),
        other => code(&other.to_string()),
    }
}

/// Generic rendering of a backend contract object: scalar keys as bullets, object-of-objects as tables.
fn render_object(out: &mut String, o: &serde_json::Map<String, Value>) {
    let mut keys: Vec<&String> = o.keys().filter(|k| k.as_str() != "name").collect();
    keys.sort();
    let mut tables = vec![];
    for k in keys {
        match &o[k] {
            Value::Object(inner) if !inner.is_empty() && inner.values().all(Value::is_object) => tables.push((k, inner)),
            v => out.push_str(&format!("- **{k}**: {}\n", render_value(v))),
        }
    }
    for (k, inner) in tables {
        out.push_str(&format!("\n**{k}**\n\n| Name | Definition |\n|---|---|\n"));
        let mut names: Vec<&String> = inner.keys().collect();
        names.sort();
        for n in names {
            out.push_str(&format!("| {} | {} |\n", code(n), code(&inner[n].to_string())));
        }
    }
}

fn page(out_rel: &str, stem: &str, title: &str, body: &str, notes: Option<&str>) -> String {
    let mut s = format!(
        "<!-- generated by brain docs; do not edit — add prose in {out_rel}/notes/{stem}.md -->\n# {title}\n\n{}",
        body.trim_end()
    );
    s.push('\n');
    if let Some(n) = notes.map(str::trim).filter(|n| !n.is_empty()) {
        s.push_str(&format!("\n## Notes\n\n{n}\n"));
    }
    s
}

fn count(n: usize, what: &str) -> String {
    format!("{n} {what}{}", if n == 1 { "" } else { "s" })
}

fn bodies(m: &Value) -> BTreeMap<&'static str, String> {
    let mut b = BTreeMap::new();
    let arr = |k: &Value| k.as_array().cloned().unwrap_or_default();

    let mut idx = format!(
        "App `{}`: {}, {}, {}, {}, {}.\n\n| Page | Contents |\n|---|---|\n",
        m["app"]["name"].as_str().unwrap_or(""),
        count(arr(&m["routes"]).len(), "route"),
        count(arr(&m["components"]).len(), "component"),
        count(arr(&m["data"]["stores"]).len(), "store key"),
        count(arr(&m["data"]["serverFunctions"]).len(), "server function"),
        count(arr(&m["env"]["keys"]).len(), "environment key"),
    );
    for (stem, title, summary) in PAGES.iter().skip(1) {
        idx.push_str(&format!("| [{title}]({stem}.md) | {summary} |\n"));
    }
    idx.push_str("\nRegenerate with `brain docs` (also run by `brain build`); `brain docs --check` fails when these pages are stale.\n");
    b.insert("index", idx);

    let mut r = String::from("| Path | File | Params | Load | Guards |\n|---|---|---|---|---|\n");
    for x in arr(&m["routes"]) {
        r.push_str(&format!(
            "| {} | {} | {} | {} | {} |\n",
            code(x["path"].as_str().unwrap_or("")),
            code(x["file"].as_str().unwrap_or("")),
            joined(&list(&x["params"])),
            if x["load"].as_bool() == Some(true) { "yes" } else { "no" },
            joined(&list(&x["guards"])),
        ));
    }
    for x in arr(&m["routes"]) {
        r.push_str(&format!("\n## {}\n\n- **file**: {}\n", code(x["path"].as_str().unwrap_or("")), code(x["file"].as_str().unwrap_or(""))));
        for k in ["title", "keyword", "description", "robots", "canonical"] {
            if let Some(v) = x["seo"][k].as_str() {
                r.push_str(&format!("- **{k}**: {}\n", code(v)));
            }
        }
        if x["paths"].as_bool() == Some(true) {
            r.push_str("- **pages**: listed by `paths()`\n");
        }
        if x["sitemap"].as_bool() == Some(true) {
            r.push_str("- **sitemap**: `sitemap()` export\n");
        }
    }
    if arr(&m["routes"]).is_empty() {
        r = "No views yet: add one with `brain add view <name>`.\n".into();
    }
    b.insert("routes", r);

    let mut c = String::new();
    for x in arr(&m["components"]) {
        let name = x["name"].as_str().unwrap_or("");
        c.push_str(&format!(
            "## {name}\n\n- **file**: `{}:{}` ({})\n",
            x["file"].as_str().unwrap_or(""),
            x["line"],
            x["level"].as_str().unwrap_or("")
        ));
        let props = arr(&x["props"]);
        if props.is_empty() {
            c.push_str("- **props**: —\n");
        } else {
            c.push_str("\n| Prop | Type | Required | Default |\n|---|---|---|---|\n");
            for p in props {
                let required = p["optional"].as_bool() != Some(true) && p.get("default").is_none();
                c.push_str(&format!(
                    "| {} | {}{} | {} | {} |\n",
                    code(p["name"].as_str().unwrap_or("")),
                    code(p["type"].as_str().unwrap_or("")),
                    if p["bindable"].as_bool() == Some(true) { " (bindable)" } else { "" },
                    if required { "yes" } else { "no" },
                    p["default"].as_str().map(code).unwrap_or_else(|| "—".into()),
                ));
            }
            c.push('\n');
        }
        for (k, label) in [("emits", "events"), ("slots", "slots"), ("reads", "reads"), ("writes", "writes"), ("children", "uses"), ("expose", "exposes")] {
            c.push_str(&format!("- **{label}**: {}\n", joined(&list(&x[k]))));
        }
        c.push_str(&format!("\n```brain\n{}\n```\n\n", usage(&x)));
    }
    if c.is_empty() {
        c = "No components yet: add one with `brain add comp <name>`.\n".into();
    }
    b.insert("components", c);

    let mut d = String::from("## Stores\n\n");
    let stores = arr(&m["data"]["stores"]);
    if stores.is_empty() {
        d.push_str("No shared state is read or written.\n");
    } else {
        d.push_str("| Key | Read by | Written by |\n|---|---|---|\n");
        for s in stores {
            d.push_str(&format!(
                "| {} | {} | {} |\n",
                code(s["key"].as_str().unwrap_or("")),
                joined(&list(&s["readBy"])),
                joined(&list(&s["writtenBy"]))
            ));
        }
    }
    d.push_str("\n## Server functions\n\n");
    let fns = arr(&m["data"]["serverFunctions"]);
    if fns.is_empty() {
        d.push_str("None.\n");
    } else {
        d.push_str("| Function | Signature | File |\n|---|---|---|\n");
        for f in fns {
            let name = f["name"].as_str().unwrap_or("");
            let sig = format!("{}{name}({})", if f["async"].as_bool() == Some(true) { "async " } else { "" }, f["params"].as_str().unwrap_or(""));
            d.push_str(&format!("| {} | {} | `{}:{}` |\n", code(name), code(&sig), f["file"].as_str().unwrap_or(""), f["line"]));
        }
    }
    for (kind, title) in [("channel", "Channels"), ("server", "Server modules"), ("module", "Modules")] {
        d.push_str(&format!("\n## {title}\n\n"));
        let ms: Vec<Value> = arr(&m["data"]["modules"]).into_iter().filter(|x| x["kind"] == kind).collect();
        if ms.is_empty() {
            d.push_str("None.\n");
            continue;
        }
        for x in ms {
            d.push_str(&format!("- {} — exports {}\n", code(x["file"].as_str().unwrap_or("")), joined(&list(&x["exports"]))));
            for f in arr(&x["functions"]) {
                d.push_str(&format!(
                    "  - `{}{}({})` (line {})\n",
                    if f["async"].as_bool() == Some(true) { "async " } else { "" },
                    f["name"].as_str().unwrap_or(""),
                    cell(f["params"].as_str().unwrap_or("")),
                    f["line"]
                ));
            }
        }
    }
    b.insert("data", d);

    let mut be = String::new();
    match m["backend"].as_object() {
        Some(o) if o.contains_key("error") => {
            be.push_str(&format!("The backend contracts do not validate: {}. Run `brain backend schema`.\n", code(o["error"].as_str().unwrap_or(""))))
        }
        Some(o) => {
            let mut keys: Vec<&String> = o.keys().collect();
            keys.sort();
            for k in keys {
                be.push_str(&format!("## {k}\n\n"));
                match &o[k] {
                    Value::Array(a) if a.is_empty() => be.push_str("None.\n\n"),
                    Value::Array(a) if k == "endpoints" => {
                        be.push_str("| Method | Path | Summary |\n|---|---|---|\n");
                        for e in a {
                            be.push_str(&format!(
                                "| {} | {} | {} |\n",
                                e["method"].as_str().unwrap_or(""),
                                code(e["path"].as_str().unwrap_or("")),
                                cell(e["summary"].as_str().unwrap_or(""))
                            ));
                        }
                        be.push('\n');
                    }
                    Value::Array(a) => {
                        for (i, item) in a.iter().enumerate() {
                            match item.as_object() {
                                Some(io) => {
                                    let title = io.get("name").and_then(Value::as_str).map(str::to_owned).unwrap_or_else(|| format!("{k} {}", i + 1));
                                    be.push_str(&format!("### {title}\n\n"));
                                    render_object(&mut be, io);
                                    be.push('\n');
                                }
                                None => be.push_str(&format!("- {}\n", render_value(item))),
                            }
                        }
                        be.push('\n');
                    }
                    Value::Object(io) => {
                        render_object(&mut be, io);
                        be.push('\n');
                    }
                    v => be.push_str(&format!("{}\n\n", render_value(v))),
                }
            }
        }
        None => be.push_str("No backend.\n"),
    }
    b.insert("backend", be);

    let mut e = String::new();
    let keys = arr(&m["env"]["keys"]);
    match m["env"]["source"].as_str() {
        Some(src) if !keys.is_empty() => {
            e.push_str(&format!("Keys from {} (values are never documented).\n\n| Key | Environments |\n|---|---|\n", code(src)));
            for k in keys {
                e.push_str(&format!("| {} | {} |\n", code(k["key"].as_str().unwrap_or("")), joined(&list(&k["environments"]))));
            }
        }
        _ => e.push_str("No environment keys: add one with `brain env set <KEY> <value>`.\n"),
    }
    b.insert("env", e);

    let mut s = String::new();
    if let Some(o) = m["app"]["seo"].as_object().filter(|o| !o.is_empty()) {
        s.push_str("Site defaults in `app.brain`:\n\n");
        let mut ks: Vec<&String> = o.keys().collect();
        ks.sort();
        for k in ks {
            s.push_str(&format!("- **{k}**: {}\n", render_value(&o[k])));
        }
        s.push('\n');
    }
    if !m["app"]["settings"].is_null() {
        s.push_str(&format!("`brain.json` seo settings: {}\n\n", code(&m["app"]["settings"].to_string())));
    }
    s.push_str("| Path | Indexable | Sitemap source | Keyword |\n|---|---|---|---|\n");
    for x in arr(&m["seo"]) {
        s.push_str(&format!(
            "| {} | {} | {} | {} |\n",
            code(x["path"].as_str().unwrap_or("")),
            if x["indexable"].as_bool() == Some(true) { "yes" } else { "no" },
            x["source"].as_str().unwrap_or(""),
            x["keyword"].as_str().map(code).unwrap_or_else(|| "—".into()),
        ));
    }
    s.push_str("\n`brain build` writes robots.txt, sitemap.xml and llms.txt from these routes (`brain guide seo`).\n");
    b.insert("seo", s);
    b
}

/// Rejects absolute paths and `..`: the output stays inside the app.
pub fn out_dir(arg: Option<&str>) -> Result<String, String> {
    let out = arg.unwrap_or(DEFAULT_OUT).trim_end_matches('/').to_string();
    let p = Path::new(&out);
    if out.is_empty() || p.is_absolute() || p.components().any(|c| !matches!(c, Component::Normal(_))) {
        return Err(format!("--out must be a relative folder inside the app (got `{out}`); fix: use --out {DEFAULT_OUT}"));
    }
    Ok(out)
}

/// (file name, contents) of every page, notes merged.
pub fn pages(root: &Path, out: &str, m: &Value) -> Vec<(String, String)> {
    let mut b = bodies(m);
    PAGES
        .iter()
        .map(|(stem, title, _)| {
            let notes = std::fs::read_to_string(root.join(out).join("notes").join(format!("{stem}.md"))).ok();
            (format!("{stem}.md"), page(out, stem, title, &b.remove(stem).unwrap_or_default(), notes.as_deref()))
        })
        .collect()
}

/// Pages whose committed copy is missing or differs from the source.
pub fn stale(root: &Path, out: &str) -> Vec<String> {
    let m = model(root);
    pages(root, out, &m)
        .into_iter()
        .filter(|(f, body)| std::fs::read_to_string(root.join(out).join(f)).ok().as_deref() != Some(body.as_str()))
        .map(|(f, _)| format!("{out}/{f}"))
        .collect()
}

/// Writes the pages that changed; returns (model, updated files).
pub fn write(root: &Path, out: &str) -> Result<(Value, Vec<String>), String> {
    let m = model(root);
    let dir = root.join(out);
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {out}: {e}"))?;
    let mut updated = vec![];
    for (f, body) in pages(root, out, &m) {
        let path = dir.join(&f);
        if std::fs::read_to_string(&path).ok().as_deref() != Some(body.as_str()) {
            std::fs::write(&path, body).map_err(|e| format!("cannot write {out}/{f}: {e}"))?;
            updated.push(format!("{out}/{f}"));
        }
    }
    Ok((m, updated))
}

/// Docs are kept up to date by `brain build` when `<out>/index.md` exists or brain.json has `"docs": true`.
pub fn enabled(root: &Path) -> bool {
    root.join(DEFAULT_OUT).join("index.md").is_file()
        || crate::map::read_source(root, "brain.json")
            .ok()
            .and_then(|s| serde_json::from_str::<Value>(&s).ok())
            .is_some_and(|j| j["docs"] == Value::Bool(true))
}

pub fn cmd(args: &[String]) -> ExitCode {
    let json_out = args.iter().any(|a| a == "--json");
    let fail = |e: String| {
        if json_out { println!("{}", crate::error_json(&e)) } else { eprintln!("error: {e}") }
        ExitCode::from(2)
    };
    if let Some(bad) = args.iter().find(|a| a.starts_with("--") && !matches!(a.as_str(), "--json" | "--check" | "--out") && !a.starts_with("--out=")) {
        return fail(format!("unknown option `{bad}`; fix: brain docs [--json] [--out {DEFAULT_OUT}] [--check]"));
    }
    let root = match crate::app_root() {
        Ok(r) => r,
        Err(e) => return fail(e),
    };
    let out = match out_dir(crate::flag(args, "--out").as_deref()) {
        Ok(o) => o,
        Err(e) => return fail(e),
    };
    if args.iter().any(|a| a == "--check") {
        let s = stale(&root, &out);
        if json_out {
            println!("{}", json!({"schemaVersion": 1, "ok": s.is_empty(), "out": out, "stale": s}));
        } else if s.is_empty() {
            println!("{out}: up to date");
        } else {
            println!("{out}: {} stale page(s): {}\n  fix: run `brain docs` and commit {out}", s.len(), s.join(", "));
        }
        return if s.is_empty() { ExitCode::SUCCESS } else { ExitCode::from(1) };
    }
    match write(&root, &out) {
        Ok((m, updated)) => {
            let files: Vec<String> = PAGES.iter().map(|(s, _, _)| format!("{out}/{s}.md")).collect();
            if json_out {
                println!("{}", json!({"schemaVersion": 1, "out": out, "pages": files, "updated": updated, "model": m}));
            } else {
                println!("{out}: {} pages, {} updated (read {out}/index.md; prose goes in {out}/notes/<page>.md)", files.len(), updated.len());
            }
            ExitCode::SUCCESS
        }
        Err(e) => fail(e),
    }
}
