//! `brain mcp`: full project control over MCP (INTERNALS §24). Six groups of tools — project, modules, quality,
//! data, run/debug and build — plus `brain://` resources, guided prompts and notifications while the managed dev
//! server is alive. Destructive tools require `confirm: true`; there is no publish tool (publishing is Chris's call).

use crate::{diags_json, run_check, scaffold, spec};
use brain_mcp::{Json, Notifier, Prompt, Resource, Server, Tool};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

fn schema<const N: usize>(props: [(&str, &str, &str); N], required: &[&str]) -> Json {
    Json::obj([
        ("type", Json::str("object")),
        (
            "properties",
            Json::Obj(
                props
                    .iter()
                    .map(|(k, ty, desc)| (k.to_string(), Json::obj([("type", Json::str(*ty)), ("description", Json::str(*desc))])))
                    .collect(),
            ),
        ),
        ("required", Json::Arr(required.iter().map(|r| Json::str(*r)).collect())),
        ("additionalProperties", Json::Bool(false)),
    ])
}

/// Output schema: an object with these top-level fields (name, JSON type).
fn out<const N: usize>(fields: [(&str, &str); N]) -> Option<Json> {
    Some(Json::obj([
        ("type", Json::str("object")),
        ("properties", Json::Obj(fields.iter().map(|(k, t)| (k.to_string(), Json::obj([("type", Json::str(*t))]))).collect())),
    ]))
}

fn arg(a: &Json, k: &str) -> Option<String> {
    a.get(k).and_then(Json::as_str).map(str::to_string)
}

fn need(a: &Json, k: &str) -> Result<String, String> {
    arg(a, k).ok_or_else(|| format!("missing argument `{k}`"))
}

fn num(a: &Json, k: &str) -> Option<f64> {
    match a.get(k) {
        Some(Json::Num(n)) => Some(*n),
        _ => None,
    }
}

fn confirmed(a: &Json) -> Result<(), String> {
    if a.get("confirm") == Some(&Json::Bool(true)) {
        Ok(())
    } else {
        Err("destructive action; fix: call again with `confirm: true`".into())
    }
}

/// Wraps a JSON array/value in an object so it can go in `structuredContent`.
fn wrap(key: &str, raw: &str) -> String {
    format!("{{{}:{raw}}}", brain_serve::json_str(key))
}

/// Bounded outputs (INTERNALS §25): an agent's context is finite, so every list a tool returns is capped.
const MAX_DIAGS: usize = 50;
const MAX_LOG_ITEMS: usize = 100;
const MAX_READ_BYTES: usize = 60 * 1024;

/// Caps every array named `diags` or `remaining` (at any depth) to `MAX_DIAGS` items starting at `cursor`, and adds
/// `<name>Total` plus `nextCursor` when more are left (call again with `cursor` to page).
fn page_diags(j: &mut Json, cursor: usize) {
    if let Json::Obj(kv) = j {
        let mut extra = Vec::new();
        for (k, v) in kv.iter_mut() {
            match v {
                Json::Arr(items) if k == "diags" || k == "remaining" => {
                    let total = items.len();
                    let start = cursor.min(total);
                    let end = (start + MAX_DIAGS).min(total);
                    if start > 0 || end < total {
                        *items = items.drain(start..end).collect();
                        extra.push((format!("{k}Total"), Json::Num(total as f64)));
                        if end < total {
                            extra.push(("nextCursor".to_string(), Json::Num(end as f64)));
                        }
                    }
                }
                _ => page_diags(v, cursor),
            }
        }
        kv.extend(extra);
    }
}

/// `page_diags` over a JSON string (tools build their output as text); malformed input is returned unchanged.
fn paged(raw: String, cursor: usize) -> String {
    match Json::parse(&raw) {
        Ok(mut j) => {
            page_diags(&mut j, cursor);
            j.stringify()
        }
        Err(_) => raw,
    }
}

/// Keeps the first `MAX_LOG_ITEMS` entries after `since` (oldest first); `more: true` tells the caller to ask again
/// with the last `seq` it got.
fn capped_items(key: &str, raw: &str) -> String {
    match Json::parse(raw) {
        Ok(Json::Arr(mut items)) => {
            let mut retained = 0;
            let mut bytes = 0;
            for item in &items {
                let size = item.stringify().len();
                if retained >= MAX_LOG_ITEMS || (retained > 0 && bytes + size > 262_144) { break; }
                bytes += size;
                retained += 1;
            }
            let more = items.len() > retained;
            items.truncate(retained);
            Json::Obj(vec![(key.to_string(), Json::Arr(items)), ("more".to_string(), Json::Bool(more))]).stringify()
        }
        _ => wrap(key, raw),
    }
}

/// Undo journal (`.brain/journal.jsonl`): one line per file write done through MCP, with the previous content
/// (`null` = the file did not exist).
fn journal_path(root: &Path) -> PathBuf {
    root.join(".brain/journal.jsonl")
}

fn journal_record(root: &Path, file: &Path, before: Option<&str>) {
    let rel = file.strip_prefix(root).unwrap_or(file).to_string_lossy().replace('\\', "/");
    let line = Json::obj([("path", Json::str(rel)), ("before", before.map_or(Json::Null, Json::str))]).stringify();
    let _ = std::fs::create_dir_all(root.join(".brain"));
    let mut cur = std::fs::read_to_string(journal_path(root)).unwrap_or_default();
    cur.push_str(&line);
    cur.push('\n');
    let _ = std::fs::write(journal_path(root), cur);
}

fn journal_undo(root: &Path, steps: usize) -> Result<Vec<String>, String> {
    let _writer = crate::edit::writer_lock(root)?;
    let text = std::fs::read_to_string(journal_path(root)).unwrap_or_default();
    let mut lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    if lines.is_empty() {
        return Err("nothing to undo: the journal is empty".into());
    }
    let mut undone = Vec::new();
    for _ in 0..steps.min(lines.len()) {
        let entry = Json::parse(lines.pop().unwrap())?;
        let rel = entry.get("path").and_then(Json::as_str).ok_or("broken journal entry")?;
        let path = writable(root, rel)?;
        match entry.get("before") {
            Some(Json::Str(prev)) => std::fs::write(&path, prev).map_err(|e| e.to_string())?,
            _ => {
                let _ = std::fs::remove_file(&path);
            }
        }
        undone.push(rel.to_string());
    }
    let rest: String = lines.iter().map(|l| format!("{l}\n")).collect();
    std::fs::write(journal_path(root), rest).map_err(|e| e.to_string())?;
    Ok(undone)
}

/// Validates a path an agent wants to write: relative to the app, no `..`, no dot files, project extensions only.
fn writable(root: &Path, rel: &str) -> Result<PathBuf, String> {
    const EXT: &[&str] = &[".brain", ".jsbrain", ".cssbrain", ".js", ".json", ".md", ".svg", ".txt"];
    let ok_ext = EXT.iter().any(|e| rel.ends_with(e));
    if rel.is_empty() || rel.starts_with('/') || rel.split('/').any(|s| s == ".." || s.starts_with('.')) || !ok_ext || rel.contains(".env") {
        return Err(format!("`{rel}` cannot be written; fix: use a path inside the app with one of {}", EXT.join(" ")));
    }
    crate::changes::source_path(root, rel)
}

/// Diagnostics of one file after a write, bounded so a response stays small (INTERNALS §25).
fn diags_after_write(root: &Path, rel: &str) -> Json {
    if !(rel.ends_with(".brain") || rel.ends_with(".jsbrain")) || rel.ends_with(".test.brain") {
        return Json::Arr(vec![]);
    }
    match run_check(Some(root.join(rel))) {
        Ok((_, d)) => match Json::parse(&diags_json(&d[..d.len().min(20)])) {
            Ok(v) => v,
            Err(_) => Json::Arr(vec![]),
        },
        Err(e) => Json::Arr(vec![Json::obj([("error", Json::str(e))])]),
    }
}

/// Shared state: the managed dev server and the debug browser pointed at it.
#[derive(Default)]
struct State {
    dev: Mutex<Option<brain_serve::Dev>>,
    /// impact report of the managed dev server (§43.1), read by brain_impact
    impact: Mutex<Option<crate::impact::Tracker>>,
    #[cfg(unix)]
    browser: Mutex<Option<crate::browser::Browser>>,
}

fn app(a: &Json) -> Result<PathBuf, String> {
    match arg(a, "app") {
        Some(p) => scaffold::find_root(&PathBuf::from(&p)).ok_or(format!("`{p}` is not a BRAINLLM app (no tokens.cssbrain)")),
        None => crate::app_root(),
    }
}

impl State {
    fn dev_url(&self) -> Result<String, String> {
        self.dev.lock().unwrap().as_ref().map(|d| d.url()).ok_or_else(|| "the dev server is not running; fix: call `brain_run_start`".to_string())
    }

    #[cfg(unix)]
    fn with_browser<T>(&self, f: impl FnOnce(&mut crate::browser::Browser) -> Result<T, String>) -> Result<T, String> {
        let url = self.dev_url()?;
        let mut b = self.browser.lock().unwrap();
        if b.as_ref().is_none_or(|b| b.origin() != url) {
            *b = Some(crate::browser::Browser::open(&url)?);
        }
        f(b.as_mut().unwrap())
    }
}

