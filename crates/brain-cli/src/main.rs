//! `brain`: the single BRAINLLM CLI. Arguments are parsed by hand (no dependencies).

#[cfg(unix)]
mod browser;
mod card;
mod brand;
mod impact;
#[cfg(unix)]
mod cdp;
mod compiler;
mod env;
mod icons;
pub(crate) mod map;
mod mcp;
mod names;
mod scaffold;
mod term;
mod spec;
#[cfg(unix)]
mod og;
#[cfg(unix)]
mod start;
#[cfg(unix)]
pub(crate) mod testrun;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

include!(concat!(env!("OUT_DIR"), "/runtime_files.rs"));

const USAGE: &str = "brain — the BRAINLLM toolchain (every command accepts --json)

main:
  brain --version                    prints the version (also -V, --json)
  brain seo <audit|links|preview R>  SEO audit of the built site (score per page, B18xx findings)
  brain new <app>                    creates the app (app.brain, tokens.cssbrain, views/, comp/, ui/, AGENTS.md)
  brain run [--port 5180] [--base /app/] [--mock[=seed]] [--profile] [--inspect-port N | --no-inspect]  dev server with hot reload; the debugger on port+1
  brain build [--site https://…] [--report] [--ext web|brain]   production (--report: bytes per page before/after each pass)
  brain doctor                       everything at once: check + tests + overengineering + leaks; exit ≠ 0 on any error
  brain start [--port 3000] [--instances N] [--install]   serves dist/ in production (static or server.mjs)
  brain stop | reload | status       control the running `brain start` (zero-downtime reload)

granular:
  brain add view|comp|ui <name> [--props \"a:num=0 b:str?\"]   (also layout, lib, collection <name>, locale <xx>)
  brain check [path]                 errors and warnings with an exact fix
  brain explain <code>               what a diagnostic code means, why, and a minimal correct example
  brain fix [path]                   applies the mechanical fixes and checks again
  brain test [path] [--timeout 60] [--heap]   *.test.brain in headless Chrome + accessibility audit (+ leaks)
  brain map                          project graph: routes, components and props, modules, tests, tokens
  brain spec [--topic X] [--all]     the spec for the LLM (topics: files script markup channels css a11y tests cli)
  brain schema                       JSON Schema of brain.json
  brain i18n --missing [--json]      per language: keys missing (exit 1) and texts still in the default language
  brain theme <preset> | --list      writes tokens.cssbrain from a preset (glass, dark, paper, contrast)
  brain card <comp>                  a component's contract (props, emits, slots, reads/writes, size, status)
  brain split <comp> <sel> <Name>    extract that subtree into a new component (refused if check fails)
  brain mcp                          the same commands as MCP tools over stdio
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // INTERNALS §25: a crash is a diagnosis too. Outside the MCP server (which must outlive a failing tool), an
    // internal panic prints where it happened and the next command to run, and exits with code 3.
    if args.first().map(String::as_str) != Some("mcp") {
        std::panic::set_hook(Box::new(|info| {
            let at = info.location().map(|l| format!(" at {}:{}", l.file(), l.line())).unwrap_or_default();
            let msg = info.payload().downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| info.payload().downcast_ref::<String>().cloned()).unwrap_or_default();
            eprintln!("error: internal brain error{at}: {msg}");
            eprintln!("  next: run `brain doctor` to see the app state; if it repeats, report it with the command and this message");
            std::process::exit(3);
        }));
    }
    let res = match args.first().map(String::as_str) {
        Some("--version") | Some("-V") | Some("version") => {
            let v = env!("CARGO_PKG_VERSION");
            if want_json(&args) {
                println!("{}", brain_mcp::Json::obj([("schemaVersion", brain_mcp::Json::Num(1.0)), ("name", brain_mcp::Json::str("brain")), ("version", brain_mcp::Json::str(v)), ("compilerVersion", brain_mcp::Json::str(brain_codegen::blocks::COMPILER_VERSION))]).stringify());
            } else {
                println!("brain {v}");
            }
            return ExitCode::SUCCESS;
        }
        Some("new") => cmd_new(&args[1..]),
        Some("add") => cmd_add(&args[1..]),
        Some("check") => return cmd_check(&args[1..]),
        // JSON Schema of brain.json (editor autocomplete/validation): `brain schema > brain.schema.json`
        // §27: the design tokens of the app from a preset (all text pairs pass AA; `contrast` passes AAA)
        Some("theme") => {
            let presets: Vec<(String, String)> = runtime_files().into_iter().filter_map(|(n, src)| n.strip_prefix("themes/").and_then(|x| x.strip_suffix(".cssbrain")).map(|x| (x.to_string(), src))).collect();
            match args.get(1).map(String::as_str) {
                None | Some("--list") => {
                    println!("{}", presets.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>().join("\n"));
                    Ok(())
                }
                Some(p) => match presets.iter().find(|(n, _)| n == p) {
                    Some((_, src)) => app_root().and_then(|r| std::fs::write(r.join("tokens.cssbrain"), src).map_err(|e| e.to_string())).map(|_| println!("tokens.cssbrain ← {p}")),
                    None => Err(format!("unknown theme `{p}`; fix: one of {}", presets.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>().join(", "))),
                },
            }
        }
        // §37: what each language still lacks — keys missing in a locale file (exit 1) and texts identical to the
        // default locale's (untranslated, listed only)
        Some("i18n") => app_root().and_then(|r| {
            let Some(i) = brain_check::i18n::load(&r) else { return Err("this app has one language; fix: `brain add locale es` adds another".into()) };
            let def = i.default_file().map(|f| f.entries.clone()).unwrap_or_default();
            let mut missing_any = false;
            let mut per: Vec<(String, brain_mcp::Json)> = Vec::new();
            let mut text = String::new();
            for l in i.locales.iter().filter(|l| **l != i.default) {
                let mine = i.file(l).map(|f| f.entries.clone()).unwrap_or_default();
                let missing: Vec<&String> = def.keys().filter(|k| !mine.contains_key(*k)).collect();
                let same: Vec<&String> = def.iter().filter(|(k, (v, _, _))| mine.get(*k).is_some_and(|(w, _, _)| w == v)).map(|(k, _)| k).collect();
                missing_any |= !missing.is_empty();
                text.push_str(&format!("{l}: {} missing, {} untranslated{}\n", missing.len(), same.len(), if missing.is_empty() && same.is_empty() { " ✓" } else { "" }));
                for k in &missing {
                    text.push_str(&format!("  missing  i18n/{l}.json  {k}\n"));
                }
                for k in &same {
                    text.push_str(&format!("  same as {}  i18n/{l}.json  {k}\n", i.default));
                }
                let arr = |v: &[&String]| brain_mcp::Json::Arr(v.iter().map(|k| brain_mcp::Json::str(k.as_str())).collect());
                per.push((l.clone(), brain_mcp::Json::obj([("missing", arr(&missing)), ("untranslated", arr(&same))])));
            }
            if want_json(&args) {
                println!("{}", brain_mcp::Json::obj([("default", brain_mcp::Json::str(&i.default)), ("locales", brain_mcp::Json::Obj(per))]).stringify());
            } else {
                print!("{text}");
            }
            if missing_any { Err("some locales lack keys; fix: add them (`brain fix` copies the default text) and translate".into()) } else { Ok(()) }
        }),
        Some("schema") => {
            print!("{}", include_str!("../schema/brain.schema.json"));
            Ok(())
        }
        Some("spec") => {
            let all = args.iter().any(|a| a == "--all");
            let text = match flag(&args[1..], "--topic") {
                Some(t) => spec::topic(&t, all),
                None => Ok(if all { spec::SPEC_ALL.to_string() } else { spec::served() }),
            };
            text.map(|t| {
                if want_json(&args) {
                    println!("{}", brain_mcp::Json::obj([("spec", brain_mcp::Json::str(t))]).stringify())
                } else {
                    print!("{t}")
                }
            })
        }
        // `dev` is a hidden alias of `run`: it is the name a model guesses from Vite/Next.
        Some("run") | Some("dev") => cmd_dev(&args[1..]),
        Some("doctor") => return cmd_doctor(&args[1..]),
        Some("fix") => return cmd_fix(&args[1..]),
        Some("explain") => cmd_explain(&args[1..]),
        // §43: one component's contract, and extracting a subtree into a new component
        Some("card") => match args.get(1) {
            Some(c) => app_root().and_then(|r| card::card(&r, c)).map(|j| println!("{}", j.stringify())),
            None => Err("usage: brain card <component>  (a name or file from `brain map`)".into()),
        },
        Some("split") => match (args.get(1), args.get(2), args.get(3)) {
            (Some(c), Some(sel), Some(name)) => app_root().and_then(|r| card::split(&r, c, sel, name, &mut |_, _| {})).map(|j| println!("{}", j.stringify())),
            _ => Err("usage: brain split <component> \"<selector>\" <NewName>  (e.g. brain split views/shop.brain \"li.row\" CartRow)".into()),
        },
        #[cfg(unix)]
        Some("start") => return cmd_start(&args[1..]),
        #[cfg(unix)]
        Some(c @ ("stop" | "reload" | "status")) => {
            let r = app_root().and_then(|root| start::send(&root, c));
            return match r {
                Ok(j) => {
                    let ok = !j.contains("\"ok\":false");
                    println!("{j}");
                    if ok { ExitCode::SUCCESS } else { ExitCode::FAILURE }
                }
                Err(e) => {
                    if want_json(&args) { println!("{}", error_json(&e)) } else { eprintln!("error: {e}") }
                    ExitCode::from(1)
                }
            };
        }
        Some("seo") => {
            // brain seo <audit|links|preview <route>|indexnow> [--json]: brain-seo over dist/
            let (code, out) = match app_root() {
                Ok(root) => brain_seo::cli(&root, &args[1..]),
                Err(e) => (2, e),
            };
            println!("{out}");
            return ExitCode::from(code.clamp(0, 255) as u8);
        }
        Some("mcp") => {
            // processes started from the MCP server (dev server, tests) print plain text: stdout is the protocol
            // SAFETY: set before any thread is spawned
            unsafe { std::env::set_var("BRAIN_MCP", "1") };
            mcp::run()
        }
        Some("test") => return cmd_test(&args[1..]),
        Some("map") => app_root().and_then(|r| {
            // filters and pages (§43): --view <route|file>, --comp <name>, --reads <key>, --limit N, --cursor N
            let num = |k: &str| flag(&args[1..], k).map(|v| v.parse::<usize>().map_err(|_| format!("{k} must be a number")));
            let q = map::Query { view: flag(&args[1..], "--view"), comp: flag(&args[1..], "--comp"), reads: flag(&args[1..], "--reads"), limit: num("--limit").transpose()?, cursor: num("--cursor").transpose()?.unwrap_or(0) };
            let plain = q.view.is_none() && q.comp.is_none() && q.reads.is_none() && q.limit.is_none() && q.cursor == 0;
            if args.iter().any(|a| a == "--json") {
                println!("{}", map::query(&r, &q).stringify())
            } else if plain {
                print!("{}", map::text(&map::build(&r)))
            } else {
                println!("{}", map::query(&r, &q).stringify())
            }
            Ok(())
        }),
        Some("build") => cmd_build(&args[1..]),
        Some("-h") | Some("--help") | Some("help") | None => {
            print!("{USAGE}");
            Ok(())
        }
        Some(other) => Err(format!("unknown command `{other}`; fix: run `brain help`")),
    };
    match res {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            if args.iter().any(|a| a == "--json") {
                println!("{}", error_json(&e));
            } else {
                eprintln!("error: {e}");
            }
            ExitCode::from(2)
        }
    }
}

/// `"<msg>; fix: <fix>"` → `{"error": msg, "fix": fix}` (same msg/fix pair as Diag).
pub(crate) fn error_json(e: &str) -> String {
    let (msg, fix) = e.split_once("; fix: ").unwrap_or((e, ""));
    brain_mcp::Json::obj([("error", brain_mcp::Json::str(msg)), ("fix", brain_mcp::Json::str(fix))]).stringify()
}

fn want_json(args: &[String]) -> bool {
    args.iter().any(|a| a == "--json")
}

/// Value of `--flag v` or `--flag=v`.
fn flag(args: &[String], name: &str) -> Option<String> {
    let eq = format!("{name}=");
    args.iter().enumerate().find_map(|(i, a)| {
        if a == name {
            args.get(i + 1).cloned()
        } else {
            a.strip_prefix(&eq).map(str::to_string)
        }
    })
}

/// Positional arguments (without flags or their values).
fn positional(args: &[String], valued: &[&str]) -> Vec<String> {
    let mut out = Vec::new();
    let mut skip = false;
    for a in args {
        if skip {
            skip = false;
        } else if a.starts_with("--") {
            skip = valued.contains(&a.as_str());
        } else {
            out.push(a.clone());
        }
    }
    out
}

pub(crate) fn cwd() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

pub(crate) fn app_root() -> Result<PathBuf, String> {
    scaffold::find_root(&cwd()).ok_or_else(|| "not inside a BRAINLLM app (tokens.cssbrain is missing); fix: `brain new <app>` or cd into its folder".into())
}

fn cmd_new(args: &[String]) -> Result<(), String> {
    let [name] = <[String; 1]>::try_from(positional(args, &[])).map_err(|_| "usage: brain new <app>".to_string())?;
    let dir = cwd().join(&name);
    let files = scaffold::new_app(&dir)?;
    let rels: Vec<String> = files.iter().map(|f| f.strip_prefix(&dir).unwrap_or(f).display().to_string()).collect();
    if want_json(args) {
        let j = brain_mcp::Json::obj([
            ("app", brain_mcp::Json::str(dir.display().to_string())),
            ("files", brain_mcp::Json::Arr(rels.iter().map(brain_mcp::Json::str).collect())),
        ]);
        println!("{}", j.stringify());
        return Ok(());
    }
    for r in rels {
        println!("+ {r}");
    }
    println!("\ndone: cd {name} && brain run");
    Ok(())
}

fn cmd_add(args: &[String]) -> Result<(), String> {
    let pos = positional(args, &["--props"]);
    let [kind, name] = <[String; 2]>::try_from(pos).map_err(|_| "usage: brain add view|comp|ui <name> [--props \"...\"]".to_string())?;
    let root = app_root()?;
    let p = scaffold::add(&root, &kind, &name, &flag(args, "--props").unwrap_or_default())?;
    let rel = p.strip_prefix(&root).unwrap_or(&p).display().to_string();
    if want_json(args) {
        println!("{}", brain_mcp::Json::obj([("file", brain_mcp::Json::str(rel))]).stringify());
    } else {
        println!("+ {rel}");
    }
    Ok(())
}

fn brain_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = rd.flatten().map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        let name = p.file_name().unwrap_or_default().to_string_lossy();
        if name.starts_with('.') || name == "dist" || name == "node_modules" {
            continue;
        }
        if p.is_dir() {
            brain_files(&p, out);
        } else if p.extension().is_some_and(|e| e == "brain") {
            out.push(p);
        }
    }
}

/// Compiles every `.brain` under `target` (file or folder; defaults to the app root).
/// Compiles every `.brain` under `target` (file or folder; defaults to the app root).
/// `Diag.file` is relative to the outermost root (highest ancestor with `tokens.cssbrain`); when walking a
/// folder, sub-apps (subfolders with their own `tokens.cssbrain`) are skipped and checked separately.
pub(crate) fn run_check(target: Option<PathBuf>) -> Result<(usize, Vec<compiler::Diag>), String> {
    run_check_base(target).map(|(n, d, _)| (n, d))
}

/// Like `run_check`, plus the base folder that `Diag.file` is relative to.
pub(crate) fn run_check_base(target: Option<PathBuf>) -> Result<(usize, Vec<compiler::Diag>, PathBuf), String> {
    let target = match target {
        Some(t) => std::path::absolute(&t).map_err(|e| e.to_string())?,
        None => app_root()?,
    };
    let mut files = Vec::new();
    if target.is_file() {
        files.push(target.clone())
    } else if target.is_dir() {
        brain_files_app(&target, &target, &mut files)
    } else {
        return Err(format!("`{}` does not exist; fix: pass the path of a .brain file or of the app", target.display()));
    }
    let base = scaffold::outer_root(&target).unwrap_or_else(|| if target.is_file() { target.parent().unwrap_or(Path::new(".")).to_path_buf() } else { target.clone() });
    let mut diags = Vec::new();
    for f in &files {
        let rel = f.strip_prefix(&base).unwrap_or(f);
        let src = std::fs::read_to_string(f).map_err(|e| format!("{}: {e}", f.display()))?;
        diags.extend(compiler::check(&src, rel, f).0);
    }
    // the app's .brain_env (B0724 malformed line, B0723 secret-shaped PUBLIC_*): keys only, never values
    if target.is_dir() {
        if let Some(app) = scaffold::find_root(&target) {
            diags.extend(env::check(&app));
            // the language files, once per app (B1401 invalid JSON, B1402 untranslated, B1404 placeholders, B1405 locale)
            if let Some(i18n) = brain_check::i18n::load(&app) {
                diags.extend(brain_check::i18n::check_locales(&i18n));
            }
            // content collections (§33): broken frontmatter, missing title/description… with file:line (B1823/B1824)
            if app.join("content").is_dir() {
                for p in brain_seo::content::load(&app, true).1 {
                    let mut d = compiler::Diag::new(p.code, &p.file, brain_parse::Span { line: p.line as u32, col: 1, ..Default::default() }, p.msg, p.fix);
                    d.level = if p.error { brain_parse::Severity::Error } else { brain_parse::Severity::Warning };
                    diags.push(d);
                }
            }
        }
    }
    if let Some(app) = scaffold::find_root(&target) {
        apply_lint(&app, &mut diags);
    }
    track_stuck(&base, &diags);
    Ok((files.len(), diags, base))
}

/// `brain.json` → `"lint": {<code>: "off" | "info" | "warning" | "error"}` (INTERNALS §20): the quality families
/// (B09xx loops, B10xx dead code, B11xx async, B12xx compatibility, B13xx practices) can be lowered or turned off;
/// encapsulation, accessibility, SEO, security, channels and every compile error cannot (an override there is ignored).
fn apply_lint(app: &Path, diags: &mut Vec<compiler::Diag>) {
    let Some(rules) = lint_rules(app) else { return };
    diags.retain_mut(|d| {
        let Some((_, lvl)) = rules.iter().find(|(c, _)| c == d.code) else { return true };
        if !lint_adjustable(d.code) {
            return true;
        }
        d.level = match lvl.as_str() {
            "off" => return false,
            "info" => brain_parse::Severity::Info,
            "warning" => brain_parse::Severity::Warning,
            _ => brain_parse::Severity::Error,
        };
        true
    });
}

pub(crate) fn lint_rules(app: &Path) -> Option<Vec<(String, String)>> {
    let j = brain_mcp::Json::parse(&std::fs::read_to_string(app.join("brain.json")).ok()?).ok()?;
    match j.get("lint")? {
        brain_mcp::Json::Obj(m) => Some(m.iter().filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string()))).collect()),
        _ => None,
    }
}

fn lint_adjustable(code: &str) -> bool {
    matches!(code.get(..3), Some("B09" | "B10" | "B11" | "B12" | "B13"))
}

/// Stuck-loop protection (INTERNALS §25): how many fix attempts each diagnostic (code + place) has survived. An attempt
/// is a check after its file changed: re-running check on an untouched file (CI, fixtures) is not a failed fix.
/// Stored in `.brain/stuck.json` as `{key: [count, file hash]}`; a diagnostic that disappears is forgotten.
fn stuck_key(d: &compiler::Diag) -> String {
    format!("{}|{}|{}|{}", d.code, d.file, d.line, d.col)
}

fn track_stuck(root: &Path, diags: &[compiler::Diag]) {
    use brain_mcp::Json;
    let file = root.join(".brain/stuck.json");
    let prev = std::fs::read_to_string(&file).ok().and_then(|s| Json::parse(&s).ok()).unwrap_or(Json::Obj(vec![]));
    let mut kv: Vec<(String, Json)> = Vec::new();
    for d in diags.iter().filter(|d| !d.is_warning()) {
        let k = stuck_key(d);
        if kv.iter().any(|(x, _)| *x == k) {
            continue;
        }
        let hash = std::fs::read(root.join(&d.file)).map(|b| brain_pack::content_hash(&String::from_utf8_lossy(&b))).unwrap_or_default();
        let n = match prev.get(&k) {
            // same bytes as last time: nobody tried a fix, the count stays
            Some(Json::Arr(a)) if a.get(1).and_then(Json::as_str) == Some(hash.as_str()) => a.first().and_then(|n| if let Json::Num(n) = n { Some(*n) } else { None }).unwrap_or(1.0),
            Some(Json::Arr(a)) => a.first().and_then(|n| if let Json::Num(n) = n { Some(*n) } else { None }).unwrap_or(0.0) + 1.0,
            Some(Json::Num(n)) => *n + 1.0,
            _ => 1.0,
        };
        kv.push((k, Json::Arr(vec![Json::Num(n), Json::str(hash)])));
    }
    let _ = std::fs::create_dir_all(root.join(".brain"));
    let _ = std::fs::write(&file, Json::Obj(kv).stringify());
}

/// Fix attempts recorded for `d` (`[count, hash]`; a bare number from older state files).
fn stuck_n(counts: &brain_mcp::Json, d: &compiler::Diag) -> f64 {
    use brain_mcp::Json;
    match counts.get(&stuck_key(d)) {
        Some(Json::Num(n)) => *n,
        Some(Json::Arr(a)) => match a.first() {
            Some(Json::Num(n)) => *n,
            _ => 0.0,
        },
        _ => 0.0,
    }
}

/// A diagnostic is stuck when it survived 3 consecutive checks at the same place (= 2 rounds of fixing failed).
fn stuck_counts() -> brain_mcp::Json {
    app_root()
        .ok()
        .and_then(|r| std::fs::read_to_string(r.join(".brain/stuck.json")).ok())
        .and_then(|s| brain_mcp::Json::parse(&s).ok())
        .unwrap_or(brain_mcp::Json::Obj(vec![]))
}

/// `brain fix`: applies the `Diag.edits` (mechanical fixes from COMPILER), checks again and repeats until
/// nothing applicable is left (max. 10 rounds). Per file it applies non-overlapping edits together; if they overlap,
/// it applies the first Diag's and leaves the rest for the next round.
pub(crate) fn run_fix(target: Option<PathBuf>) -> Result<brain_mcp::Json, String> {
    run_fix_with(target, &mut |_, _| {})
}

/// `run_fix` that reports every file it is about to overwrite (path, previous content) — the MCP journal uses it.
pub(crate) fn run_fix_with(target: Option<PathBuf>, before_write: &mut dyn FnMut(&Path, &str)) -> Result<brain_mcp::Json, String> {
    run_fix_only(target, None, before_write)
}