/// Raw HTTP/1.1 request to the local dev server (for `brain_api_call`).
fn http_call(base: &str, method: &str, path: &str, body: Option<&str>) -> Result<String, String> {
    use std::io::{Read, Write};
    let host = base.trim_start_matches("http://");
    let mut s = std::net::TcpStream::connect(host).map_err(|e| e.to_string())?;
    s.set_read_timeout(Some(std::time::Duration::from_secs(30))).ok();
    let body = body.unwrap_or("");
    write!(
        s,
        "{method} {path} HTTP/1.1\r\nHost: {host}\r\nOrigin: {base}\r\nX-Brain-RPC: 1\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .map_err(|e| e.to_string())?;
    let mut raw = String::new();
    s.read_to_string(&mut raw).map_err(|e| e.to_string())?;
    let (head, resp_body) = raw.split_once("\r\n\r\n").unwrap_or((&raw, ""));
    let status: u16 = head.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
    let ctype = head.lines().find_map(|l| l.to_ascii_lowercase().strip_prefix("content-type:").map(|v| v.trim().to_string())).unwrap_or_default();
    let body_json = if ctype.contains("json") { Json::parse(resp_body).unwrap_or(Json::str(resp_body)) } else { Json::str(resp_body.chars().take(4000).collect::<String>()) };
    Ok(Json::obj([("status", Json::Num(status as f64)), ("contentType", Json::str(ctype)), ("body", body_json)]).stringify())
}

/// The inspector buffer reduced to what a model can act on: the latest record per component, the worst updates and
/// frames, every leak and the latest errors (bounded output).
fn inspect_summary(m: &Json) -> Json {
    let mut heap = Json::Null;
    let mut comps: std::collections::BTreeMap<String, Json> = Default::default();
    let (mut upd, mut jank, mut errs): (Vec<Json>, Vec<Json>, Vec<Json>) = (vec![], vec![], vec![]);
    if let Some(Json::Arr(bs)) = m.get("batches") {
        for b in bs {
            let Some(Json::Arr(d)) = b.get("d") else { continue };
            for r in d {
                match r.get("k").and_then(Json::as_str) {
                    Some("heap") => heap = r.clone(),
                    Some("comp") => {
                        if let Some(h) = r.get("h").and_then(Json::as_str) {
                            comps.insert(h.to_string(), r.clone());
                        }
                    }
                    Some("upd") => upd.push(r.clone()),
                    Some("jank") => jank.push(r.clone()),
                    Some("err") => errs.push(r.clone()),
                    _ => {}
                }
            }
        }
    }
    let num = |r: &Json, k: &str| match r.get(k) {
        Some(Json::Num(n)) => *n,
        _ => 0.0,
    };
    let mut cs: Vec<Json> = comps.into_values().collect();
    cs.sort_by(|a, b| num(b, "est").total_cmp(&num(a, "est")));
    cs.truncate(10);
    upd.sort_by(|a, b| num(b, "ms").total_cmp(&num(a, "ms")));
    upd.truncate(10);
    let jank: Vec<Json> = jank.into_iter().rev().take(5).collect();
    let errs: Vec<Json> = errs.into_iter().rev().take(10).collect();
    Json::obj([
        ("heap", heap),
        ("components", Json::Arr(cs)),
        ("leaks", m.get("leaks").cloned().unwrap_or(Json::Arr(vec![]))),
        ("updates", Json::Arr(upd)),
        ("jank", Json::Arr(jank)),
        ("errors", Json::Arr(errs)),
    ])
}

pub fn run_profile(profile: &str) -> Result<(), String> {
    if !matches!(profile,"full"|"minimal"|"backend") {return Err("unknown MCP profile; use full, minimal or backend".into())}
    let notifier = Notifier::new(Box::new(std::io::stdout()));
    let st: Arc<State> = Arc::default();
    let mut tools: Vec<Tool> = Vec::new();

    // ---------- project ----------
    tools.push(Tool {
        name: "brain_info",
        description: "Toolchain and project facts: version, app root, embedded runtime modules, Chrome and Node availability, dev server status.",
        input_schema: schema([], &[]),
        output_schema: out([("version", "string"), ("root", "string"), ("runtime", "array"), ("chrome", "string"), ("node", "string"), ("dev", "string")]),
        run: {
            let st = st.clone();
            Box::new(move |_| {
                let node = std::process::Command::new("node").arg("--version").output().ok().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
                #[cfg(unix)]
                let chrome = crate::cdp::find_chrome().map(|p| p.display().to_string()).unwrap_or_default();
                #[cfg(not(unix))]
                let chrome = String::new();
                Ok(Json::obj([
                    ("version", Json::str(env!("CARGO_PKG_VERSION"))),
                    ("root", Json::str(crate::app_root().map(|p| p.display().to_string()).unwrap_or_default())),
                    ("runtime", Json::Arr(crate::runtime_files().into_iter().map(|(n, _)| Json::Str(n)).collect())),
                    ("chrome", Json::str(chrome)),
                    ("node", Json::str(node)),
                    ("dev", Json::str(st.dev_url().unwrap_or_else(|_| "stopped".into()))),
                ])
                .stringify())
            })
        },
    });
    tools.push(Tool {
        name: "brain_spec",
        description: "The BRAINLLM spec (.brain format, BrainScript, markup, CSS, a11y, tests, CLI). Read it before writing components; `topic` returns one section to save context.",
        input_schema: schema(
            [
                ("topic", "string", "files | script | markup | channels | css | a11y | tests | cli | seo | server | component | example"),
                ("all", "boolean", "include planned, not yet implemented parts (default false)"),
            ],
            &[],
        ),
        output_schema: None,
        run: Box::new(|a| {
            let all = a.get("all") == Some(&Json::Bool(true));
            match arg(a, "topic") {
                Some(t) => spec::topic(&t, all),
                None => Ok(if all { spec::SPEC_ALL.to_string() } else { spec::served() }),
            }
        }),
    });
    tools.push(Tool {
        name: "brain_guide",
        description: "Task playbooks: what to do for one kind of task, in which order, the commands that verify it, the diagnostic codes that catch common mistakes and a minimal correct example. Call it without `topic` for the list; read `start` first, then the topic of the task (e.g. `seo` before writing a page that must be found).",
        input_schema: schema([("topic", "string", "start | views | data | forms | server | seo | a11y | webmcp | tests | i18n | performance; omit for the list")], &[]),
        output_schema: None,
        run: Box::new(|a| crate::guide::text(arg(a, "topic").as_deref())),
    });
    tools.push(Tool {
        name: "brain_map",
        description: "Project graph: routes (path, file, params, load), components (props with type/default, emits, expose, imports), .jsbrain modules (exports), tests, tokens, and `reverse` (who reads/writes each store field, context key and resource). Filter with view/comp/reads; components come 100 per page (total, nextCursor). Read it instead of opening files; use brain_card for one component's contract.",
        input_schema: schema(
            [
                ("app", "string", "app folder; defaults to the current one"),
                ("view", "string", "only this route (`/users/:id`) or view file and the components it uses"),
                ("comp", "string", "only this component (name or file)"),
                ("reads", "string", "who reads or writes this store field / context key / resource (`cart` also matches `cart.total`)"),
                ("limit", "number", "components per page (default 100)"),
                ("cursor", "number", "nextCursor of the previous page"),
            ],
            &[],
        ),
        output_schema: out([("routes", "array"), ("components", "array"), ("total", "number"), ("reverse", "object")]),
        run: Box::new(|a| {
            let s = |k: &str| a.get(k).and_then(Json::as_str).map(str::to_string);
            let q = crate::map::Query { view: s("view"), comp: s("comp"), reads: s("reads"), limit: num(a, "limit").map(|n| n as usize), cursor: num(a, "cursor").unwrap_or(0.0) as usize };
            Ok(crate::map::query(&app(a)?, &q).stringify())
        }),
    });
    tools.push(Tool {
        name: "brain_new",
        description: "Creates a new BRAINLLM app (app.brain, tokens.cssbrain, views/, comp/, ui/, AGENTS.md). With `dir`, the app is created EXACTLY in that folder (use \".\" for the current one; it must be empty apart from dot files) and `name` only names it. Without `dir`, it creates the folder `<name>` inside the current one.",
        input_schema: schema(
            [("name", "string", "app name (letters, digits, hyphens); also the folder name when `dir` is omitted"), ("dir", "string", "exact app root, e.g. \".\"")],
            &["name"],
        ),
        output_schema: out([("app", "string"), ("files", "array")]),
        run: Box::new(|a| {
            let name = need(a, "name")?;
            let dir = match arg(a, "dir") {
                Some(d) => std::path::absolute(PathBuf::from(d)).map_err(|e| e.to_string())?,
                None => crate::cwd().join(&name),
            };
            let name = if name == "." { dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default() } else { name };
            let files = scaffold::new_app_named(&dir, &name)?;
            Ok(Json::obj([
                ("app", Json::str(dir.display().to_string())),
                ("files", Json::Arr(files.iter().map(|f| Json::str(f.strip_prefix(&dir).unwrap_or(f).display().to_string())).collect())),
            ])
            .stringify())
        }),
    });

    // ---------- modules and files ----------
    tools.push(Tool {
        name: "brain_add",
        description: "Creates a file from a checked template, including channel, service, api, type, guard, server and mandatory before-hook modules. Hooks fail closed until implemented. Type additions preserve existing declarations. props: \"name:type[?][=default] ...\" with types str num bool list obj fn any. Fill the template in; do not write files from scratch.",
        input_schema: schema(
            [
                ("kind", "string", "view | comp | ui | test | layout | lib | locale | collection | channel | service | api | type | guard | server | hook"),
                ("name", "string", "component name (kebab or PascalCase)"),
                ("props", "string", "typed props, e.g. \"user:obj active:bool=false note:str?\""),
                ("store", "string", "channel literal object initializer"),
                ("bus", "boolean", "generate an event bus channel"),
                ("key", "boolean", "generate a context key channel (default)"),
                ("get", "string", "API GET response schema"),
                ("post", "string", "API POST input:output schemas"),
                ("fields", "string", "type fields, e.g. id:num! email:email!"),
                ("app", "string", "app folder; defaults to the current one"),
            ],
            &["kind", "name"],
        ),
        output_schema: out([("file", "string")]),
        run: Box::new(|a| {
            let root = app(a)?;
            for key in ["props", "store", "get", "post", "fields"] {
                if a.get(key).is_some_and(|v| v.as_str().is_none()) { return Err(format!("`{key}` must be a string")); }
            }
            for key in ["bus", "key"] {
                if a.get(key).is_some_and(|v| !matches!(v, Json::Bool(_))) { return Err(format!("`{key}` must be a boolean")); }
            }
            let kind = need(a, "kind")?;
            let before = if kind == "type" { std::fs::read_to_string(root.join("types.jsbrain")).ok() } else { None };
            let options = scaffold::AddOptions {
                props: arg(a, "props").unwrap_or_default(), store: arg(a, "store"),
                bus: a.get("bus") == Some(&Json::Bool(true)), key: a.get("key") == Some(&Json::Bool(true)),
                get: arg(a, "get"), post: arg(a, "post"), fields: arg(a, "fields"),
            };
            let p = scaffold::add_with_options(&root, &kind, &need(a, "name")?, &options)?;
            journal_record(&root, &p, before.as_deref());
            Ok(Json::obj([("file", Json::str(p.strip_prefix(&root).unwrap_or(&p).display().to_string()))]).stringify())
        }),
    });
    tools.push(Tool {
        name: "brain_write",
        description: "Creates or overwrites a project file with `content` (paths inside the app only; .brain .jsbrain .cssbrain .js .json .md .svg .txt). Returns the file's diagnostics from brain_check in the same response. Optional expectedRevision rejects stale overwrites. For concurrent agents use brain_changes leases/plan/apply. Reversible with brain_undo.",
        input_schema: schema([("path", "string", "path relative to the app, e.g. views/about.brain"), ("content", "string", "full file content"), ("app", "string", "app folder"), ("expectedRevision", "string", "optional SHA-256 revision from brain_read; rejects stale overwrites")], &["path", "content"]),
        output_schema: out([("file", "string"), ("created", "boolean"), ("diags", "array")]),
        run: Box::new(|a| {
            let root = app(a)?;
            let _writer = crate::edit::writer_lock(&root)?;
            let rel = need(a, "path")?;
            let content = a.get("content").and_then(Json::as_str).ok_or("missing argument `content`")?;
            if content.len() > 200_000 {
                return Err("content larger than 200 KB; fix: split the file".into());
            }
            let p = writable(&root, &rel)?;
            if let Some(expected)=arg(a,"expectedRevision") {
                let current=crate::changes::read(&root,&rel)?;
                if current["revision"].as_str()!=Some(expected.as_str()) {return Err("source revision conflict; reread the file and retry with its current revision, or use brain_changes for leased plans".into());}
            }
            let before = std::fs::read_to_string(&p).ok();
            journal_record(&root, &p, before.as_deref());
            std::fs::create_dir_all(p.parent().unwrap()).map_err(|e| e.to_string())?;
            std::fs::write(&p, content).map_err(|e| e.to_string())?;
            Ok(Json::obj([("file", Json::str(&rel)), ("created", Json::Bool(before.is_none())), ("diags", diags_after_write(&root, &rel))]).stringify())
        }),
    });
    tools.push(Tool {
        name: "brain_edit",
        description: "Checked AST edits with op=add-prop/remove-prop/rename/move/extract. Rename changes a component file and its import paths, not JavaScript symbols. Without op, exact old/new replacement remains supported. Reversible with brain_undo.",
        input_schema: schema(
            [("path", "string", "path relative to the app"), ("old", "string", "exact text to replace"), ("new", "string", "replacement text"), ("all", "boolean", "replace every occurrence"), ("app", "string", "app folder"), ("op", "string", "add-prop | remove-prop | rename | move | extract"), ("declaration", "string", "one typed prop, e.g. title:str?"), ("name", "string", "prop name or PascalCase component name"), ("to", "string", "destination comp/ or ui/ .brain path"), ("selector", "string", "subtree selector for extract")],
            &["path"],
        ),
        output_schema: out([("file", "string"), ("replaced", "number"), ("diags", "array"), ("changed", "array"), ("checked", "boolean")]),
        run: Box::new(|a| {
            let root = app(a)?;
            let rel = need(a, "path")?;
            if a.get("op").is_some() {
                let op = need(a, "op")?;
                if ["old", "new", "all"].iter().any(|k| a.get(k).is_some()) { return Err("AST op cannot be combined with old/new/all text edit fields".into()); }
                use crate::edit::Operation;
                let operation = match op.as_str() {
                    "add-prop" => Operation::AddProp { declaration: need(a, "declaration")? },
                    "remove-prop" => Operation::RemoveProp { name: need(a, "name")? },
                    "rename" => Operation::Rename { name: need(a, "name")? },
                    "move" => Operation::Move { to: need(a, "to")? },
                    "extract" => Operation::Extract { selector: need(a, "selector")?, name: need(a, "name")? },
                    _ => return Err("unknown edit op; use add-prop, remove-prop, rename, move or extract".into()),
                };
                return crate::edit::apply(&root, &rel, operation, &mut |path, before| journal_record(&root, path, before)).map(|j| j.stringify());
            }
            let _writer = crate::edit::writer_lock(&root)?;
            let (old, new) = (need(a, "old")?, a.get("new").and_then(Json::as_str).ok_or("missing argument `new`")?.to_string());
            let p = writable(&root, &rel)?;
            let src = std::fs::read_to_string(&p).map_err(|e| format!("{rel}: {e}; fix: create it with brain_write or brain_add"))?;
            let n = src.matches(&old).count();
            let all = a.get("all") == Some(&Json::Bool(true));
            if n == 0 {
                return Err(format!("`old` does not appear in {rel}; fix: brain_read the file and copy the exact text (whitespace included)"));
            }
            if n > 1 && !all {
                return Err(format!("`old` appears {n} times in {rel}; fix: include more surrounding text so it is unique, or pass `all: true`"));
            }
            let out_src = if all { src.replace(&old, &new) } else { src.replacen(&old, &new, 1) };
            journal_record(&root, &p, Some(&src));
            std::fs::write(&p, out_src).map_err(|e| e.to_string())?;
            Ok(Json::obj([("file", Json::str(&rel)), ("replaced", Json::Num(if all { n } else { 1 } as f64)), ("diags", diags_after_write(&root, &rel))]).stringify())
        }),
    });
    tools.push(Tool {
        name: "brain_read",
        description: "Reads a project file and, for .brain/.jsbrain, its summary from the project map (props, emits, imports, exports).",
        input_schema: schema([("path", "string", "path relative to the app, e.g. comp/counter.brain"), ("offset", "number", "byte offset for files over 60 KB (from `nextOffset`)"), ("app", "string", "app folder")], &["path"]),
        output_schema: out([("path", "string"), ("source", "string"), ("summary", "object"), ("size", "number"), ("nextOffset", "number"), ("revision", "string")]),
        run: Box::new(|a| {
            let root = app(a)?;
            let rel = need(a, "path")?;
            if rel.contains("..") || rel.contains(".env") || rel.contains(".brain_env") {
                return Err(format!("`{rel}` is outside the app or a secrets file"));
            }
            let stored = crate::changes::read(&root,&rel)?;
            let source=stored["content"].as_str().ok_or_else(||format!("{rel}: no such source file"))?;
            let map = crate::map::build(&root);
            let summary = ["components", "modules", "tests"]
                .iter()
                .filter_map(|k| match map.get(k) {
                    Some(Json::Arr(v)) => v.iter().find(|c| c.get("file").and_then(Json::as_str) == Some(rel.as_str())).cloned(),
                    _ => None,
                })
                .next()
                .unwrap_or(Json::Null);
            // Files over MAX_READ_BYTES come in pages cut on line boundaries; `nextOffset` continues.
            let size = source.len();
            let mut start = (num(a, "offset").unwrap_or(0.0) as usize).min(size);
            while !source.is_char_boundary(start) {
                start -= 1;
            }
            let mut end = (start + MAX_READ_BYTES).min(size);
            if end < size {
                end = source[start..end].rfind('\n').map(|i| start + i + 1).filter(|&e| e > start).unwrap_or(end);
                while !source.is_char_boundary(end) {
                    end -= 1;
                }
            }
            let mut j = vec![("revision".to_string(), Json::str(stored["revision"].as_str().unwrap_or(""))), ("path".to_string(), Json::str(&rel)), ("source".to_string(), Json::str(&source[start..end])), ("summary".to_string(), summary)];
            if start > 0 || end < size {
                j.push(("size".to_string(), Json::Num(size as f64)));
                if end < size {
                    j.push(("nextOffset".to_string(), Json::Num(end as f64)));
                }
            }
            Ok(Json::Obj(j).stringify())
        }),
    });
    tools.push(Tool {
        name: "brain_remove",
        description: "Deletes a project file. Destructive: requires `confirm: true`; `brain_undo` restores it.",
        input_schema: schema([("path", "string", "path relative to the app"), ("confirm", "boolean", "must be true"), ("app", "string", "app folder")], &["path", "confirm"]),
        output_schema: out([("removed", "string")]),
        run: Box::new(|a| {
            confirmed(a)?;
            let root = app(a)?;
            let _writer = crate::edit::writer_lock(&root)?;
            let rel = need(a, "path")?;
            if rel.contains("..") || rel.starts_with(".brain") {
                return Err(format!("`{rel}` cannot be removed"));
            }
            let p = writable(&root, &rel)?;
            let prev = std::fs::read_to_string(&p).map_err(|e| format!("{rel}: {e}"))?;
            journal_record(&root, &p, Some(&prev));
            std::fs::remove_file(&p).map_err(|e| e.to_string())?;
            Ok(Json::obj([("removed", Json::str(rel))]).stringify())
        }),
    });
    tools.push(Tool {
        name: "brain_undo",
        description: "Reverts the last N file writes made through MCP (brain_add, brain_remove, brain_fix). Destructive: requires `confirm: true`.",
        input_schema: schema([("steps", "number", "how many writes to revert (default 1)"), ("confirm", "boolean", "must be true"), ("app", "string", "app folder")], &["confirm"]),
        output_schema: out([("undone", "array")]),
        run: Box::new(|a| {
            confirmed(a)?;
            let undone = journal_undo(&app(a)?, num(a, "steps").unwrap_or(1.0).max(1.0) as usize)?;
            Ok(Json::obj([("undone", Json::Arr(undone.into_iter().map(Json::Str).collect()))]).stringify())
        }),
    });

    // ---------- quality ----------
    tools.push(Tool {
        name: "brain_check",
        description: "Compiles the app's .brain files (or one file) and returns {diags:[{code,file,line,col,msg,fix,edits}]}. Empty `diags` = no errors.",
        input_schema: schema([("path", "string", ".brain file or app folder; defaults to the current app"), ("cursor", "number", "index of the first diag to return (from `nextCursor`)")], &[]),
        output_schema: out([("files", "number"), ("diags", "array"), ("diagsTotal", "number"), ("nextCursor", "number")]),
        run: Box::new(|a| {
            let (n, diags) = run_check(arg(a, "path").map(PathBuf::from))?;
            let j = paged(format!("{{\"files\":{n},\"diags\":{}}}", diags_json(&diags)), num(a, "cursor").unwrap_or(0.0) as usize);
            if diags.iter().all(|d| d.is_warning()) { Ok(j) } else { Err(j) }
        }),
    });
    tools.push(Tool {
        name: "brain_fix",
        description: "Applies every machine fix that brain_check proposes (`edits`), re-checks and returns what was applied plus the errors left (fix those by hand with their `fix`). Reversible with brain_undo.",
        input_schema: schema([("path", "string", ".brain file or folder; defaults to the current app")], &[]),
        output_schema: out([("applied", "array"), ("errors", "number"), ("remaining", "array")]),
        run: Box::new(|a| {
            let root = crate::app_root().ok();
            let j = crate::run_fix_with(arg(a, "path").map(PathBuf::from), &mut |p, prev| {
                if let Some(r) = &root {
                    journal_record(r, p, Some(prev));
                }
            })?;
            let errors = matches!(j.get("errors"), Some(Json::Num(n)) if *n > 0.0);
            let j = paged(j.stringify(), 0);
            if errors { Err(j) } else { Ok(j) }
        }),
    });
    tools.push(Tool {
        name: "brain_env_set",
        description: "Sets one variable of .brain_env (the app's environments, §36) without ever returning a value: key, section (development, staging, production; empty = shared by all) and value. PUBLIC_* keys reach the browser, the rest stay on the server. The file itself cannot be read or written with brain_read/brain_write; use this and brain_env_keys.",
        input_schema: schema(
            [("key", "string", "UPPER_SNAKE_CASE, e.g. PUBLIC_API_URL"), ("section", "string", "development | staging | production | \"\" (shared)"), ("value", "string", "one line"), ("app", "string", "app folder; defaults to the current one")],
            &["key", "section", "value"],
        ),
        output_schema: out([("ok", "boolean"), ("key", "string"), ("section", "string")]),
        run: Box::new(|a| {
            let g = |k: &str| a.get(k).and_then(Json::as_str).map(str::to_string).ok_or(format!("missing `{k}`"));
            let (key, section, value) = (g("key")?, g("section")?, g("value")?);
            crate::env::set(&app(a)?, &key, &section, &value)?;
            Ok(Json::obj([("ok", Json::Bool(true)), ("key", Json::str(&key)), ("section", Json::str(if section.is_empty() { "shared" } else { section.as_str() })), ("set", Json::Bool(true))]).stringify())
        }),
    });
    tools.push(Tool {
        name: "brain_env_keys",
        description: "The keys of .brain_env per environment (development, staging, production…), never their values. Keys whose value is empty in an environment still appear; `brain build --env production` fails on them (B0720).",
        input_schema: schema([("app", "string", "app folder; defaults to the current one")], &[]),
        output_schema: None,
        run: Box::new(|a| Ok(crate::env::map(&app(a)?).unwrap_or(Json::Obj(vec![])).stringify())),
    });
    tools.push(Tool {
        name: "brain_card",
        description: "A component's contract without its body: props (type, default, optional, bindable), emits, slots, the stores/context it reads and writes, its children, size and check status, plus `suggest` when it is over the size limit. Compose pages from cards; never read component bodies to use them.",
        input_schema: schema([("comp", "string", "component name or file, as brain_map lists it"), ("app", "string", "app folder; defaults to the current one")], &["comp"]),
        output_schema: out([("name", "string"), ("file", "string"), ("props", "array"), ("emits", "array"), ("slots", "array"), ("reads", "array"), ("writes", "array"), ("children", "array"), ("size", "object")]),
        run: Box::new(|a| {
            let comp = a.get("comp").and_then(Json::as_str).ok_or("missing `comp`")?;
            crate::card::card(&app(a)?, comp).map(|j| j.stringify())
        }),
    });
    tools.push(Tool {
        name: "brain_split",
        description: "CALL IT when a component passes ~150 lines, repeats the same markup block twice, or you are about to copy part of it into another file: one call does the extraction instead of hand-editing two files. Extracts the subtree that `selector` picks (one compound selector: tag, .class, #id, [attr]) out of a component into a new one named `name` (PascalCase; written to comp/ or ui/), with props inferred from what it reads and emits from the parent handlers it calls; the original then uses it. Refused, with nothing changed, if either file fails brain check. brain_undo reverts it.",
        input_schema: schema(
            [("comp", "string", "component name or file"), ("selector", "string", "e.g. `li.row`, `article.body`"), ("name", "string", "new component name, PascalCase"), ("app", "string", "app folder; defaults to the current one")],
            &["comp", "selector", "name"],
        ),
        output_schema: out([("original", "string"), ("created", "string"), ("props", "array"), ("emits", "array")]),
        run: Box::new(|a| {
            let g = |k: &str| a.get(k).and_then(Json::as_str).ok_or(format!("missing `{k}`"));
            let root = app(a)?;
            let r = root.clone();
            crate::card::split(&root, g("comp")?, g("selector")?, g("name")?, &mut |p, before| journal_record(&r, p, before)).map(|j| j.stringify())
        }),
    });
    tools.push(Tool {
        name: "brain_explain",
        description: "Explains a diagnostic code from brain_check: title, why the rule exists, and a minimal correct example.",
        input_schema: schema([("code", "string", "diagnostic code, e.g. B0302")], &["code"]),
        output_schema: out([("code", "string"), ("title", "string"), ("why", "string"), ("example", "string")]),
        run: Box::new(|a| crate::explain_json(&need(a, "code")?).map(|j| j.stringify())),
    });
    #[cfg(unix)]
    tools.push(Tool {
        name: "brain_status",
        description: "Status of the production process manager (`brain start`): mode (static|server), url, build signature, uptime, instances with pid and RSS, restarts and the last exit with its reason. Read-only: start, stop and reload stay CLI-only.",
        input_schema: schema([("app", "string", "app folder")], &[]),
        output_schema: out([("ok", "boolean"), ("mode", "string"), ("build", "string"), ("instances", "array"), ("restarts", "number")]),
        run: Box::new(|a| crate::start::send(&app(a)?, "status")),
    });
    // brain_seo_* (audit, links, preview of the built site): defined by brain-seo, registered here
    for &(name, description, params) in brain_seo::TOOLS {
        let props = Json::Obj(params.iter().map(|(k, ty, d)| (k.to_string(), Json::obj([("type", Json::str(*ty)), ("description", Json::str(*d))]))).collect());
        tools.push(Tool {
            name,
            description,
            input_schema: Json::obj([("type", Json::str("object")), ("properties", props), ("required", Json::Arr(vec![])), ("additionalProperties", Json::Bool(false))]),
            output_schema: None,
            run: Box::new(move |a| {
                let root = crate::app_root()?;
                let j = brain_seo::tool(name, a, &root)?;
                if matches!(j.get("ok"), Some(Json::Bool(false))) { Err(j.stringify()) } else { Ok(j.stringify()) }
            }),
        });
    }
    tools.push(Tool {
        name: "brain_doctor",
        description: "Everything at once: check (errors + over-engineering warnings), tests in headless Chrome with an accessibility audit, and leaks. Returns a single summary with `ok`; use it before calling a task done.",
        input_schema: schema([("path", "string", "app or folder; defaults to the current app"), ("cursor", "number", "index of the first check diag to return (from `nextCursor`)")], &[]),
        output_schema: out([("ok", "boolean"), ("check", "object"), ("test", "object"), ("heap", "object")]),
        run: Box::new(|a| {
            let (ok, j) = crate::run_doctor(arg(a, "path").map(PathBuf::from))?;
            let j = paged(j.stringify(), num(a, "cursor").unwrap_or(0.0) as usize);
            if ok { Ok(j) } else { Err(j) }
        }),
    });
    tools.push(Tool {
        name: "brain_test",
        description: "Runs declared *.test.brain scenarios in a detected connected Claude Viewer/Codex Viewer first, with a complementary owned headless Chrome lane for the full accessibility/heap gates. Returns explicit engine/coverage/fallback evidence; a connected Viewer failure cannot become a headless success.",
        input_schema: schema([("path", "string", ".test.brain file or folder; defaults to the current app")], &[]),
        output_schema: out([("files", "array")]),
        run: Box::new(|a| {
            #[cfg(unix)]
            {
                let reports = crate::run_tests(arg(a, "path").map(PathBuf::from), 60)?;
                let failed = reports.iter().any(crate::testrun::FileReport::failed);
                let j = wrap("files", &Json::Arr(reports.iter().map(crate::testrun::FileReport::to_json).collect()).stringify());
                if failed { Err(j) } else { Ok(j) }
            }
            #[cfg(not(unix))]
            {
                let _ = a;
                Err("brain test needs macOS or Linux".into())
            }
        }),
    });
    #[cfg(unix)]
    tools.push(Tool {
        name: "brain_shot",
        description: "Screenshot and accessibility text/audit from the managed debug browser. Optional named PNG baseline compares pixels within maxDiff (0..1). updateBaseline explicitly replaces that baseline. Returns files, text, comparison and ok; caller must inspect ok.",
        input_schema: schema(
            [("route", "string", "local route, e.g. /users/42"), ("width", "number", "viewport width, default 1280"), ("height", "number", "viewport height, default 800"), ("baseline", "string", "named visual baseline"), ("updateBaseline", "boolean", "explicitly accept current pixels as baseline"), ("maxDiff", "number", "allowed changed-pixel ratio, default 0")],
            &["route"],
        ),
        output_schema: out([("ok", "boolean"), ("file", "string"), ("textFile", "string"), ("text", "string"), ("page", "object"), ("a11y", "array"), ("comparison", "object")]),
        run: {
            let st = st.clone();
            Box::new(move |a| {
                let root = crate::app_root()?;
                let route = need(a, "route")?;
                let dimension = |key: &str, default: f64| -> Result<u32,String> {
                    let n = num(a,key).unwrap_or(default);
                    if !n.is_finite() || n.fract() != 0.0 || !(1.0..=4096.0).contains(&n) { return Err(format!("{key} must be an integer from 1 to 4096")); }
                    Ok(n as u32)
                };
                let (w,h) = (dimension("width",1280.0)?,dimension("height",800.0)?);
                let opts = crate::shot::Options { baseline: arg(a,"baseline"), update: a.get("updateBaseline") == Some(&Json::Bool(true)), max_diff: num(a,"maxDiff").unwrap_or(0.0) };
                st.with_browser(|b| crate::shot::capture(b,&root,&route,w,h,&opts)).map(|j|j.stringify())
            })
        },
    });

    // ---------- data ----------
    tools.push(Tool {
        name: "brain_load",
        description: "Runs the `load(ctx)` of the view that serves `route` (with its params and query) on the managed dev server and returns the props it produces.",
        input_schema: schema([("route", "string", "route with optional query, e.g. /users/42?tab=x")], &["route"]),
        output_schema: out([("route", "string"), ("view", "string"), ("params", "object"), ("props", "object")]),
        run: {
            let st = st.clone();
            Box::new(move |a| {
                let route = need(a, "route")?;
                let dev = st.dev.lock().unwrap();
                let d = dev.as_ref().ok_or("the dev server is not running; fix: call `brain_run_start`")?;
                d.load(&route)
            })
        },
    });
    tools.push(Tool {
        name: "brain_api_call",
        description: "HTTP request to the managed dev server (same origin, with the RPC header). Returns {status, contentType, body}.",
        input_schema: schema(
            [("path", "string", "path, e.g. /_r/<hash>/save"), ("method", "string", "GET | POST | PUT | PATCH | DELETE (default GET)"), ("body", "string", "JSON body")],
            &["path"],
        ),
        output_schema: out([("status", "number"), ("contentType", "string"), ("body", "object")]),
        run: {
            let st = st.clone();
            Box::new(move |a| {
                let path = need(a, "path")?;
                if !path.starts_with('/') {
                    return Err("`path` must start with `/` (only the local dev server is reachable)".into());
                }
                let method = arg(a, "method").unwrap_or_else(|| "GET".into()).to_ascii_uppercase();
                http_call(&st.dev_url()?, &method, &path, arg(a, "body").as_deref())
            })
        },
    });

    // ---------- run and debug ----------
    tools.push(Tool {
        name: "brain_run_start",
        description: "Starts the dev server (hot reload) inside this MCP process. While it runs, HMR results, build errors and browser errors arrive as MCP notifications.",
        input_schema: schema([("port", "number", "port (default 5180; tries the next 20 if busy)"), ("app", "string", "app folder")], &[]),
        output_schema: out([("url", "string"), ("port", "number")]),
        run: {
            let (st, notifier) = (st.clone(), notifier.clone());
            Box::new(move |a| {
                let mut dev = st.dev.lock().unwrap();
                if let Some(d) = dev.as_ref() {
                    return Ok(Json::obj([("url", Json::str(d.url())), ("port", Json::Num(d.addr.port() as f64))]).stringify());
                }
                let root = app(a)?;
                let opts = brain_serve::Options {
                    root: root.clone(),
                    host: "127.0.0.1".into(),
                    port: num(a, "port").unwrap_or(5180.0) as u16,
                    head_prepend: crate::head_prepend(),
                    runtime: crate::runtime_files(),
                    env: Some(crate::env::loader(&root, "development")),
                    base: brain_pack::read_base(&root),
                    diagnose: Some(crate::dev_diagnose(&root)),
                    problems: Some(crate::dev_problems(&root)),
                    upstream: None,
                    mock: None,
                };
                let n = notifier.clone();
                let tracker = crate::impact::Tracker::new(&root, false);
                *st.impact.lock().unwrap() = Some(tracker.clone());
                let on_event: brain_serve::EventFn = Arc::new(move |ev: &str| {
                    tracker.event(ev);
                    let Ok(j) = Json::parse(ev) else { return };
                    let level = match j.get("type").and_then(Json::as_str) {
                        Some("error") | Some("client-error") => "error",
                        Some("log") => return, // the log ring is read with brain_logs
                        _ => "info",
                    };
                    n.log(level, j);
                });
                let d = crate::start_project_dev(opts, crate::compile_fn(root.clone(), true), true, Some(on_event), "development")?;
                let info = Json::obj([("url", Json::str(d.url())), ("port", Json::Num(d.addr.port() as f64))]);
                let _ = std::fs::create_dir_all(root.join(".brain"));
                let _ = std::fs::write(crate::dev_json_path(&root), format!("{{\"port\":{},\"pid\":{},\"url\":{}}}", d.addr.port(), std::process::id(), brain_serve::json_str(&d.url())));
                *dev = Some(d);
                Ok(info.stringify())
            })
        },
    });
    tools.push(Tool {
        name: "brain_inspect",
        description: "Memory and runtime health of the app open in the dev server (§9), without a screen: the JS heap now, the heaviest components (instances, sig/drv/fx, DOM nodes, estimated bytes), suspected leaks B0601 (instance alive after unmount) / B0602 (global listener or timer in an fx without cleanup) with their fix, the worst updates and long frames, and the latest captured errors. Needs a page open on `brain run` (brain_navigate or a browser).",
        input_schema: schema([("app", "string", "app folder; defaults to the current one")], &[]),
        output_schema: out([("heap", "object"), ("components", "array"), ("leaks", "array"), ("updates", "array"), ("jank", "array"), ("errors", "array")]),
        run: {
            let st = st.clone();
            Box::new(move |a| {
                // the in-process dev server, else the `brain run` of this app (.brain/dev.json)
                let url = st.dev_url().or_else(|_| {
                    let root = app(a)?;
                    let j = std::fs::read_to_string(crate::dev_json_path(&root)).map_err(|_| "no dev server for this app; fix: call brain_run_start (or run `brain run`) and open a page".to_string())?;
                    Json::parse(&j).ok().and_then(|j| j.get("url").and_then(Json::as_str).map(str::to_string)).ok_or("unreadable .brain/dev.json".to_string())
                })?;
                let (origin, pre) = match url.trim_start_matches("http://").split_once('/') {
                    Some((host, p)) => (format!("http://{host}"), format!("/{p}")),
                    None => (url.clone(), String::new()),
                };
                let raw = http_call(&origin, "GET", &format!("{}/@brain/metrics", pre.trim_end_matches('/')), None)?;
                let body = Json::parse(&raw).ok().and_then(|j| j.get("body").cloned()).unwrap_or(Json::Null);
                Ok(inspect_summary(&body).stringify())
            })
        },
    });
    tools.push(Tool {
        name: "brain_impact",
        description: "CALL IT after every save while brain_run_start is running (and before brain_doctor): it is the cheap check of what the edit did. What your last saves did, one record per save (newest last): diagnostics the save introduced (`new`, with their edits) or fixed (`fixed`), the error count of the file, and the Δ gz bytes of every view whose production output changed (`bytes`: view, route, before, after, delta). With a debug page open (brain_navigate), `interactions` lists the last clicks/inputs with how many nodes each mutated and its ms, and `mounts` the mount time of the last mounted components (`hmr: true` after a save). Needs brain_run_start.",
        input_schema: schema([("last", "number", "how many records (default 5, max 50)")], &[]),
        output_schema: out([("records", "array")]),
        run: {
            let st = st.clone();
            Box::new(move |a| {
                let t = st.impact.lock().unwrap().clone().ok_or("the dev server is not running; fix: call brain_run_start, save a file, then brain_impact")?;
                let n = num(a, "last").unwrap_or(5.0).clamp(1.0, 50.0) as usize;
                let recs: Vec<Json> = t.last(n).iter().filter_map(|r| Json::parse(r).ok()).collect();
                // updates per interaction (RUNTIME #198), from the debug page when one is open: click/input/… on
                // which element, how many nodes it mutated and in how many ms
                #[cfg(unix)]
                let inter = st.with_browser(|b| b.eval("globalThis.__brainWhy ? globalThis.__brainWhy.interactions().slice(-20) : null")).unwrap_or(Json::Null);
                // mount time per component (newest first, HMR remounts marked `hmr`): compare a component's ms before
                // and after a save
                #[cfg(unix)]
                let mounts = st.with_browser(|b| b.eval("globalThis.__brainWhy && globalThis.__brainWhy.mounts ? globalThis.__brainWhy.mounts().slice(0, 20) : null")).unwrap_or(Json::Null);
                #[cfg(not(unix))]
                let (inter, mounts) = (Json::Null, Json::Null);
                Ok(Json::obj([("records", Json::Arr(recs)), ("interactions", inter), ("mounts", mounts)]).stringify())
            })
        },
    });
    tools.push(Tool {
        name: "brain_run_status",
        description: "Whether the managed dev server is running, and its URL.",
        input_schema: schema([], &[]),
        output_schema: out([("running", "boolean"), ("url", "string")]),
        run: {
            let st = st.clone();
            Box::new(move |_| {
                let url = st.dev_url().ok();
                Ok(Json::obj([("running", Json::Bool(url.is_some())), ("url", Json::str(url.unwrap_or_default()))]).stringify())
            })
        },
    });
    tools.push(Tool {
        name: "brain_run_stop",
        description: "Stops the managed dev server and closes the debug browser.",
        input_schema: schema([], &[]),
        output_schema: out([("stopped", "boolean")]),
        run: {
            let st = st.clone();
            Box::new(move |_| {
                #[cfg(unix)]
                {
                    *st.browser.lock().unwrap() = None;
                }
                let d = st.dev.lock().unwrap().take();
                let was = d.is_some();
                if let Some(mut d) = d {
                    d.stop();
                    if let Ok(root) = crate::app_root() {
                        let _ = std::fs::remove_file(crate::dev_json_path(&root));
                    }
                }
                Ok(Json::obj([("stopped", Json::Bool(was))]).stringify())
            })
        },
    });
    tools.push(Tool {
        name: "brain_logs",
        description: "Dev server log (HMR, compile errors, load failures) after sequence number `since`, at most 100 entries; `more: true` = call again with the last `seq`.",
        input_schema: schema([("since", "number", "last seq already seen (default 0)"), ("level", "string", "info | error")], &[]),
        output_schema: out([("logs", "array"), ("more", "boolean")]),
        run: {
            let st = st.clone();
            Box::new(move |a| {
                let dev = st.dev.lock().unwrap();
                let d = dev.as_ref().ok_or("the dev server is not running; fix: call `brain_run_start`")?;
                Ok(capped_items("logs", &d.logs(num(a, "since").unwrap_or(0.0) as u64, arg(a, "level").as_deref())))
            })
        },
    });
    tools.push(Tool {
        name: "brain_errors",
        description: "JavaScript errors with original component/file/line, generated location, bounded frames, causes, mapping precision and truncation flags after sequence number `since`, at most 100 entries and 256 KiB; `more: true` = call again with the last `seq`.",
        input_schema: schema([("since", "number", "last seq already seen (default 0)")], &[]),
        output_schema: out([("errors", "array"), ("more", "boolean")]),
        run: {
            let st = st.clone();
            Box::new(move |a| {
                let dev = st.dev.lock().unwrap();
                let d = dev.as_ref().ok_or("the dev server is not running; fix: call `brain_run_start`")?;
                Ok(capped_items("errors", &d.errors(num(a, "since").unwrap_or(0.0) as u64)))
            })
        },
    });
    #[cfg(unix)]
    {
        tools.push(Tool {
            name: "brain_navigate",
            description: "Opens a route of the managed dev server in the debug browser (headless Chrome, 127.0.0.1 only) and waits for the view to mount. Returns url, title, h1 and the error overlay text if any.",
            input_schema: schema([("route", "string", "route, e.g. /users/42")], &["route"]),
            output_schema: out([("url", "string"), ("title", "string"), ("h1", "string"), ("overlay", "string")]),
            run: {
                let st = st.clone();
                Box::new(move |a| {
                    let route = need(a, "route")?;
                    st.with_browser(|b| b.navigate(&route)).map(|j| j.stringify())
                })
            },
        });
        tools.push(Tool {
            name: "brain_click",
            description: "Real mouse click (CDP) on the first element matching `selector` in the debug browser page.",
            input_schema: schema([("selector", "string", "CSS selector")], &["selector"]),
            output_schema: out([("tag", "string"), ("text", "string"), ("x", "number"), ("y", "number")]),
            run: {
                let st = st.clone();
                Box::new(move |a| {
                    let sel = need(a, "selector")?;
                    st.with_browser(|b| b.click(&sel)).map(|j| j.stringify())
                })
            },
        });
        tools.push(Tool {
            name: "brain_type",
            description: "Focuses the element matching `selector`, clears it and types `text` like a keyboard would. Returns the resulting value.",
            input_schema: schema([("selector", "string", "CSS selector"), ("text", "string", "text to type")], &["selector", "text"]),
            output_schema: out([("value", "string")]),
            run: {
                let st = st.clone();
                Box::new(move |a| {
                    let (sel, text) = (need(a, "selector")?, need(a, "text")?);
                    st.with_browser(|b| b.type_text(&sel, &text)).map(|v| Json::obj([("value", v)]).stringify())
                })
            },
        });
        tools.push(Tool {
            name: "brain_mock",
            description: "Turns fake data on or off in the live debug page (#266): every api() answers deterministic data generated from its schema (same seed + method + URL → same JSON; lists get ids 1..n), with no backend. Dev only. `brain run --mock[=seed]` does the same from the start.",
            input_schema: schema([("on", "boolean", "true = fake data, false = the real API"), ("seed", "string", "any text; default \"brain\"")], &["on"]),
            output_schema: out([("mock", "boolean")]),
            run: {
                let st = st.clone();
                Box::new(move |a| {
                    let on = matches!(a.get("on"), Some(Json::Bool(true)));
                    let seed = a.get("seed").and_then(Json::as_str).unwrap_or("brain").to_string();
                    let expr = format!(
                        "Promise.all([import('/@brain/brain-mock.js'), import('/@brain/brain-data.js')]).then(([m, d]) => (d.http({{ mock: {} }}), {on}))",
                        if on { format!("m.mocker({})", Json::str(&seed).stringify()) } else { "null".into() }
                    );
                    st.with_browser(|b| b.eval(&expr)).map(|v| Json::obj([("mock", v)]).stringify())
                })
            },
        });
        tools.push(Tool {
            name: "brain_why",
            description: "CALL IT when the page shows a wrong value and no diagnostic explains the cause, or when a node updates more often than expected: it names the signal write and the file:line behind the change, so you fix the cause instead of guessing. For the element `selector` picks in the live debug page: its last updates (newest first, up to 10), each with what changed (text, attr:x, class:x, insert…) and the signal writes that caused it — signal or store field (`cart.total`), component, `file.brain:line` and the chain of drv between them. `[]` when it never updated. Dev only; open the page with brain_navigate first.",
            input_schema: schema([("selector", "string", "CSS selector of one element, e.g. `.cart .total`")], &["selector"]),
            output_schema: None,
            run: {
                let st = st.clone();
                Box::new(move |a| {
                    let sel = need(a, "selector")?;
                    let expr = format!("globalThis.__brainWhy ? globalThis.__brainWhy.why({}) : {{ error: 'brain-why is not loaded in this page; fix: open it from `brain run` (brain_run_start + brain_navigate)' }}", Json::str(&sel).stringify());
                    st.with_browser(|b| b.eval(&expr)).map(|v| v.stringify())
                })
            },
        });
        tools.push(Tool {
            name: "brain_eval",
            description: "Evaluates a JavaScript expression (await allowed) in the live debug page and returns its JSON value. Answers within 30 s; a modal dialog the page opens (e.g. \"Leave this page?\" of a dirty form) is dismissed and listed in `dialogs`. Dev only; no access to HttpOnly cookies.",
            input_schema: schema([("expr", "string", "expression, e.g. document.querySelector('h1').textContent")], &["expr"]),
            output_schema: out([("value", "object"), ("dialogs", "array")]),
            run: {
                let st = st.clone();
                Box::new(move |a| {
                    let expr = need(a, "expr")?;
                    st.with_browser(|b| {
                        let v = b.eval(&expr);
                        let dialogs = b.take_dialogs();
                        let v = v.map_err(|e| if dialogs.is_empty() { e } else { format!("{e} (dialogs: {})", dialogs.join("; ")) })?;
                        let mut o = vec![("value".to_string(), v)];
                        if !dialogs.is_empty() {
                            o.push(("dialogs".into(), Json::Arr(dialogs.iter().map(Json::str).collect())));
                        }
                        Ok(Json::Obj(o))
                    })
                    .map(|j| j.stringify())
                })
            },
        });
    }

    // ---------- build ----------
    tools.push(Tool {
        name: "brain_build",
        description: "Build production assets, prerendered pages and SEO into dist/. Returns conservative route budgets and a final-asset raw/gzip/Brotli ledger separating initial and deferred references; not measured network traffic.",
        input_schema: schema([("site", "string", "absolute origin for the sitemap, e.g. https://example.dev")], &[]),
        output_schema: out([("files", "array"), ("warnings", "array"), ("ledger", "array")]),
        run: Box::new(|a| {
            let r = crate::run_build(&crate::app_root()?, arg(a, "site"))?;
            Ok(crate::build_json(&r).stringify())
        }),
    });

    tools.push(Tool {
        name: "brain_backend_contract",
        description: "Discover experimental Rust/Postgres backend resources and jobs. Omit name for a small list; give one name for its complete schema, policy, transactional hooks and owned files. One literal schema drives validation and OpenAPI; do not invent routes or database APIs.",
        input_schema: schema([("name","string","resource or job name; omit to list")], &[]),
        output_schema: out([("schemaVersion","number")]),
        run: Box::new(|a| crate::backend::card(&crate::app_root()?,arg(a,"name").as_deref()).map(|v|v.to_string())),
    });
    tools.push(Tool {
        name: "brain_resource_add",
        description: "Create an experimental backend resource with schema.jsbrain, explicit default-admin policy.jsbrain, hooks.jsbrain and search.jsbrain. Fields: title:str! active:bool. Generates typed collection/detail clients, a canonical client contract, server service and repository. adminUi remains planned: no views, guards, controls or budget changes. Does not migrate Postgres or start services; inspect brain_backend_contract, review policy, then run brain backend migrate explicitly.",
        input_schema: schema([("name","string","lowercase identifier"),("fields","string","name:type pairs; str, num, bool, email, obj; ! means required")], &["name","fields"]),
        output_schema: out([("schemaVersion","number"),("files","array")]),
        run: Box::new(|a| backend_add(a,"resource")),
    });
    tools.push(Tool {
        name: "brain_job_add",
        description: "Create a typed durable job schema and server-only handler. Implement the handler before starting brain jobs work; delivery is at least once and effects must be idempotent by ctx.jobId. Transactional resource hooks can target this queue.",
        input_schema: schema([("name","string","lowercase job/queue identifier"),("fields","string","payload name:type pairs; required fields end with !")], &["name","fields"]),
        output_schema: out([("schemaVersion","number"),("files","array")]),
        run: Box::new(|a| backend_add(a,"job")),
    });
    tools.push(Tool {
        name: "brain_agent_setup",
        description: "Merge project-local Claude Code/Cursor MCP registrations, preserving other servers. With hooks:true install optional Claude Code PostToolUse edit feedback for brain/jsbrain/cssbrain files. Checks only, no tests/downloads/auto-fixes. Restart the client; registration is not proof of a live connection.",
        input_schema: schema([("hooks","boolean","install Claude Code edit feedback")], &[]),
        output_schema: out([("schemaVersion","number"),("files","array")]),
        run: Box::new(|a|crate::agent::setup(&crate::app_root()?,a.get("hooks")==Some(&Json::Bool(true))).map(|f|serde_json::json!({"schemaVersion":1,"files":f,"restartClient":true}).to_string())),
    });
    tools.push(Tool {
        name: "brain_changes",
        description: "Revision-checked multi-file changes with per-file agent leases. Read returns revisions; lease acquires bounded ownership; plan validates an isolated snapshot; apply rechecks revisions and commits atomic individual files with conditional rollback. Use JSON request for operations, owner, token and planId. No merge or publication.",
        input_schema: schema([("operation","string","read, lease, plan, apply, recover, status or discard"),("request","object","operation arguments; see docs/CHANGES.md")], &["operation","request"]),
        output_schema: out([("schemaVersion","number")]),
        run: Box::new(|a| {
            let input = a.get("request").ok_or("request is required")?;
            let input = serde_json::from_str(&input.stringify()).map_err(|e|format!("invalid request: {e}"))?;
            crate::changes::dispatch(&crate::app_root()?,&need(a,"operation")?,&input).map(|v|crate::versioned(v).to_string())
        }),
    });
    tools.push(Tool {
        name: "brain_intelligence",
        description: "Bounded static evidence: graph, task context, impact, why or preflight. Preflight reports real checker diagnostics with component/file/line/fix before execution. Declarations are not proof of runtime behavior; dynamic unknowns are explicit.",
        input_schema: schema([("operation","string","graph, context, impact, why or preflight"),("target","string","component, resource, file or route"),("task","string","task for context selection"),("cursor","number","graph offset"),("limit","number","graph page size"),("budget","number","context byte budget")], &["operation"]),
        output_schema: out([("schemaVersion","number")]),
        run: Box::new(|a| {
            let root = crate::app_root()?;
            let input: serde_json::Value=serde_json::from_str(&a.stringify()).map_err(|e|e.to_string())?;
            let target=arg(a,"target");
            let number=|key:&str,default:usize| input.get(key).and_then(|v|v.as_u64()).and_then(|v|usize::try_from(v).ok()).unwrap_or(default);
            let result=match need(a,"operation")?.as_str() {
                "graph"=>crate::intelligence::graph_page(&root,number("cursor",0),number("limit",100)),
                "context"=>crate::intelligence::context(&root,&arg(a,"task").unwrap_or_default(),number("budget",8192)),
                "impact"=>crate::intelligence::impact(&root,target.as_deref().ok_or("target is required")?,None),
                "why"=>crate::intelligence::why(&root,target.as_deref().ok_or("target is required")?),
                "preflight"=>crate::intelligence::preflight(&root,target.as_deref()),
                _=>Err("unknown intelligence operation".into()),
            }; result.map(|v|v.to_string())
        }),
    });
    tools.push(Tool {
        name: "brain_recipe",
        description: "List recipes or prepare schema-derived resource-ui/auth-ui/google-signin/password-reset/email-verification file operations. Preview only; acquire leases and pass operations through brain_changes plan/apply to validate and write. Experimental backend must be configured separately.",
        input_schema: schema([("name","string","omit to list; resource-ui, auth-ui, google-signin, password-reset or email-verification"),("params","object","resource and optional view")], &[]),
        output_schema: out([("schemaVersion","number")]),
        run: Box::new(|a| match arg(a,"name") {
            None=>Ok(crate::recipes::list().to_string()),
            Some(name)=>{
                let input=match a.get("params") {Some(v)=>serde_json::from_str(&v.stringify()).map_err(|e|e.to_string())?,None=>serde_json::json!({})};
                crate::recipes::generate(&crate::app_root()?,&name,&input,true).map(|v|v.to_string())
            }
        }),
    });
    tools.push(Tool {
        name: "brain_upgrade",
        description: "Preview an explicit exact framework package version. Returns revision-checked package.json operations and remaining migration/install decisions without writing, installing or contacting registries. Apply through brain_changes; this is not a completed app upgrade.",
        input_schema: schema([("version","string","explicit exact SemVer target")], &["version"]),
        output_schema: out([("schemaVersion","number"),("operations","array")]),
        run: Box::new(|a|crate::upgrade::preview(&crate::app_root()?,&need(a,"version")?).map(|v|v.to_string())),
    });
    tools.push(Tool {
        name: "brain_lib",
        description: "Discover built-in HTTP, validation, state, date, hash, format and environment capabilities without installs. operation=new creates one pure named-export project helper; no package or PR is published.",
        input_schema: schema([("operation","string","list (default) or new"),("name","string","helper name for new")], &[]),
        output_schema: out([("schemaVersion","number")]),
        run: Box::new(|a| match arg(a,"operation").as_deref().unwrap_or("list") {
            "list"=>Ok(crate::libraries::list().to_string()),
            "new"=>{
                let root=crate::app_root()?;let value=crate::libraries::new(&root,&need(a,"name")?)?;
                if let Some(files)=value["files"].as_array() {for file in files.iter().filter_map(serde_json::Value::as_str) {journal_record(&root,&root.join(file),None);}}
                Ok(value.to_string())
            },
            _=>Err("unknown library operation; use list or new".into()),
        }),
    });
    tools.push(Tool {
        name: "brain_search",
        description: "Native bounded source index for components, functions, props, imports and lexical references. No browser or whole-app compile. Returns exact token locations, content revisions and explicit resolution limits; page with cursor.",
        input_schema: schema([("operation","string","find, symbols or refs"),("request","object","query for find; file for symbols; name for refs; optional kind/limit/cursor/metrics")], &["operation","request"]),
        output_schema: out([("schemaVersion","number")]),
        run: Box::new(|a| {
            let input=serde_json::from_str(&a.get("request").ok_or("request is required")?.stringify()).map_err(|e|e.to_string())?;
            crate::search::query(&crate::app_root()?,&need(a,"operation")?,&input).map(|v|v.to_string())
        }),
    });
    tools.push(Tool {
        name: "brain_design",
        description: "Inspect the project-owned design baseline, check drift, preview reversible token adoption or generate a private searchable static component gallery. Existing tokens, fonts and app data are preserved; no app code executes.",
        input_schema: schema([("operation","string","query (default), check, init, gallery, blueprint, seed or configure"),("target","string","optional token name for query"),("options","object","guarded design adoption/seed/configuration request")], &[]),
        output_schema: out([("schemaVersion","number")]),
        run: Box::new(|a| {
            let root=crate::app_root()?;
            let options:serde_json::Value=serde_json::from_str(&a.get("options").map(Json::stringify).unwrap_or_else(||"{}".into())).map_err(|_|"design options must be JSON")?;
            let value=match arg(a,"operation").as_deref().unwrap_or("query") {
                "query"=>crate::design::query(&root,arg(a,"target").as_deref())?,
                "check"=>crate::design::check(&root)?,
                "init"=>crate::design::preview_init(&root,&options)?,
                "gallery"=>crate::design::gallery(&root)?,
                "blueprint"=>crate::design::blueprint(&root)?,
                "seed"=>crate::design::preview_seed(&root,&options)?,
                "configure"=>crate::design::preview_configure(&root,&options)?,
                _=>return Err("unknown design operation; use query, check, init or gallery".into()),
            }; Ok(value.to_string())
        }),
    });
    tools.push(Tool {
        name: "brain_manifest",
        description: "Read bounded private per-view/component contracts, revisions, dependency edges and structural UI states without executing source or reading secrets. Refresh updates only owned metadata.",
        input_schema: schema([("target","string","optional source file"),("refresh","boolean","refresh bounded private manifests")], &[]),
        output_schema: out([("schemaVersion","number")]),
        run: Box::new(|a| {
            let root=crate::app_root()?;
            if matches!(a.get("refresh"),Some(Json::Bool(true))) {crate::manifests::refresh(&root).map(|v|v.to_string())}
            else {crate::manifests::query(&root,arg(a,"target").as_deref()).map(|v|v.to_string())}
        }),
    });
    tools.push(Tool {
        name: "brain_history",
        description: "Project-local checklist, reported evidence, proposed lessons and safe incident metadata. Read operation defaults to status. Mutations require request.expectedRevision from the last query; no commands execute or reports publish.",
        input_schema: schema([("operation","string","status/tasks/task/evidence/lessons/incidents/timeline or task-create/task-update/lesson/incident"),("request","object","bounded operation fields and expectedRevision for edits"),("write","boolean","explicitly select mutation for evidence")], &[]),
        output_schema: out([("schemaVersion","number")]),
        run: Box::new(|a| {
            let root=crate::app_root()?;let op=arg(a,"operation").unwrap_or_else(||"status".into());
            let request=serde_json::from_str(&a.get("request").map(Json::stringify).unwrap_or_else(||"{}".into())).map_err(|_|"history request must be JSON")?;
            let write=matches!(op.as_str(),"task-create"|"task-update"|"lesson"|"incident") || a.get("write")==Some(&Json::Bool(true));
            if write {crate::history::edit(&root,&op,&request).map(|v|v.to_string())} else {crate::history::query(&root,&op,&request).map(|v|v.to_string())}
        }),
    });
    tools.push(Tool {
        name: "brain_integration",
        description: "Discover versioned official provider SDK adapters or preview selective installation/update as revision-checked source operations. Does not install, contact providers, invoke AI models or expose credentials.",
        input_schema: schema([("id","string","omit to list integrations"),("options","object","optional exact version, expectedRevision and includeTemplate")], &[]),
        output_schema: out([("schemaVersion","number")]),
        run: Box::new(|a| {
            if let Some(id)=arg(a,"id") {
                let request=serde_json::from_str(&a.get("options").map(Json::stringify).unwrap_or_else(||"{}".into())).map_err(|_|"integration options must be JSON")?;
                crate::integrations::preview(&crate::app_root()?,&id,&request).map(|v|v.to_string())
            } else {Ok(crate::integrations::catalog().to_string())}
        }),
    });
    tools.push(Tool {
        name: "brain_feedback",
        description: "Poll bounded project-local compiler events published before the browser error overlay. Resume with epoch/sequence; inspect reset/gap flags and exact source evidence. Does not spawn an agent or edit source. Managed brain_run also delivers live MCP notifications.",
        input_schema: schema([("options","object","afterEpoch, afterSequence and limit (1..128)")], &[]),
        output_schema: out([("schemaVersion","number")]),
        run: Box::new(|a| {
            let options=serde_json::from_str(&a.get("options").map(Json::stringify).unwrap_or_else(||"{}".into())).map_err(|_|"feedback options must be JSON")?;
            crate::feedback::query(&crate::app_root()?,&options).map(|v|v.to_string())
        }),
    });
    tools.push(Tool {
        name: "brain_quality",
        description: "Agent-first persistent quality feedback: bounded findings with evidence, exact known locations, priorities, fixes and acceptance criteria. Read JSON/Markdown/fixed panel; refresh collects static evidence without compiling or calling providers. Unknown coverage stays explicit.",
        input_schema: schema([("refresh","boolean","refresh local quality evidence"),("options","object","query filters and pagination")], &[]),
        output_schema: out([("schemaVersion","number")]),
        run: Box::new(|a| {
            let root=crate::app_root()?;
            if a.get("refresh")==Some(&Json::Bool(true)) {crate::quality::refresh(&root).map(|v|v.to_string())}
            else {let request=serde_json::from_str(&a.get("options").map(Json::stringify).unwrap_or_else(||"{}".into())).map_err(|_|"quality options must be JSON")?;crate::quality::query(&root,&request).map(|v|v.to_string())}
        }),
    });
    tools.push(Tool {
        name: "brain_deployment",
        description: "Prepare optional PM2/nginx deployment configuration as guarded file operations. No services are installed, activated or deployed. Explicit topology and production settings remain reviewable.",
        input_schema: schema([("target","string","pm2 or nginx"),("options","object","deployment topology and public settings; never secret values")], &["target","options"]),
        output_schema: out([("schemaVersion","number")]),
        run: Box::new(|a| {
            let request=serde_json::from_str(&a.get("options").ok_or("options are required")?.stringify()).map_err(|_|"deployment options must be JSON")?;
            crate::deployment::preview(&crate::app_root()?,&need(a,"target")?,&request).map(|v|v.to_string())
        }),
    });
    tools.push(Tool {
        name: "brain_e2e",
        description: "Probe the local browser extension connection and invocation contract, or run declared app E2E scenarios. Install Claude Viewer/Codex Viewer in the browser and enable its scripting bridge. Only newly owned test tabs are touched; reports preserve actual lane evidence.",
        input_schema: schema([("operation","string","probe (default) or run"),("path","string","optional declared .test.brain file or folder")], &[]),
        output_schema: out([("schemaVersion","number")]),
        run: Box::new(|a| {
            match arg(a,"operation").as_deref().unwrap_or("probe") {
                "probe" => return crate::viewer::probe().map(|v|v.to_string()),
                "run" => {},
                _ => return Err("E2E operation must be probe or run".into()),
            }
            #[cfg(unix)] {
                let reports=crate::run_tests(arg(a,"path").map(PathBuf::from),60)?;
                let failed=reports.iter().any(crate::testrun::FileReport::failed);
                let value=Json::obj([("schemaVersion",Json::Num(1.0)),("files",Json::Arr(reports.iter().map(crate::testrun::FileReport::to_json).collect()))]).stringify();
                if failed {Err(value)} else {Ok(value)}
            }
            #[cfg(not(unix))] {Err("E2E needs a supported browser runner".into())}
        }),
    });
    tools.push(Tool {
        name: "brain_dependencies",
        description: "Analyze installed browser ESM offline or preview a revision-checked optimization. Explicit fetch acquires one exact pinned HTTPS CDN ESM artifact with SHA-256 and lock CAS; it writes the reviewed local vendor/lock paths without executing code. Builds never fetch or call an LLM.",
        input_schema: schema([("operation","string","analyze (default), preview or explicit fetch"),("request","object","file for analyze; file/expectedRevision/content for preview; url/package/version/sha256/expectedRevision for explicit fetch")], &[]),
        output_schema: out([("schemaVersion","number")]),
        run: Box::new(|a| {
            let request=serde_json::from_str(&a.get("request").map(Json::stringify).unwrap_or_else(||"{}".into())).map_err(|_|"dependency request must be JSON")?;
            crate::dependencies::query(&crate::app_root()?,arg(a,"operation").as_deref().unwrap_or("analyze"),&request).map(|v|v.to_string())
        }),
    });
    if profile != "full" {
        tools.retain(|t| profile_tool(profile,t.name));
    }

    // ---------- resources ----------
    let resources = vec![
        Resource { uri: "brain://spec", name: "spec", description: "The BRAINLLM spec (what works today).", mime: "text/markdown", read: Box::new(|| Ok(spec::served())) },
        Resource {
            uri: "brain://map",
            name: "map",
            description: "Project graph as JSON (routes, components, modules, tests, tokens).",
            mime: "application/json",
            read: Box::new(|| Ok(crate::map::build(&crate::app_root()?).stringify())),
        },
        Resource {
            uri: "brain://errors",
            name: "errors",
            description: "Browser errors captured by the managed dev server.",
            mime: "application/json",
            read: {
                let st = st.clone();
                Box::new(move || Ok(st.dev.lock().unwrap().as_ref().map_or("[]".to_string(), |d| d.errors(0))))
            },
        },
        Resource {
            uri: "brain://doctor",
            name: "doctor",
            description: "Full project review (check + tests + accessibility).",
            mime: "application/json",
            read: Box::new(|| crate::run_doctor(None).map(|(_, j)| j.stringify())),
        },
    ];

    // ---------- prompts ----------
    let prompts = vec![
        Prompt {
            name: "new-view",
            description: "Create a new view end to end.",
            args: vec![("name", "view name, e.g. users", true), ("goal", "what the view must do", false)],
            get: Box::new(|a| {
                let name = need(a, "name")?;
                let goal = arg(a, "goal").unwrap_or_default();
                Ok(format!(
                    "Create the BRAINLLM view `{name}`. {goal}\n1. Read `brain_spec` (topics markup, css, a11y) and `brain_map`.\n2. Call `brain_add` with kind=view, name={name} and typed props.\n3. Fill in the template; import only files listed by `brain_map`.\n4. Loop `brain_check` → `brain_fix` → fix the rest by hand until there are no errors.\n5. `brain_run_start`, `brain_navigate` to the route and `brain_shot` to verify it renders.\n6. Finish with `brain_doctor` (must return ok)."
                ))
            }),
        },
        Prompt {
            name: "fix-all",
            description: "Bring the project to zero errors.",
            args: vec![],
            get: Box::new(|_| Ok("Run `brain_fix`, then `brain_check`. For each remaining diag apply its `fix` exactly, then check again. Repeat until there are no errors, then run `brain_doctor` and fix any failing test or accessibility issue.".into())),
        },
        Prompt {
            name: "debug-error",
            description: "Reproduce and fix a browser error captured by the dev server.",
            args: vec![("seq", "error sequence number from brain_errors", false)],
            get: Box::new(|a| {
                let seq = arg(a, "seq").map(|s| format!(" (seq {s})")).unwrap_or_default();
                Ok(format!(
                    "Debug the browser error{seq}.\n1. `brain_errors` to read message, stack and url.\n2. `brain_navigate` to that url; use `brain_click`/`brain_type` to reproduce the steps; `brain_eval` to inspect state.\n3. Open the component with `brain_read`, fix it, `brain_check`.\n4. Navigate again and confirm `brain_errors` shows nothing new, then `brain_doctor`."
                ))
            }),
        },
    ];

    let instructions = if profile == "full" { "BRAINLLM toolchain. Read brain_spec, brain_map and brain_guide before writing app code. Use brain_add, check/fix, browser verification and brain_doctor. Discover backend resources/jobs with brain_backend_contract; backend is experimental; explicitly start it and configure fixed loopback BRAIN_BACKEND_URL for native development/production identity and resources." } else { "BRAINLLM compact tool profile. Read brain_spec and brain_map; use CLI brain add to create files, check/fix until clean, then brain_test and brain_doctor. Backend profile also exposes resource/job generators and small backend contracts. No backend server is started by this MCP process." };
    let server = Server {
        name: "brain",
        version: env!("CARGO_PKG_VERSION"),
        instructions,
        tools,
        resources,
        prompts: if profile == "full" {prompts} else {prompts.into_iter().filter(|p|p.name=="fix-all").collect()},
    };
    let stdin = std::io::stdin();
    let r = server.run(stdin.lock(), &notifier).map_err(|e| e.to_string());
    if let Some(mut d) = st.dev.lock().unwrap().take() {
        d.stop();
    }
    r
}