/// `run_fix_with` limited to the diagnostics whose code is in `only` (`None` = every code).
pub(crate) fn run_fix_only(target: Option<PathBuf>, only: Option<&[&str]>, before_write: &mut dyn FnMut(&Path, &str)) -> Result<brain_mcp::Json, String> {
    use brain_mcp::Json;
    let mut applied: Vec<Json> = Vec::new();
    for _round in 0..10 {
        let (_, diags, base) = run_check_base(target.clone())?;
        let mut by_file: Vec<(String, Vec<&compiler::Diag>)> = Vec::new();
        for d in diags.iter().filter(|d| !d.edits.is_empty() && only.is_none_or(|o| o.contains(&d.code))) {
            match by_file.iter_mut().find(|(f, _)| *f == d.file) {
                Some((_, v)) => v.push(d),
                None => by_file.push((d.file.clone(), vec![d])),
            }
        }
        if by_file.is_empty() {
            break;
        }
        let mut changed = false;
        for (file, ds) in by_file {
            let path = base.join(&file);
            // an edit may create its file (B1401: a declared locale without i18n/<loc>.json)
            let src = match std::fs::read_to_string(&path) {
                Ok(s) => s,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
                Err(e) => return Err(format!("{}: {e}", path.display())),
            };
            let all: Vec<_> = ds.iter().flat_map(|d| d.edits.iter().cloned()).collect();
            let (out, used): (Option<String>, Vec<&&compiler::Diag>) = match compiler::Diag::apply(&src, &all) {
                Some(o) => (Some(o), ds.iter().collect()),
                None => (compiler::Diag::apply(&src, &ds[0].edits), vec![&ds[0]]),
            };
            let Some(out) = out else { continue };
            if out != src {
                before_write(&path, &src);
                if let Some(d) = path.parent() {
                    std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
                }
                std::fs::write(&path, &out).map_err(|e| format!("{}: {e}", path.display()))?;
                changed = true;
                for d in used {
                    applied.push(Json::obj([
                        ("code", Json::str(d.code)),
                        ("file", Json::str(&d.file)),
                        ("line", Json::Num(d.line as f64)),
                        ("fix", Json::str(&d.fix)),
                    ]));
                }
            }
        }
        if !changed {
            break;
        }
    }
    let (_, remaining) = run_check(target)?;
    let errors = remaining.iter().filter(|d| !d.is_warning()).count();
    Ok(Json::obj([
        ("applied", Json::Arr(applied)),
        ("errors", Json::Num(errors as f64)),
        ("remaining", Json::parse(&diags_json(&remaining)).unwrap_or(Json::Arr(vec![]))),
    ]))
}

fn cmd_fix(args: &[String]) -> ExitCode {
    match run_fix(positional(args, &[]).first().map(PathBuf::from)) {
        Err(e) => {
            if want_json(args) { println!("{}", error_json(&e)) } else { eprintln!("error: {e}") }
            ExitCode::from(2)
        }
        Ok(j) => {
            let errors = match j.get("errors") { Some(brain_mcp::Json::Num(n)) => *n as usize, _ => 0 };
            if want_json(args) {
                println!("{}", j.stringify());
            } else {
                if let Some(brain_mcp::Json::Arr(a)) = j.get("applied") {
                    for d in a {
                        let s = |k: &str| d.get(k).and_then(brain_mcp::Json::as_str).unwrap_or("").to_string();
                        println!("✓ {} {} — {}", s("code"), s("file"), s("fix"));
                    }
                    eprintln!("{} fix(es) applied; {errors} error(s) left{}", a.len(), if errors > 0 { ": fix them with the `fix` from `brain check`" } else { "" });
                }
            }
            if errors == 0 { ExitCode::SUCCESS } else { ExitCode::FAILURE }
        }
    }
}

/// Like `brain_files`, but does not descend into sub-apps (folders with their own `tokens.cssbrain`).
fn brain_files_app(top: &Path, dir: &Path, out: &mut Vec<PathBuf>) {
    if dir != top && scaffold::is_app_dir(dir) {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = rd.flatten().map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        let name = p.file_name().unwrap_or_default().to_string_lossy();
        if name.starts_with('.') || name == "dist" || name == "node_modules" {
            continue;
        }
        if p.is_dir() {
            brain_files_app(top, &p, out);
        } else if p.extension().is_some_and(|e| e == "brain" || e == "jsbrain" || e == "cssbrain") && !name.ends_with(".test.brain") {
            out.push(p);
        }
    }
}

/// Diagnostics as JSON; a stuck one also carries `stuck: true` and `explain` (title, why, minimal correct example),
/// so the agent gets new information instead of the same message again.
pub(crate) fn diags_json(diags: &[compiler::Diag]) -> String {
    use brain_mcp::Json;
    let counts = stuck_counts();
    let Ok(Json::Arr(items)) = Json::parse(&compiler::Diag::list_to_json(diags)) else { return compiler::Diag::list_to_json(diags) };
    let out: Vec<Json> = items
        .into_iter()
        .zip(diags)
        .map(|(j, d)| {
            let stuck = stuck_n(&counts, d) >= 3.0;
            match (j, stuck) {
                (Json::Obj(mut kv), true) => {
                    kv.push(("stuck".into(), Json::Bool(true)));
                    if let Ok(e) = explain_json(d.code) {
                        kv.push(("explain".into(), e));
                    }
                    Json::Obj(kv)
                }
                (j, _) => j,
            }
        })
        .collect();
    Json::Arr(out).stringify()
}

fn cmd_check(args: &[String]) -> ExitCode {
    let identity = brand::Session::start(args, "Checking your frontend against its contract.");
    let json = args.iter().any(|a| a == "--json");
    let (n, diags) = match run_check(positional(args, &[]).first().map(PathBuf::from)) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(2);
        }
    };
    if json {
        println!("{}", diags_json(&diags));
    } else {
        let counts = stuck_counts();
        for d in &diags {
            let kind = if d.is_warning() { "warning" } else { "error" };
            println!("{}:{}:{} {kind} {} {}\n  fix: {}", d.file, d.line, d.col, d.code, d.msg, d.fix);
            if stuck_n(&counts, d) >= 3.0 {
                if let Ok(e) = explain_json(d.code) {
                    let s = |k: &str| e.get(k).and_then(brain_mcp::Json::as_str).unwrap_or("").to_string();
                    println!("  stuck: same error here for 3 checks. why: {}\n  correct example: {}", s("why"), s("example"));
                }
            }
        }
        let w = diags.iter().filter(|d| d.is_warning()).count();
        eprintln!("{n} file(s), {} error(s), {w} warning(s)", diags.len() - w);
    }
    identity.finish(if diags.iter().all(|d| d.is_warning()) { "Check complete. Ready for your next idea." } else { "Issues found. Review the diagnostics and suggested fixes." });
    // Warnings (today B0201) are reported but do not fail.
    if diags.iter().all(|d| d.is_warning()) { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}

pub(crate) struct BuildReport {
    pub files: Vec<String>,
    pub warnings: Vec<String>,
    /// route → [js raw, js min, js gz, css, css gz, html, html gz]
    pub report: Vec<(String, [usize; 7])>,
    /// dist files written by this build / left untouched because their bytes did not change
    pub written: usize,
    pub unchanged: usize,
    /// build signature (`build <sig>`, also in --json)
    pub signature: String,
    /// lowest and highest SEO score of the indexable pages
    pub seo: Option<(u32, u32)>,
}

/// `brain build` (today: sitemap.xml + robots.txt; bundling, minification and prerender arrive with #9/#26).
pub(crate) fn run_build(root: &Path, site: Option<String>) -> Result<BuildReport, String> {
    run_build_with(root, site, "production", None)
}

/// `run_build` reporting its phases to the terminal view (`brain build`; MCP and tests pass None).
pub(crate) fn run_build_with(root: &Path, site: Option<String>, env_name: &str, progress: Option<&term::Progress>) -> Result<BuildReport, String> {
    // environment (§36): PUBLIC_* become constants; a key with no value in it stops the build (B0720)
    env::select(root, env_name);
    // any error in the file stops the build too (B0724 malformed line, B0723 secret-shaped PUBLIC_*)
    let mut empty: Vec<compiler::Diag> = env::check(root).into_iter().filter(|d| !d.is_warning()).collect();
    empty.extend(env::empty(root, env_name));
    if !empty.is_empty() {
        return Err(empty.iter().map(|d| format!("{} {}:{}:{} {}; fix: {}", d.code, d.file, d.line, d.col, d.msg, d.fix)).collect::<Vec<_>>().join("\n"));
    }
    // the site URL: --site > PUBLIC_SITE_URL of the environment > brain.json "site" (legacy)
    let site = site.or_else(|| env::value(root, env_name, "PUBLIC_SITE_URL")).or_else(|| {
        let j = brain_mcp::Json::parse(&std::fs::read_to_string(root.join("brain.json")).ok()?).ok()?;
        j.get("site")?.as_str().filter(|s| !s.is_empty()).map(str::to_string)
    });
    if let Some(s) = &site
        && !(s.starts_with("https://") || s.starts_with("http://"))
    {
        return Err(format!("`--site {s}` must be absolute; fix: `--site https://{s}`"));
    }
    let mut rep = BuildReport { files: vec![], warnings: vec![], report: vec![], written: 0, unchanged: 0, signature: String::new(), seo: None };
    // the native linter (INTERNALS §20): the same diagnostics as `brain check`; errors stop the build with a summary,
    // warnings are listed with their count (info stays in `brain check`)
    if let Some(p) = progress {
        p.event("check", 0, 0)
    }
    let (_, diags, _) = run_check_base(Some(root.to_path_buf()))?;
    let line = |d: &compiler::Diag| format!("{} {}:{}:{} {}; fix: {}", d.code, d.file, d.line, d.col, d.msg, d.fix);
    let errors: Vec<String> = diags.iter().filter(|d| !d.is_warning()).map(line).collect();
    let warns = diags.iter().filter(|d| d.level == brain_parse::Severity::Warning).count();
    if !errors.is_empty() {
        return Err(format!("{}\nbuild stopped: {} error(s), {warns} warning(s); fix them (`brain check` lists the same) and build again", errors.join("\n"), errors.len()));
    }
    rep.warnings.extend(diags.iter().filter(|d| d.level == brain_parse::Severity::Warning).map(line));
    let dist = root.join("dist");
    std::fs::create_dir_all(&dist).map_err(|e| e.to_string())?;
    // robots.txt, sitemap.xml, llms.txt, manifest and the head rewrite come from brain-seo's finish (inside bundle)
    bundle(root, &dist, site.as_deref(), env_name, &mut rep, progress)?;
    Ok(rep)
}

/// Production hash length per file: 4 base36 chars, 5 for every file whose 4-char hash collides with another's.
fn hash_lengths(root: &Path) -> HashMap<String, usize> {
    let mut files = Vec::new();
    brain_files(root, &mut files);
    let rels: Vec<String> = files.iter().map(|f| f.strip_prefix(root).unwrap_or(f).to_string_lossy().replace('\\', "/")).collect();
    let mut by_hash: HashMap<String, Vec<&String>> = HashMap::new();
    for r in &rels {
        by_hash.entry(brain_codegen::component_hash_len(r, 4)).or_default().push(r);
    }
    rels.iter().map(|r| (r.clone(), if by_hash[&brain_codegen::component_hash_len(r, 4)].len() > 1 { 5 } else { 4 })).collect()
}

/// `views/a/b.brain` → its `_layout.brain` chain from the outside in (the build's route table).
pub(crate) fn layouts_of(root: &Path, view: &str) -> Vec<String> {
    brain_pack::routes::layouts_of(root, view)
}