/// Keep a stable six-tool minimal vocabulary; backend adds only its focused contract/generation tools.
fn profile_tool(profile:&str,name:&str)->bool {
    matches!(name,"brain_spec"|"brain_check"|"brain_fix"|"brain_map"|"brain_test"|"brain_doctor")
        || profile=="backend" && matches!(name,"brain_backend_contract"|"brain_resource_add"|"brain_job_add"|"brain_agent_setup")
}

fn backend_add(args:&Json,kind:&str)->Result<String,String> {
    let root=crate::app_root()?;
    let result=crate::backend::add(&root,kind,&need(args,"name")?,&need(args,"fields")?)?;
    if let Some(files)=result["files"].as_array() {
        for file in files.iter().filter_map(serde_json::Value::as_str) {journal_record(&root,&root.join(file),None);}
    }
    Ok(result.to_string())
}

#[cfg(test)]
mod mutation_tests {
    use super::*;
    #[test]
    fn file_mutations_and_tampered_undo_cannot_escape_the_app() {
        let root=std::env::temp_dir().join(format!("brain-mcp-boundary-{}-{}",std::process::id(),std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir_all(root.join("app")).unwrap();
        let app=root.join("app"); let outside=root.join("outside.txt");
        std::fs::write(&outside,"Retain this file").unwrap();
        assert!(writable(&app,outside.to_str().unwrap()).is_err());
        assert!(writable(&app,"../outside.txt").is_err());
        assert!(writable(&app,"secrets/private.txt").is_err());
        #[cfg(unix)] {
            std::os::unix::fs::symlink(&root,app.join("linked")).unwrap();
            assert!(writable(&app,"linked/outside.txt").is_err());
        }
        std::fs::create_dir_all(app.join(".brain")).unwrap();
        std::fs::write(journal_path(&app),Json::obj([("path",Json::str(outside.to_str().unwrap())),("before",Json::Null)]).stringify()).unwrap();
        assert!(journal_undo(&app,1).is_err());
        assert_eq!(std::fs::read_to_string(outside).unwrap(),"Retain this file");
        std::fs::remove_dir_all(root).unwrap();
    }
}