/// Parallel cold start (INTERNALS §28): compiles every `.brain`/`.jsbrain` on `std::thread` workers into the
/// persistent cache, so the bundler's sequential graph walk only reads cache hits. Errors are ignored here: the
/// bundler reports them when it reaches the file.
fn prewarm(root: &Path, lens: &HashMap<String, usize>) {
    let mut files = Vec::new();
    brain_files(root, &mut files);
    let jobs: Vec<(PathBuf, String, usize)> = files
        .into_iter()
        .filter(|f| !f.to_string_lossy().ends_with(".test.brain"))
        .map(|f| {
            let rel = f.strip_prefix(root).unwrap_or(&f).to_string_lossy().replace('\\', "/");
            let len = lens.get(&rel).copied().unwrap_or(4);
            (f, rel, len)
        })
        .collect();
    let workers = std::thread::available_parallelism().map_or(4, |n| n.get()).min(jobs.len().max(1));
    let next = std::sync::atomic::AtomicUsize::new(0);
    std::thread::scope(|sc| {
        for _ in 0..workers {
            sc.spawn(|| loop {
                let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let Some((f, rel, len)) = jobs.get(i) else { break };
                if let Ok(src) = std::fs::read_to_string(f) {
                    let _ = compiler::compile_hash(&src, Path::new(rel), root, false, *len);
                }
            });
        }
    });
}

/// Bundles the app into `dist/` with brain-pack: minified chunks with content hashes, CSS per view, one page per
/// static route, the runtime core with its short export names (`RUNTIME_MAP`).
/// `brain.json` → `"cache": { "http", "sw", "offline", "precacheKB" }` (INTERNALS §35); everything on by default.
pub(crate) fn cache_cfg(root: &Path) -> Result<brain_pack::cache::CacheCfg, String> {
    use brain_mcp::Json;
    let mut c = brain_pack::cache::CacheCfg::default();
    let Ok(src) = std::fs::read_to_string(root.join("brain.json")) else { return Ok(c) };
    let j = Json::parse(&src).map_err(|e| format!("brain.json: {e}; fix: make it valid JSON"))?;
    let Some(cache) = j.get("cache") else { return Ok(c) };
    let flag = |k: &str, d: bool| -> Result<bool, String> {
        match cache.get(k) {
            None => Ok(d),
            Some(Json::Bool(b)) => Ok(*b),
            Some(_) => Err(format!("brain.json: cache.{k} must be true or false")),
        }
    };
    c.http = flag("http", true)?;
    c.sw = flag("sw", true)?;
    c.offline = flag("offline", true)?;
    c.data = flag("data", true)?;
    match cache.get("precacheKB") {
        None => {}
        Some(Json::Num(n)) if *n >= 0.0 => c.precache_kb = *n as usize,
        Some(_) => return Err("brain.json: cache.precacheKB must be a number of KB (default 2048)".into()),
    }
    Ok(c)
}

/// Returns (route, HTML) of every built page.
fn bundle(root: &Path, dist: &Path, site: Option<&str>, env_name: &str, rep: &mut BuildReport, progress: Option<&term::Progress>) -> Result<Vec<(String, String)>, String> {
    let on_phase = |phase: &str, done: usize, total: usize| {
        if let Some(p) = progress {
            p.event(phase, done, total)
        }
    };
    let rt = runtime_files();
    let shim = rt.iter().find(|(n, _)| n == "shim/dom.js").map(|(_, b)| b.clone());
    let core_src = rt.iter().find(|(n, _)| n == "brain.js").map(|(_, b)| b.clone()).ok_or("runtime brain.js is not embedded")?;
    let core = brain_min::minify_runtime_core(&core_src)?;
    let siblings: Vec<(String, String)> =
        rt.into_iter()
            // runtime modules, plus the built-in ui/ kit (compiled by brain-pack like an app component) and RUNTIME's
            // runaway probe for the build (shim/probe.js, B0907; never served)
            .filter(|(n, _)| (n.ends_with(".js") && !n.contains('/') && n != "brain-dev.js" && n != "brain-inspect.js" && n != "brain-why.js" && n != "brain-mock.js" && n != "brain-overlay.js") || (n.starts_with("ui/") && n.ends_with(".brain")) || n == "shim/probe.js")
            .collect();
    on_phase("check", 0, 0);
    let lens = hash_lengths(root);
    prewarm(root, &lens);
    let r2 = root.to_path_buf();
    let compile = move |src: &str, rel: &Path| -> Result<brain_pack::Unit, String> {
        let key = rel.to_string_lossy().replace('\\', "/");
        let len = lens.get(&key).copied().unwrap_or(4);
        let o = compiler::compile_hash(src, rel, &r2, false, len).map_err(|ds| {
            format!("{key} does not compile: {}; fix: run `brain check`", ds.iter().map(|d| format!("{} {}", d.code, d.msg)).collect::<Vec<_>>().join("; "))
        })?;
        Ok(brain_pack::Unit { js: o.js, css: o.css, server_js: o.server_js, server_fns: o.server_fns })
    };
    // Server modules for build-time evaluation (paths, load on paths() routes, seo, head): server_js with its
    // relative imports pointing at the server modules of the files it imports, under .brain/build/server/.
    fn server_module(root: &Path, rel: &str, compile: &dyn Fn(&str, &Path) -> Result<brain_pack::Unit, String>, depth: u32) -> Result<(String, String), String> {
        if depth > 32 {
            return Err(format!("server imports nested too deep (cycle?) in {rel}"));
        }
        let disk = root.join(rel);
        let code = if rel.ends_with(".js") {
            std::fs::read_to_string(&disk).map_err(|e| format!("{rel}: {e}"))?
        } else {
            let src = std::fs::read_to_string(&disk).map_err(|e| format!("{rel}: {e}"))?;
            let u = compile(&src, Path::new(rel))?;
            if u.server_js.trim().is_empty() { u.js } else { u.server_js }
        };
        let dir = disk.parent().unwrap_or(root).to_path_buf();
        let code = brain_pack::rewrite_specifiers(&code, &dir, &mut |abs| {
            let abs = abs.canonicalize().map_err(|e| format!("{}: {e}", abs.display()))?;
            let root = root.canonicalize().unwrap_or(root.to_path_buf());
            match abs.strip_prefix(&root) {
                Ok(r) if abs.extension().is_some_and(|e| e == "brain" || e == "jsbrain") => server_module(&root, &r.to_string_lossy().replace('\\', "/"), compile, depth + 1).map(|(u, _)| u),
                _ => Ok(brain_serve::load::file_url(&abs)),
            }
        })?;
        // runtime imports (`/@brain/brain-ch.js`, a client module evaluated at build time) → the embedded runtime
        // written next to the server modules, so Node can load them
        let code = brain_pack::rewrite_imports(&code, &mut |spec| {
            // content collections (§33): written once per build by `bundle` (write_content_module)
            if spec == "brain:content" {
                return Ok(Some(brain_serve::load::file_url(&root.join(".brain/build/server/_rt/content.mjs"))));
            }
            let Some(name) = spec.strip_prefix("/@brain/") else { return Ok(None) };
            let rt = runtime_files();
            if !rt.iter().any(|(n, _)| n == name) {
                return Ok(None);
            }
            // the runtime modules import each other (`./brain.js`): write them all side by side
            let dir = root.join(".brain/build/server/_rt");
            for (n, src) in rt.iter().filter(|(n, _)| n.ends_with(".js") && !n.contains('/')) {
                let f = dir.join(n);
                if std::fs::read_to_string(&f).ok().as_deref() != Some(src.as_str()) {
                    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                    std::fs::write(&f, src).map_err(|e| e.to_string())?;
                }
            }
            Ok(Some(brain_serve::load::file_url(&dir.join(name))))
        })?;
        let out = root.join(".brain/build/server").join(format!("{rel}.mjs"));
        std::fs::create_dir_all(out.parent().unwrap()).map_err(|e| e.to_string())?;
        if std::fs::read_to_string(&out).ok().as_deref() != Some(code.as_str()) {
            std::fs::write(&out, &code).map_err(|e| e.to_string())?;
        }
        Ok((brain_serve::load::file_url(&out), code))
    }
    // `brain:content` for paths()/load()/seo() at build time: the collections of content/, drafts only in staging
    {
        let cfg = brain_seo::Config::load(root)?.with_env(root, env_name);
        let (cols, _) = brain_seo::content::load(root, cfg.staging);
        let f = root.join(".brain/build/server/_rt/content.mjs");
        std::fs::create_dir_all(f.parent().unwrap()).map_err(|e| e.to_string())?;
        std::fs::write(&f, brain_seo::content::module_js(&cols, cfg.per_page)).map_err(|e| e.to_string())?;
    }
    let srv = |rel: &str| server_module(root, rel, &compile, 0);
    // SEO step (INTERNALS §32/§33): brain-seo rewrites every head and writes robots, sitemap, llms, manifest, feeds
    // staging outside production and INDEXNOW_KEY from the selected .brain_env section (process env wins)
    let mut seo_cfg = brain_seo::Config::load(root)?.with_env(root, env_name);
    if let Some(s) = site {
        seo_cfg.site = Some(brain_seo::config::origin(s)?);
    }
    if seo_cfg.name.is_none() {
        seo_cfg.name = env::value(root, env_name, "PUBLIC_SITE_NAME");
    }
    let seo_range: std::cell::Cell<Option<(u32, u32)>> = std::cell::Cell::new(None);
    let mut all_views = Vec::new();
    brain_files(&root.join("views"), &mut all_views);
    let routes: Vec<String> = all_views
        .iter()
        .filter_map(|f| map::route_of(&f.strip_prefix(root).ok()?.to_string_lossy().replace('\\', "/")))
        .filter(|r| r != "*")
        .collect();
    #[cfg(unix)]
    let shooter = og::Shooter::new(root);
    let post = |pages: &mut [brain_pack::Page], assets: &[(String, u64)]| -> Result<brain_pack::PostOut, String> {
        let mut sp: Vec<brain_seo::Page> = pages
            .iter()
            .filter(|p| !p.file.starts_with("_server/"))
            .map(|p| {
                let info = std::fs::read_to_string(root.join(&p.view)).ok().and_then(|src| brain_parse::parse(&src, Path::new(&p.view)).ok()).map(|c| brain_check::seo::view_info(&c));
                let text = |k: &str| {
                    info.as_ref()?.attrs.iter().find(|(a, _)| a == k).and_then(|(_, v)| match v {
                        brain_check::seo::SeoValue::Text(t) => Some(t.clone()),
                        _ => None,
                    })
                };
                brain_seo::Page {
                    route: p.route.clone(),
                    file: p.file.clone(),
                    html: p.html.clone(),
                    lastmod: text("lastmod"),
                    keyword: info.as_ref().and_then(|i| i.keyword.clone()),
                    article: None,
                    view: Some(p.view.clone()),
                    ssr: false,
                }
            })
            .collect();
        // social images: views/_og.brain (or brain-seo's template) rendered in headless Chrome, cached by brain-seo
        #[cfg(unix)]
        let built = {
            let shoot = |job: &brain_seo::og::Job, tpl: &str| shooter.shoot(&job.props, tpl);
            brain_seo::finish_with_og(&seo_cfg, root, &mut sp, &routes, assets, Some(&shoot))?
        };
        #[cfg(not(unix))]
        let built = brain_seo::finish(&seo_cfg, root, &mut sp, &routes, assets)?;
        for s in &sp {
            if let Some(p) = pages.iter_mut().find(|p| p.file == s.file) {
                p.html = s.html.clone();
            }
        }
        let scores: Vec<u32> = built.report.pages.iter().filter_map(|p| p.score).collect();
        seo_range.set(scores.iter().min().zip(scores.iter().max()).map(|(a, b)| (*a, *b)));
        // redirects.json → host configs (BUILD #219): brain-pack folds these fragments into nginx/Caddy (with the base)
        // and the server manifest, then drops them from dist/
        let mut files = built.files;
        if !built.redirects.is_empty() {
            files.push(("_brain/redirects.nginx".into(), brain_seo::redirects::nginx(&built.redirects)));
            files.push(("_brain/redirects.caddy".into(), brain_seo::redirects::caddy(&built.redirects)));
            files.push(("_server/redirects.json".into(), brain_seo::redirects::server_json(&built.redirects).stringify()));
        }
        Ok(brain_pack::PostOut { files, binary: built.binary, warnings: built.warnings, csp: built.csp.iter().map(|(d, o)| (d.to_string(), o.to_string())).collect() })
    };
    let mut cache = cache_cfg(root)?;
    // empty = brain.json "ext" (brain-pack reads it), else web
    cache.ext = BUILD_EXT.get().cloned().unwrap_or_default();
    let offline = cache.offline;
    cache.sw_template = runtime_files().into_iter().find(|(n, _)| n == "sw/brain-sw.js").map(|(_, s)| s);
    let input = brain_pack::Input {
        server_module: Some(&srv),
        cache,
        server_mjs: runtime_files().into_iter().find(|(n, _)| n == "server/server.mjs").map(|(_, s)| s),
        progress: Some(&on_phase),
        post: Some(&post),
        root: root.to_path_buf(),
        compile: &compile,
        runtime_core: core,
        runtime_siblings: siblings,
        layer_order: head_prepend(),
        minify: true,
        site: site.map(str::to_string),
        shim,
        generator: format!("BRAINLLM {}", env!("CARGO_PKG_VERSION")),
    };
    let mut files = Vec::new();
    brain_files(&root.join("views"), &mut files);
    let views: Vec<(String, String)> = files
        .iter()
        .filter_map(|f| {
            let rel = f.strip_prefix(root).ok()?.to_string_lossy().replace('\\', "/");
            map::route_of(&rel).map(|route| (route, rel))
        })
        // the offline page exists only when the offline layer is on (cache.offline)
        .chain((offline && root.join("views/_offline.brain").is_file()).then(|| ("/_offline".to_string(), "views/_offline.brain".to_string())))
        .collect();
    // static `<meta seo>` text values (the fallback when a view has no evaluated seo(props)), read by the parser
    let seo_of = |v: &str| -> Vec<(String, String)> {
        let Some(c) = std::fs::read_to_string(root.join(v)).ok().and_then(|src| brain_parse::parse(&src, Path::new(v)).ok()) else { return Vec::new() };
        brain_check::seo::view_info(&c)
            .attrs
            .into_iter()
            .filter_map(|(k, val)| match val {
                brain_check::seo::SeoValue::Text(t) => Some((k, t)),
                brain_check::seo::SeoValue::Expr(_) => None,
            })
            .collect()
    };
    let out = brain_pack::build(&input, &views, &|v| layouts_of(root, v), &seo_of)?;
    // write only what changed and drop stale hashed chunks: an unchanged rebuild touches no file
    on_phase("write", 0, out.files.len() + out.binary.len());
    let (mut written, mut unchanged) = (0, 0);
    let mut write_if_changed = |f: &Path, bytes: &[u8]| -> Result<(), String> {
        if std::fs::read(f).ok().as_deref() == Some(bytes) {
            unchanged += 1;
            return Ok(());
        }
        written += 1;
        std::fs::create_dir_all(f.parent().unwrap()).map_err(|e| e.to_string())?;
        std::fs::write(f, bytes).map_err(|e| format!("{}: {e}", f.display()))
    };
    for (path, content) in &out.files {
        write_if_changed(&dist.join(path), content.as_bytes())?;
        rep.files.push(format!("dist/{path}"));
    }
    for (path, bytes) in &out.binary {
        write_if_changed(&dist.join(path), bytes)?;
    }
    rep.written = written;
    rep.unchanged = unchanged;
    // dist/ is pure build output: anything this build did not produce (stale chunks, a sitemap from another
    // environment, removed pages) is deleted, so a deploy never ships leftovers
    fn sweep(dir: &Path, dist: &Path, keep: &dyn Fn(&str) -> bool) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                sweep(&p, dist, keep);
                let _ = std::fs::remove_dir(&p); // only succeeds when empty
            } else {
                let rel = p.strip_prefix(dist).unwrap_or(&p).to_string_lossy().replace('\\', "/");
                if !keep(&rel) {
                    let _ = std::fs::remove_file(&p);
                }
            }
        }
    }
    let keep = |rel: &str| out.files.contains_key(rel) || out.binary.contains_key(rel);
    sweep(dist, dist, &keep);
    rep.report = out
        .report
        .iter()
        .map(|r| (r.route.clone(), [r.js_raw, r.js_min, r.js_gz, r.css, r.css_gz, r.html, r.html_gz]))
        .collect();
    rep.warnings.extend(out.warnings);
    rep.signature = out.signature;
    rep.seo = seo_range.get();
    Ok(out.pages.into_iter().map(|p| (p.route, p.html)).collect())
}

pub(crate) fn build_json(r: &BuildReport) -> brain_mcp::Json {
    use brain_mcp::Json;
    let keys = ["jsRaw", "jsMin", "jsGz", "css", "cssGz", "html", "htmlGz"];
    Json::obj([
        ("build", Json::str(&r.signature)),
        ("files", Json::Arr(r.files.iter().map(Json::str).collect())),
        ("warnings", Json::Arr(r.warnings.iter().map(Json::str).collect())),
        (
            "report",
            Json::Arr(
                r.report
                    .iter()
                    .map(|(route, n)| {
                        let mut kv = vec![("route".to_string(), Json::str(route))];
                        kv.extend(keys.iter().zip(n).map(|(k, v)| (k.to_string(), Json::Num(*v as f64))));
                        Json::Obj(kv)
                    })
                    .collect(),
            ),
        ),
    ])
}

/// `brain build --ext web|brain` (#26): the published file names; flag > `BRAIN_EXT` > brain.json `"ext"` > `web`.
static BUILD_EXT: std::sync::OnceLock<String> = std::sync::OnceLock::new();

fn cmd_build(args: &[String]) -> Result<(), String> {
    let identity = brand::Session::start(args, "Turning your frontend into browser-ready output.");
    if let Some(e) = flag(args, "--ext").or_else(|| std::env::var("BRAIN_EXT").ok().filter(|v| !v.is_empty())) {
        if e != "web" && e != "brain" {
            return Err(format!("invalid --ext `{e}`; fix: `--ext web` (.html/.js/.css, the default) or `--ext brain` (.brain/.jsbrain/.cssbrain)"));
        }
        let _ = BUILD_EXT.set(e);
    }
    let mode = term::detect(args, term::stderr_tty(), &term::env_var);
    let root = app_root()?;
    let mut pr = term::Progress::new(mode, term::utf8(&term::env_var));
    if mode == term::Mode::Rich {
        eprintln!("{}", term::header(pr.paint(), &format!("build · {}", root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())));
    }
    let res = run_build_with(&root, flag(args, "--site"), &env::name(args, "production"), Some(&pr));
    pr.end();
    let rep = match res {
        Ok(rep) => rep,
        Err(error) => { identity.finish("Build stopped. Resolve the diagnostic and try again."); return Err(error); }
    };
    identity.finish("Build complete. Your intent, compiled.");
    if want_json(args) {
        println!("{}", build_json(&rep).stringify());
        return Ok(());
    }
    if args.iter().any(|a| a == "--report") {
        println!("{:<24} {:>9} {:>9} {:>8} {:>8} {:>8} {:>8}", "page", "js raw", "js min", "js gz", "css", "css gz", "html gz");
        for (route, n) in &rep.report {
            println!("{route:<24} {:>9} {:>9} {:>8} {:>8} {:>8} {:>8}", n[0], n[1], n[2], n[3], n[4], n[6]);
        }
    }
    for w in &rep.warnings {
        eprintln!("{}", pr.paint().warn(&format!("warning: {w}")));
    }
    if mode != term::Mode::Off {
        let views: Vec<term::ViewSize> = rep.report.iter().map(|(r, n)| term::ViewSize { route: r.clone(), js_gz: n[2], css_gz: n[4], html_gz: n[6] }).collect();
        let budget = budget_view_kb(&root).map(|kb| kb * 1024);
        let s = term::Summary { views: &views, budget, written: rep.written, unchanged: rep.unchanged, signature: Some(&rep.signature).filter(|s| !s.is_empty()).map(String::as_str), seo: rep.seo, took: pr.elapsed(), warnings: rep.warnings.len() };
        print!("{}", term::summary(pr.paint(), &s));
    }
    Ok(())
}

/// `brain start [--port N] [--host H] [--instances N] [--max-memory 300M] [--env production] [--install]`.
#[cfg(unix)]
const START_USAGE: &str = "usage: brain start [--port 3000] [--host 127.0.0.1] [--instances 1] [--max-memory 300M] [--env production]
       brain start --install [same flags]   write a systemd unit / launchd plist for this app
       brain stop | reload | status [--json]

Serves dist/ in production: static apps from Rust, apps with dist/server.mjs under a supervisor (zero-downtime
reload, restart with backoff, crash-loop exit 1, /_health, RSS limit, logs/app.log). Environment: PORT, HOST,
BRAIN_INSTANCES, BRAIN_MAX_MEMORY, BRAIN_NODE.
";

fn cmd_start(args: &[String]) -> ExitCode {
    if args.iter().any(|a| a == "--help" || a == "-h") {
        print!("{START_USAGE}");
        return ExitCode::SUCCESS;
    }
    // an unknown flag is an error: `brain start --hepl` must never start serving
    const VALUED: &[&str] = &["--port", "--host", "--instances", "--max-memory", "--env", "--health-ms", "--rss-ms", "--backoff-ms"];
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        let name = a.split('=').next().unwrap_or(a);
        if VALUED.contains(&name) {
            i += if a.contains('=') { 1 } else { 2 };
            continue;
        }
        if a == "--install" || a == "--json" {
            i += 1;
            continue;
        }
        eprintln!("error: unknown argument `{a}` for brain start; fix: run `brain start --help`");
        return ExitCode::from(2);
    }
    let run = || -> Result<i32, String> {
        let root = app_root()?;
        if args.iter().any(|a| a == "--install") {
            let rest: Vec<String> = args.iter().filter(|a| *a != "--install").cloned().collect();
            println!("{}", start::install(&root, &rest.join(" "))?);
            return Ok(0);
        }
        let num = |k: &str, env: &str, d: &str| flag(args, k).or_else(|| std::env::var(env).ok()).unwrap_or_else(|| d.to_string());
        let port = num("--port", "PORT", "3000").parse().map_err(|_| "invalid --port; fix: a number 1-65535".to_string())?;
        let o = start::Opts {
            host: num("--host", "HOST", "127.0.0.1"),
            port,
            instances: num("--instances", "BRAIN_INSTANCES", "1").parse().map_err(|_| "invalid --instances".to_string())?,
            max_rss_kb: start::parse_mem(&num("--max-memory", "BRAIN_MAX_MEMORY", "300M")).ok_or("invalid --max-memory; fix: e.g. 300M or 1G")?,
            env: env::name(args, "production"),
            health_ms: num("--health-ms", "BRAIN_HEALTH_MS", "10000").parse().unwrap_or(10_000),
            rss_ms: num("--rss-ms", "BRAIN_RSS_MS", "5000").parse().unwrap_or(5_000),
            backoff_ms: num("--backoff-ms", "BRAIN_BACKOFF_MS", "100").parse().unwrap_or(100),
        };
        start::start(&root, &o)
    };
    match run() {
        Ok(c) => ExitCode::from(c as u8),
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(2)
        }
    }
}

/// `brain.json` → `"budget": { "viewKB": N }` (gz bytes per view, INTERNALS §16).
fn budget_view_kb(root: &Path) -> Option<usize> {
    let j = brain_mcp::Json::parse(&std::fs::read_to_string(root.join("brain.json")).ok()?).ok()?;
    match j.get("budget")?.get("viewKB")? {
        brain_mcp::Json::Num(n) if *n > 0.0 => Some(*n as usize),
        _ => None,
    }
}

pub(crate) fn compile_fn(root: PathBuf, dev: bool) -> brain_serve::CompileFn {
    // Block-level incremental compile (INTERNALS §28): compare the file's blocks with the last compile of the same
    // path. No change → previous output; style only → recompile CSS (no codegen); anything else → full compile
    // (through the persistent content cache).
    // + the PUBLIC_* constants it was compiled with: a changed .brain_env makes every file compile again
    type Last = (brain_codegen::blocks::Blocks, String, String, String, Vec<PathBuf>, Vec<(String, String)>, Option<Vec<String>>);
    let last: Arc<std::sync::Mutex<HashMap<PathBuf, Last>>> = Arc::default();
    Arc::new(move |src: &str, path: &Path| {
        use brain_codegen::blocks::{blocks, change, compile_style, Change};
        let rel = path.strip_prefix(&root).unwrap_or(path);
        let b = blocks(src, rel);
        let to_json = |ds: Vec<compiler::Diag>| ds.iter().map(compiler::Diag::to_json).collect::<Vec<_>>();
        // clone out first: an `if let` keeps the MutexGuard alive for its whole block (re-locking inside deadlocked)
        let prev = last.lock().unwrap().get(path).cloned();
        let public = env::public();
        if let Some((ob, js, css, server_js, deps, _, stamps)) = prev.filter(|p| p.5 == public && p.6.is_some() && compiler::dependency_hashes(&p.4) == p.6) {
            match change(&ob, &b) {
                Change::None => return Ok(brain_serve::Compiled { js, css, deps, server_js }),
                Change::Style => {
                    let opts = brain_codegen::Options { dev, root: Some(root.clone()), ..Default::default() };
                    let css = compile_style(src, rel, &opts).map_err(to_json)?;
                    let mut cache = last.lock().unwrap();
                    cache.remove(path);
                    let size = js.len() + css.len() + server_js.len();
                    if size <= 1024 * 1024 {
                        let bytes: usize = cache.values().map(|v| v.1.len() + v.2.len() + v.3.len()).sum();
                        if cache.len() >= 256 || bytes.saturating_add(size) > 64 * 1024 * 1024 { cache.clear(); }
                        cache.insert(path.to_path_buf(), (b, js.clone(), css.clone(), server_js.clone(), deps.clone(), public.clone(), stamps));
                    }
                    return Ok(brain_serve::Compiled { js, css, deps, server_js });
                }
                _ => {}
            }
        }
        let o = compiler::compile(src, rel, &root, dev).map_err(to_json)?;
        let stamps = compiler::dependency_hashes(&o.deps);
        let mut cache = last.lock().unwrap();
        // This optional optimization must not retain every compiled output for an arbitrarily large app.
        let bytes: usize = cache.values().map(|v| v.1.len() + v.2.len() + v.3.len()).sum();
        let size = o.js.len() + o.css.len() + o.server_js.len();
        if cache.len() >= 256 || bytes.saturating_add(size) > 64 * 1024 * 1024 { cache.clear(); }
        if size <= 1024 * 1024 {
            cache.insert(path.to_path_buf(), (b, o.js.clone(), o.css.clone(), o.server_js.clone(), o.deps.clone(), public, stamps));
        }
        Ok(brain_serve::Compiled { js: o.js, css: o.css, deps: o.deps, server_js: o.server_js })
    })
}

pub(crate) fn head_prepend() -> String {
    format!("<style>{}</style>", brain_css::LAYER_ORDER)
}

pub(crate) fn runtime_files() -> Vec<(String, String)> {
    RUNTIME.iter().map(|(n, b)| (n.to_string(), b.to_string())).collect()
}

#[cfg(not(unix))]
fn cmd_test(_: &[String]) -> ExitCode {
    eprintln!("error: `brain test` needs a unix system (CDP pipe); fix: run it on macOS or Linux");
    ExitCode::from(2)
}

/// Runs the `*.test.brain` files of `target` (file, folder or the current app) in headless Chrome.
#[cfg(unix)]
pub(crate) fn run_tests(target: Option<PathBuf>, timeout: u64) -> Result<Vec<testrun::FileReport>, String> {
    // Absolute: with a relative path, find_root would return "" as the root.
    let target = target.map(|t| std::path::absolute(&t).unwrap_or(t));
    let root = target
        .as_deref()
        .map(scaffold::find_root)
        .unwrap_or_else(|| app_root().ok())
        .ok_or("not inside a BRAINLLM app (tokens.cssbrain is missing); fix: cd into the app or pass its path")?;
    let mut files = Vec::new();
    match &target {
        Some(t) if t.is_file() => files.push(t.canonicalize().unwrap_or(t.clone())),
        Some(t) => testrun::test_files(t, &mut files),
        None => testrun::test_files(&root, &mut files),
    }
    let root = root.canonicalize().unwrap_or(root);
    let files: Vec<PathBuf> = files.into_iter().map(|f| f.canonicalize().unwrap_or(f)).collect();
    if files.is_empty() {
        return Ok(vec![]);
    }
    let opts = brain_serve::Options { root: root.clone(), host: "127.0.0.1".into(), port: 0, head_prepend: head_prepend(), runtime: runtime_files(), env: Some(env::loader(&root, "development")), base: "/".into(), diagnose: None, problems: None, upstream: None, mock: None };
    let (addr, _) = brain_serve::start(opts, compile_fn(root.clone(), true), false).map_err(|e| format!("could not start the test server: {e}"))?;
    testrun::run(&format!("http://{addr}"), &root, &files, std::time::Duration::from_secs(timeout))
}

#[cfg(unix)]
fn cmd_test(args: &[String]) -> ExitCode {
    let json = want_json(args);
    let timeout = flag(args, "--timeout").and_then(|t| t.parse().ok()).unwrap_or(60u64);
    let t0 = std::time::Instant::now();
    // --heap: an instance alive after its test unmounted, or an fx timer/listener without cleanup, fails the file (§9)
    testrun::set_heap(args.iter().any(|a| a == "--heap"));
    let reports = match run_tests(positional(args, &["--timeout"]).first().map(PathBuf::from), timeout) {
        Ok(r) => r,
        Err(e) => {
            if json { println!("{}", error_json(&e)) } else { eprintln!("error: {e}") }
            return ExitCode::from(2);
        }
    };
    let failed = reports.iter().any(testrun::FileReport::failed);
    if json {
        println!("{}", brain_mcp::Json::Arr(reports.iter().map(testrun::FileReport::to_json).collect()).stringify());
    } else {
        if reports.is_empty() {
            eprintln!("no *.test.brain files");
        }
        let (mut pass, mut fail, mut ax, mut lk) = (0, 0, 0, 0);
        for r in &reports {
            println!("{}", r.file);
            if let Some(e) = &r.load_error {
                println!("  ✗ failed to load: {e}");
                fail += 1;
            }
            for t in &r.tests {
                if t.ok {
                    pass += 1;
                    println!("  ✓ {} ({:.1} ms)", t.name, t.ms);
                } else {
                    fail += 1;
                    println!("  ✗ {}\n      {}", t.name, t.error.as_deref().unwrap_or(""));
                }
            }
            for a in &r.a11y {
                ax += 1;
                println!("  ♿ {} {} `{}` (role {}, test «{}»)\n      fix: {}", a.code, a.msg, a.node, a.role, a.test, a.fix);
            }
            for l in &r.leaks {
                lk += 1;
                let s = |k: &str| l.get(k).and_then(brain_mcp::Json::as_str).unwrap_or("").to_string();
                println!("  ⚠ {} {} {}\n      fix: {}", s("code"), s("file"), s("msg"), s("fix"));
            }
        }
        let leaks = if args.iter().any(|a| a == "--heap") { format!(", {lk} leak(s)") } else { String::new() };
        eprintln!("{pass} ok, {fail} failed, {ax} accessibility issues{leaks} in {:.2} s", t0.elapsed().as_secs_f64());
    }
    if failed { ExitCode::FAILURE } else { ExitCode::SUCCESS }
}

/// `brain explain <code>` → `{code, title, why, example}` from the compiler's catalog.
pub(crate) fn explain_json(code: &str) -> Result<brain_mcp::Json, String> {
    use brain_mcp::Json;
    let code = code.trim().to_ascii_uppercase();
    let (c, title, why, ex) = brain_check::catalog::explain(&code)
        .or_else(|| brain_seo::CATALOG.iter().find(|e| e.0 == code).map(|e| (e.0, e.1, e.2, e.3)))
        .ok_or_else(|| format!("unknown diagnostic code `{code}`; fix: use a code printed by `brain check`, e.g. B0302"))?;
    Ok(Json::obj([("code", Json::str(c)), ("title", Json::str(title)), ("why", Json::str(why)), ("example", Json::str(ex))]))
}

fn cmd_explain(args: &[String]) -> Result<(), String> {
    let code = positional(args, &[]).into_iter().next().ok_or("usage: brain explain <code>; fix: e.g. `brain explain B0302`")?;
    let j = explain_json(&code)?;
    if want_json(args) {
        println!("{}", j.stringify());
    } else {
        let s = |k: &str| j.get(k).and_then(brain_mcp::Json::as_str).unwrap_or("").to_string();
        println!("{} — {}\n\nwhy: {}\n\nexample:\n  {}", s("code"), s("title"), s("why"), s("example"));
    }
    Ok(())
}

/// `brain doctor`: check (errors + B10xx overengineering warnings) → tests in Chrome with accessibility → leaks.
/// A single summary; `ok` is false on any check error or failed test.
pub(crate) fn run_doctor(root: Option<PathBuf>) -> Result<(bool, brain_mcp::Json), String> {
    // doctor = check + tests + leaks (§18): the tests run with the leak watch on (B0601/B0602 fail their file)
    #[cfg(unix)]
    testrun::set_heap(true);
    use brain_mcp::Json;
    let (files, diags) = run_check(root.clone())?;
    let errors: Vec<&compiler::Diag> = diags.iter().filter(|d| !d.is_warning()).collect();
    let warnings = diags.len() - errors.len();
    let check = Json::obj([
        ("files", Json::Num(files as f64)),
        ("errors", Json::Num(errors.len() as f64)),
        ("warnings", Json::Num(warnings as f64)),
        ("diags", brain_mcp::Json::parse(&diags_json(&diags)).unwrap_or(Json::Arr(vec![]))),
    ]);
    #[cfg(unix)]
    let (test_ok, test) = match run_tests(root.clone(), 60) {
        Ok(reports) => {
            let passed = reports.iter().flat_map(|r| &r.tests).filter(|t| t.ok).count();
            let failed = reports.iter().map(|r| r.tests.iter().filter(|t| !t.ok).count() + usize::from(r.load_error.is_some())).sum::<usize>();
            let a11y = reports.iter().map(|r| r.a11y.len()).sum::<usize>();
            let ok = !reports.iter().any(testrun::FileReport::failed);
            (ok, Json::obj([
                ("passed", Json::Num(passed as f64)),
                ("failed", Json::Num(failed as f64)),
                ("a11y", Json::Num(a11y as f64)),
                // captured runtime errors of every test file, deduplicated (brain_errors' view, offline)
                ("errors", Json::Arr(reports.iter().flat_map(|r| r.errors.iter().map(move |e| match e {
                    Json::Obj(kv) => Json::Obj([("test".to_string(), Json::str(&r.file))].into_iter().chain(kv.clone()).collect()),
                    o => o.clone(),
                })).collect())),
                ("files", Json::Arr(reports.iter().map(testrun::FileReport::to_json).collect())),
            ]))
        }
        Err(e) => (false, Json::obj([("error", Json::str(e))])),
    };
    #[cfg(not(unix))]
    let (test_ok, test) = (true, Json::obj([("skipped", Json::str("brain test needs macOS or Linux"))]));
    // the app folder (a file target checks its app); SEO audits the last build, advice reads the sources
    let app = root.as_deref().map(|p| std::path::absolute(p).unwrap_or(p.to_path_buf())).and_then(|p| scaffold::find_root(&p)).or_else(|| app_root().ok());
    // the production build (§25: done = doctor exit 0 = the app ships): what fails `brain build` fails doctor too —
    // empty production keys (B0720), broken links and anchors (B1819/B1820)… — and SEO then audits that dist/
    let (build_ok, build) = match &app {
        Some(a) if check.get("errors") == Some(&Json::Num(0.0)) => match run_build(a, None) {
            Ok(r) => (true, Json::obj([("ok", Json::Bool(true)), ("signature", Json::str(&r.signature)), ("files", Json::Num(r.files.len() as f64))])),
            Err(e) => (false, Json::obj([("ok", Json::Bool(false)), ("error", Json::str(&e))])),
        },
        Some(_) => (true, Json::obj([("skipped", Json::str("check has errors; fix them first"))])),
        None => (true, Json::obj([("skipped", Json::str("no app"))])),
    };
    let (seo_ok, seo) = match &app {
        Some(a) if build_ok && a.join("dist").is_dir() => {
            let (code, out) = brain_seo::cli(a, &["audit".into(), "--json".into()]);
            // no .brain_env value in doctor's output (§36): the audit's absolute URLs carry PUBLIC_SITE_URL, so they
            // become paths (`https://example.com/about` → `/about`)
            let out = match env::value(a, "production", "PUBLIC_SITE_URL") {
                Some(site) => out.replace(site.trim_end_matches('/'), ""),
                None => out,
            };
            (code == 0, Json::parse(&out).unwrap_or_else(|_| Json::obj([("error", Json::str(out))])))
        }
        _ if !build_ok => (true, Json::obj([("skipped", Json::str("the build fails (see build); SEO audits the built pages"))])),
        _ => (true, Json::obj([("skipped", Json::str("no dist/ yet; run `brain build` to audit SEO"))])),
    };
    let mut advice: Vec<Json> = app.as_deref().map(brain_check::doctor::doctor).unwrap_or_default().iter().filter_map(|d| Json::parse(&d.to_json()).ok()).collect();
    // .brain_env: tracked by git (error), a stray .env, the example kept in sync, unused keys (B0722)
    let mut env_ok = true;
    for (code, level, msg, fix) in app.as_deref().map(env::doctor).unwrap_or_default() {
        env_ok &= level != "error";
        advice.push(Json::obj([("code", Json::str(code)), ("level", Json::str(level)), ("file", Json::str(env::FILE)), ("msg", Json::str(msg)), ("fix", Json::str(fix))]));
    }
    if let Some(a) = &app {
        let legacy = std::fs::read_to_string(a.join("brain.json")).ok().and_then(|s| Json::parse(&s).ok()).and_then(|j| j.get("site").and_then(Json::as_str).map(str::to_string));
        if legacy.is_some() {
            advice.push(Json::obj([("code", Json::str("B0728")), ("level", Json::str("info")), ("file", Json::str("brain.json")), ("msg", Json::str("brain.json \"site\" is a legacy fallback: the site URL comes from PUBLIC_SITE_URL of each environment")), ("fix", Json::str("set PUBLIC_SITE_URL in each section of .brain_env and delete \"site\" from brain.json"))]));
        }
    }
    // a quality bar lowered below its default does not count as passing: an agent must fix the pages, not the bar
    // (QA #153: a model reached doctor 0 by setting seo.minScore to 60)
    let mut bar_ok = true;
    if let Some(a) = &app {
        let j = std::fs::read_to_string(a.join("brain.json")).ok().and_then(|s| Json::parse(&s).ok());
        if let Some(Json::Num(n)) = j.as_ref().and_then(|j| j.get("seo")).and_then(|s| s.get("minScore")) {
            if *n < 80.0 {
                bar_ok = false;
                advice.push(Json::obj([
                    ("code", Json::str("B0733")),
                    ("level", Json::str("error")),
                    ("file", Json::str("brain.json")),
                    ("msg", Json::str(format!("seo.minScore is {n}, below the default 80: lowering the bar does not make the pages pass"))),
                    ("fix", Json::str("set \"minScore\": 80 (or remove it) and fix the findings `brain seo audit` lists")),
                ]));
            }
        }
    }
    let ok = errors.is_empty() && test_ok && build_ok && seo_ok && env_ok && bar_ok;
    // leaks found while the tests ran (each also fails its test file): B0601 / B0602 with their fix
    let heap = match test.get("files") {
        Some(Json::Arr(fs)) => Json::obj([(
            "leaks",
            Json::Arr(fs.iter().flat_map(|f| match f.get("leaks") {
                Some(Json::Arr(ls)) => ls.iter().map(|l| match l {
                    Json::Obj(kv) => Json::Obj([("test".to_string(), f.get("file").cloned().unwrap_or(Json::Null))].into_iter().chain(kv.clone()).collect()),
                    o => o.clone(),
                }).collect::<Vec<_>>(),
                _ => vec![],
            }).collect()),
        )]),
        _ => Json::obj([("skipped", Json::str("the tests did not run"))]),
    };
    Ok((ok, Json::obj([("ok", Json::Bool(ok)), ("check", check), ("test", test), ("build", build), ("seo", seo), ("advice", Json::Arr(advice)), ("heap", heap)])))
}

fn cmd_doctor(args: &[String]) -> ExitCode {
    let target = positional(args, &[]).first().map(PathBuf::from);
    match run_doctor(target) {
        Err(e) => {
            if want_json(args) { println!("{}", error_json(&e)) } else { eprintln!("error: {e}") }
            ExitCode::from(2)
        }
        Ok((ok, j)) => {
            if want_json(args) {
                println!("{}", j.stringify());
            } else {
                let n = |a: &str, b: &str| match j.get(a).and_then(|x| x.get(b)) {
                    Some(brain_mcp::Json::Num(v)) => *v as u64,
                    _ => 0,
                };
                if let Some(brain_mcp::Json::Arr(ds)) = j.get("check").and_then(|c| c.get("diags")) {
                    for d in ds {
                        let s = |k: &str| d.get(k).and_then(brain_mcp::Json::as_str).unwrap_or("").to_string();
                        let num = |k: &str| match d.get(k) { Some(brain_mcp::Json::Num(v)) => *v as u64, _ => 0 };
                        println!("{}:{}:{} {} {}\n  fix: {}", s("file"), num("line"), num("col"), s("code"), s("msg"), s("fix"));
                    }
                }
                let test_err = j.get("test").and_then(|t| t.get("error")).and_then(brain_mcp::Json::as_str);
                if let Some(brain_mcp::Json::Arr(ds)) = j.get("advice") {
                    for d in ds {
                        let s = |k: &str| d.get(k).and_then(brain_mcp::Json::as_str).unwrap_or("").to_string();
                        println!("advice  {} {} {}\n  fix: {}", s("file"), s("code"), s("msg"), s("fix"));
                    }
                }
                let seo_line = match j.get("seo") {
                    Some(s) if s.get("skipped").is_some() => s.get("skipped").and_then(brain_mcp::Json::as_str).unwrap_or("").to_string(),
                    Some(s) if s.get("error").is_some() => format!("did not run: {}", s.get("error").and_then(brain_mcp::Json::as_str).unwrap_or("")),
                    Some(s) => {
                        let scores: Vec<u64> = match s.get("pages") {
                            Some(brain_mcp::Json::Arr(ps)) => ps.iter().filter_map(|p| match p.get("score") { Some(brain_mcp::Json::Num(v)) => Some(*v as u64), _ => None }).collect(),
                            _ => vec![],
                        };
                        let min = match s.get("minScore") { Some(brain_mcp::Json::Num(v)) => *v as u64, _ => 80 };
                        let ok = matches!(s.get("ok"), Some(brain_mcp::Json::Bool(true)));
                        format!("{} page(s) scored {}–{} (minimum {min}){}", scores.len(), scores.iter().min().unwrap_or(&0), scores.iter().max().unwrap_or(&0), if ok { "" } else { " · failing: run `brain seo audit`" })
                    }
                    None => String::new(),
                };
                // runtime errors captured while the tests ran (SPEC: listed by brain doctor; they do not fail it alone)
                let captured: Vec<String> = match j.get("test").and_then(|t| t.get("errors")) {
                    Some(brain_mcp::Json::Arr(es)) => es
                        .iter()
                        .map(|e| {
                            let s = |k: &str| e.get(k).and_then(brain_mcp::Json::as_str).unwrap_or("").to_string();
                            let n = match e.get("n") { Some(brain_mcp::Json::Num(v)) => *v as u64, _ => 1 };
                            format!("  {} {}{}{}", s("type"), s("msg"), if s("line").is_empty() { String::new() } else { format!(" ({})", s("line")) }, if n > 1 { format!(" ×{n}") } else { String::new() })
                        })
                        .collect(),
                    _ => vec![],
                };
                let leaks_line = match j.get("heap").and_then(|h| h.get("leaks")) {
                    Some(brain_mcp::Json::Arr(ls)) if !ls.is_empty() => format!(
                        "{} suspected:\n{}",
                        ls.len(),
                        ls.iter().map(|l| { let s = |k: &str| l.get(k).and_then(brain_mcp::Json::as_str).unwrap_or("").to_string(); format!("  {} {} {} ({})\n  fix: {}", s("code"), s("name"), s("msg"), s("test"), s("fix")) }).collect::<Vec<_>>().join("\n")
                    ),
                    Some(_) => "none after the tests unmounted (GC forced)".into(),
                    None => j.get("heap").and_then(|h| h.get("skipped")).and_then(brain_mcp::Json::as_str).unwrap_or("did not run").to_string(),
                };
                let build_line = match j.get("build") {
                    Some(b) if b.get("error").is_some() => format!("fails (as `brain build` would):\n{}", b.get("error").and_then(brain_mcp::Json::as_str).unwrap_or("").lines().map(|l| format!("  {l}")).collect::<Vec<_>>().join("\n")),
                    Some(b) if b.get("skipped").is_some() => format!("skipped: {}", b.get("skipped").and_then(brain_mcp::Json::as_str).unwrap_or("")),
                    Some(b) => format!("ok · build {}", b.get("signature").and_then(brain_mcp::Json::as_str).unwrap_or("")),
                    None => String::new(),
                };
                let errors_line = if captured.is_empty() { "none captured while the tests ran".to_string() } else { format!("{} captured while the tests ran:\n{}", captured.len(), captured.join("\n")) };
                println!(
                    "\ncheck   {} file(s) · {} error(s) · {} warning(s)\ntests   {}\nerrors  {errors_line}\nbuild   {build_line}\nseo     {seo_line}\nleaks   {leaks_line}\n{}",
                    n("check", "files"),
                    n("check", "errors"),
                    n("check", "warnings"),
                    match test_err {
                        Some(e) => format!("did not run: {e}"),
                        None => format!("{} ok · {} failed · {} accessibility issues", n("test", "passed"), n("test", "failed"), n("test", "a11y")),
                    },
                    if ok { "✓ no errors" } else { "✗ there are errors: fix them and run `brain doctor` again" }
                );
            }
            if ok { ExitCode::SUCCESS } else { ExitCode::FAILURE }
        }
    }
}

/// `--base`: a path that starts and ends with `/` (`/`, `/app/`, `/docs/v2/`).
pub(crate) fn check_base(b: &str) -> Result<String, String> {
    if b.starts_with('/') && b.ends_with('/') && !b.contains("//") && !b.contains(['?', '#', ' ']) {
        Ok(b.to_string())
    } else {
        Err(format!("invalid base `{b}`; fix: a path that starts and ends with `/`, like `/app/`"))
    }
}

/// Live dev server info (INTERNALS §9.1): `<app>/.brain/dev.json` = `{port, pid}`.
pub(crate) fn dev_json_path(root: &Path) -> PathBuf {
    root.join(".brain/dev.json")
}

fn cmd_dev(args: &[String]) -> Result<(), String> {
    let root = app_root()?;
    let port = match flag(args, "--port") {
        Some(p) => p.parse().map_err(|_| format!("invalid port `{p}`; fix: a number 1-65535"))?,
        None => 5180,
    };
    let host = flag(args, "--host").unwrap_or_else(|| "127.0.0.1".into());
    // --mock[=seed] (#266): fake api() data; the seed defaults to the app's name, so a reload shows the same data
    let mock = args.iter().find_map(|a| {
        if a == "--mock" {
            Some(root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "brain".into()))
        } else {
            a.strip_prefix("--mock=").map(str::to_string)
        }
    });
    // base (#148): `--base /app/` or brain.json "base"; dev serves under it exactly like production
    let base = match flag(args, "--base") {
        Some(b) => check_base(&b)?,
        None => brain_pack::read_base(&root),
    };
    let opts = brain_serve::Options { root: root.clone(), host: host.clone(), port, head_prepend: head_prepend(), runtime: runtime_files(), env: Some(env::loader(&root, &env::name(args, "development"))), base, diagnose: Some(dev_diagnose(&root)), problems: Some(dev_problems(&root)), upstream: None, mock };
    let mode = term::detect(args, term::stderr_tty(), &term::env_var);
    let identity = brand::Session::start(args, "Wiring your frontend for the next idea.");
    let boot = std::time::Instant::now();
    let view = (mode == term::Mode::Rich).then(|| Arc::new(term::DevView::new(term::utf8(&term::env_var))));
    // --profile (#86.4): one JSON line per edit on stdout, `{"profile":"edit","path","ms","js","css"}` (ms = file
    // event → compiled), plus `{"profile":"boot","ms"}`; an unchanged save emits nothing (no work was done)
    let profile = args.iter().any(|a| a == "--profile");
    // impact report per save (§43.1): an `impact {…}` line in plain mode (the rich view keeps its own panel)
    let tracker = impact::Tracker::new(&root, view.is_none() && !args.iter().any(|a| a == "--quiet"));
    let v = view.clone();
    let on_event: Option<brain_serve::EventFn> = Some(Arc::new(move |ev: &str| {
            tracker.event(ev);
            if profile && ev.starts_with("{\"type\":\"update\"") {
                println!("{{\"profile\":\"edit\",{}", &ev["{\"type\":\"update\",".len()..]);
            }
            if let Some(v) = &v {
                v.event(ev)
            }
        }) as brain_serve::EventFn);
    brain_serve::set_quiet(view.is_some());
    let dev = brain_serve::start_dev(opts, compile_fn(root.clone(), true), true, on_event).map_err(|e| e.to_string())?;
    if profile {
        println!("{{\"profile\":\"boot\",\"ms\":{:.1}}}", boot.elapsed().as_secs_f64() * 1000.0);
    }
    let addr = dev.addr;
    let url = format!("http://{host}:{}{}", addr.port(), dev.url().trim_start_matches(&format!("http://{addr}")));
    // the debugger (§9.1, #269) on :P+1, same origin for its data: it proxies /@brain/metrics and /@brain/problems here
    let inspector = if args.iter().any(|a| a == "--no-inspect") {
        None
    } else {
        let port = match flag(args, "--inspect-port") {
            Some(p) => p.parse().map_err(|_| format!("invalid --inspect-port `{p}`; fix: a number 1-65535"))?,
            None => addr.port().saturating_add(1),
        };
        match start_devtools(&root, &host, port, &format!("http://127.0.0.1:{}{}", addr.port(), dev.url().trim_start_matches(&format!("http://{addr}")))) {
            Ok(d) => Some(d),
            Err(e) => {
                eprintln!("brain run: the debugger did not start ({e}); the app is unaffected");
                None
            }
        }
    };
    let inspect_port = inspector.as_ref().map(|d| d.addr.port());
    let info = format!(
        "{{\"port\":{},\"pid\":{},\"url\":{}{}}}",
        addr.port(),
        std::process::id(),
        brain_serve::json_str(&url),
        inspect_port.map(|p| format!(",\"inspectPort\":{p},\"inspectUrl\":{}", brain_serve::json_str(&format!("http://{host}:{p}")))).unwrap_or_default()
    );
    let _ = std::fs::create_dir_all(root.join(".brain"));
    let _ = std::fs::write(dev_json_path(&root), &info);
    identity.finish("Dev server ready. Save a change and keep building.");
    if want_json(args) {
        println!("{info}");
    } else if let Some(v) = &view {
        let mut views = Vec::new();
        brain_files(&root.join("views"), &mut views);
        let name = root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        v.boot(&name, &url, lan_url(&host, addr.port()).as_deref(), views.len(), boot.elapsed());
    } else {
        eprintln!("brain run → {url}  (root {})", root.display());
    }
    if let (Some(p), None) = (inspect_port, &view) {
        eprintln!("debugger → http://{host}:{p}");
    }
    let _keep = inspector;
    dev.join()
}

/// Every diagnostic of one file, as `brain check --json` objects: what the dev overlay shows (#267).
pub(crate) fn dev_diagnose(root: &Path) -> brain_serve::DiagnoseFn {
    let root = root.to_path_buf();
    Arc::new(move |p: &Path| {
        let Ok(src) = std::fs::read_to_string(p) else { return Vec::new() };
        let rel = p.strip_prefix(&root).unwrap_or(p);
        compiler::check(&src, rel, p).0.iter().map(|d| d.to_json()).collect()
    })
}

/// `GET /@brain/problems` (#269): `brain check --json` of the whole app, each diagnostic with its `family` (`B05xx`)
/// and the `component` of its file, for the debugger's Problems view.
pub(crate) fn dev_problems(root: &Path) -> brain_serve::ProblemsFn {
    let root = root.to_path_buf();
    Arc::new(move || {
        let diags = run_check_base(Some(root.clone())).map(|(_, d, _)| d).unwrap_or_default();
        let items: Vec<String> = diags
            .iter()
            .map(|d| {
                let j = d.to_json();
                let family = format!("{}xx", d.code.get(..3).unwrap_or(d.code));
                let comp = if d.file.ends_with(".brain") { brain_parse::component_name(Path::new(&d.file)) } else { String::new() };
                match j.rfind('}') {
                    Some(i) => format!("{},\"family\":{},\"component\":{}}}", &j[..i], brain_serve::json_str(&family), brain_serve::json_str(&comp)),
                    None => j,
                }
            })
            .collect();
        format!("[{}]", items.join(","))
    })
}

/// The debugger app (devtools/, embedded) written to `<app>/.brain/devtools/` and served on `port` by its own dev server,
/// which proxies the inspector data of `upstream` (the app's dev server).
fn start_devtools(root: &Path, host: &str, port: u16, upstream: &str) -> Result<brain_serve::Dev, String> {
    let dir = root.join(".brain/devtools");
    for (name, src) in DEVTOOLS {
        let f = dir.join(name);
        if std::fs::read_to_string(&f).ok().as_deref() != Some(*src) {
            std::fs::create_dir_all(f.parent().unwrap()).map_err(|e| e.to_string())?;
            std::fs::write(&f, src).map_err(|e| e.to_string())?;
        }
    }
    let opts = brain_serve::Options {
        root: dir.clone(),
        host: host.to_string(),
        port,
        head_prepend: head_prepend(),
        runtime: runtime_files(),
        env: None,
        base: "/".into(),
        diagnose: None,
        problems: None,
        upstream: Some(upstream.to_string()),
        mock: None,
    };
    brain_serve::start_dev(opts, compile_fn(dir, true), true, None).map_err(|e| e.to_string())
}

/// The LAN URL when the server listens on every interface (the address a phone on the same network uses).
fn lan_url(host: &str, port: u16) -> Option<String> {
    if host != "0.0.0.0" && host != "::" {
        return None;
    }
    // connect() on UDP sends nothing: it only picks the outgoing interface
    let s = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("192.0.2.1:9").ok()?;
    Some(format!("http://{}:{port}", s.local_addr().ok()?.ip()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unchanged_blocks_revalidate_dependencies_before_hmr_reuse() {
        let root = std::env::temp_dir().join(format!("brain-cfn-deps-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("comp")).unwrap();
        std::fs::write(root.join("tokens.cssbrain"), ":root{--space-2:8px}").unwrap();
        let cf = compile_fn(root.clone(), true);
        let src = "<p>Hello</p><style>p{gap:var(--space-2)}</style>";
        let path = root.join("comp/x.brain");
        assert!(cf(src, &path).is_ok());
        std::fs::write(root.join("tokens.cssbrain"), ":root{--space-3:12px}").unwrap();
        assert!(cf(src, &path).is_err(), "unchanged component blocks must not hide a changed token contract");
        std::fs::remove_dir_all(root).unwrap();
    }

    /// Regression: the style-only branch re-locked the `last` map while the `if let` guard was alive (deadlock that
    /// silently froze HMR after the first style edit). Runs on a thread with a timeout so a deadlock fails fast.
    #[test]
    fn style_only_edits_do_not_deadlock() {
        let root = std::env::temp_dir().join(format!("brain-cfn-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("comp")).unwrap();
        std::fs::write(root.join("tokens.cssbrain"), ":root{--accent:#0062c4;--space-2:8px;--space-3:12px}").unwrap();
        let f = root.join("comp/x.brain");
        let (tx, rx) = std::sync::mpsc::channel();
        let r2 = root.clone();
        std::thread::spawn(move || {
            let cf = compile_fn(r2, true);
            let a = "<p>x</p>\n<style>p{gap:var(--space-2)}</style>";
            let b = "<p>x</p>\n<style>p{gap:var(--space-3)}</style>";
            let first = cf(a, &f).ok().unwrap();
            let second = cf(b, &f).ok().unwrap();
            let third = cf(b, &f).ok().unwrap();
            tx.send((first.js == second.js, first.css != second.css, second.css == third.css)).unwrap();
        });
        let (same_js, css_changed, noop_same) = rx.recv_timeout(std::time::Duration::from_secs(10)).expect("compile_fn deadlocked");
        assert!(same_js && css_changed && noop_same);
        let _ = std::fs::remove_dir_all(&root);
    }
}
