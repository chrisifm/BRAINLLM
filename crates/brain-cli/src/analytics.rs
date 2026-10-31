//! `brain analytics` (docs/backend/analytics.md): the element map (component hash + element → `file:line`) and the
//! usage reports over the behavior events the native backend stores in the project's Postgres.
//!
//! Element ids cost zero build bytes: the collector reads the component hash every component root already carries
//! (`data-b`, a hash of the component path) plus the element's tag, accessible name, `data-track` and its ordinal among
//! same-tag elements of that component. The map recomputes those facts from the `.brain` sources, so a report resolves
//! events offline for any hash width the build used.
use brain_parse::ast::{AttrKind, AttrValue, Node};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

const INTERACTIVE: &[&str] = &["a", "button", "input", "select", "textarea", "summary", "details", "form", "video", "audio"];
const USAGE: &str = "usage: brain analytics report [--since 7d] [--json] | funnel <route> <route>… [--since 7d] | segment --viewed <route> [--min-views 2] [--within 7] [--not-viewed <route>] [--tracked <name>] [--signed-in true|false] [--limit 1000] | export [--since 7d] [--limit 10000] [--csv] | map | purge [--days 180] | erase --user <uuid>   (all take --env development)";

/// One rendered element of a component, in source order.
fn elements(nodes: &[Node], out: &mut Vec<Value>, dynamic: bool, ordinals: &mut HashMap<String, u32>) {
    for node in nodes {
        match node {
            Node::Element(e) => {
                let tag = e.tag.to_ascii_lowercase();
                let attr = |name: &str| {
                    e.attrs.iter().find(|a| a.kind == AttrKind::Plain && a.name == name).and_then(|a| match &a.value {
                        AttrValue::Text(t) => Some(t.trim().to_string()),
                        _ => None,
                    })
                };
                let handlers: Vec<&str> = e.attrs.iter().filter(|a| a.kind == AttrKind::On).map(|a| a.name.as_str()).collect();
                let text: String = e
                    .children
                    .iter()
                    .map(|c| if let Node::Text(t) = c { t.value.as_str() } else { "" })
                    .collect::<String>()
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ");
                let name = attr("aria-label")
                    .or_else(|| attr("placeholder").filter(|_| matches!(tag.as_str(), "input" | "textarea")))
                    .or_else(|| attr("title"))
                    .or_else(|| attr("alt"))
                    .or_else(|| (!text.is_empty() && !matches!(tag.as_str(), "input" | "textarea" | "select")).then(|| text.chars().take(64).collect()));
                let ordinal = ordinals.entry(tag.clone()).or_insert(0);
                let interactive = INTERACTIVE.contains(&tag.as_str())
                    || !handlers.is_empty()
                    || attr("role").is_some()
                    || attr("data-track").is_some()
                    || attr("tabindex").is_some();
                let mut row = Map::new();
                row.insert("tag".into(), tag.clone().into());
                row.insert("line".into(), e.span.line.into());
                row.insert("ordinal".into(), (*ordinal).into());
                row.insert("interactive".into(), interactive.into());
                if let Some(n) = name {
                    row.insert("name".into(), n.into());
                }
                for (key, value) in [("track", attr("data-track")), ("role", attr("role")), ("href", attr("href")), ("field", attr("name"))] {
                    if let Some(v) = value {
                        row.insert(key.into(), v.into());
                    }
                }
                if !handlers.is_empty() {
                    row.insert("handlers".into(), json!(handlers));
                }
                if dynamic {
                    row.insert("dynamic".into(), true.into());
                }
                *ordinal += 1;
                out.push(Value::Object(row));
                elements(&e.children, out, dynamic, ordinals);
            }
            Node::If(b) => {
                for branch in &b.branches {
                    elements(&branch.body, out, true, ordinals);
                }
                if let Some(o) = &b.otherwise {
                    elements(o, out, true, ordinals);
                }
            }
            Node::Each(b) => {
                elements(&b.body, out, true, ordinals);
                if let Some(o) = &b.empty {
                    elements(o, out, true, ordinals);
                }
            }
            Node::Await(b) => {
                elements(&b.pending, out, true, ordinals);
                for branch in [&b.then, &b.catch].into_iter().flatten() {
                    elements(&branch.body, out, true, ordinals);
                }
            }
            Node::SlotScope(s) => elements(&s.body, out, dynamic, ordinals),
            // a child component renders under its own `data-b`; slot content placed into it is attributed there
            Node::ComponentRef(_) | Node::Text(_) | Node::Interp(_) | Node::Slot(_) => {}
        }
    }
}

struct Component {
    file: String,
    key: String,
    elements: Vec<Value>,
}
fn components(root: &Path) -> Result<Vec<Component>, String> {
    let mut files = Vec::new();
    crate::brain_files(root, &mut files);
    let mut out = Vec::new();
    for file in files {
        let rel = file.strip_prefix(root).unwrap_or(&file).to_string_lossy().replace('\\', "/");
        let src = crate::map::read_source(root, &rel)?;
        let key = brain_codegen::component_scope_key(&rel, &src);
        let Ok(c) = brain_parse::parse(&src, Path::new(&rel)) else { continue };
        let mut rows = Vec::new();
        elements(&c.template, &mut rows, false, &mut HashMap::new());
        out.push(Component { file: rel, key, elements: rows });
    }
    Ok(out)
}

/// The element map the build writes to `.brain/analytics/elements.json` (never deployed).
pub(crate) fn element_map(root: &Path) -> Result<Value, String> {
    let width = crate::hash_lengths(root)?.values().copied().max().unwrap_or(4);
    let list: Vec<Value> = components(root)?
        .into_iter()
        .map(|c| json!({"hash": brain_codegen::component_hash_len(&c.key, width), "file": c.file, "elements": c.elements}))
        .collect();
    Ok(json!({"schemaVersion": 1, "width": width, "components": list}))
}
pub(crate) fn write_map(root: &Path) -> Result<(), String> {
    let dir = root.join(".brain/analytics");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let body = serde_json::to_string(&element_map(root)?).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("elements.json"), body + "\n").map_err(|e| e.to_string())
}

/// Resolves recorded descriptors (`{c,t,i,n,x,…}`) to source elements; any hash width matches.
pub(crate) struct Resolver {
    by_hash: HashMap<String, usize>,
    comps: Vec<Component>,
}
impl Resolver {
    pub(crate) fn new(root: &Path) -> Result<Self, String> {
        Ok(Resolver { by_hash: HashMap::new(), comps: components(root)? })
    }
    fn component(&mut self, hash: &str) -> Option<usize> {
        if let Some(i) = self.by_hash.get(hash) {
            return Some(*i);
        }
        let found = self.comps.iter().position(|c| brain_codegen::component_hash_len(&c.key, hash.len()) == hash);
        if let Some(i) = found {
            self.by_hash.insert(hash.to_string(), i);
        }
        found
    }
    /// `{file, line, tag, match}`; `match` says how sure: track · name · ordinal · only-of-tag · component.
    pub(crate) fn resolve(&mut self, el: &Value) -> Option<Value> {
        let comp = self.component(el["c"].as_str()?)?;
        let c = &self.comps[comp];
        let tag = el["t"].as_str().unwrap_or("");
        let same: Vec<&Value> = c.elements.iter().filter(|e| e["tag"] == tag).collect();
        let pick = |e: &Value, how: &str| Some(json!({"file": c.file, "line": e["line"], "tag": tag, "match": how}));
        if let Some(x) = el["x"].as_str()
            && let Some(e) = same.iter().find(|e| e["track"].as_str().is_some_and(|t| t.eq_ignore_ascii_case(x)))
        {
            return pick(e, "track");
        }
        if let Some(n) = el["n"].as_str() {
            let named: Vec<&&Value> = same.iter().filter(|e| e["name"].as_str() == Some(n)).collect();
            if named.len() == 1 {
                return pick(named[0], "name");
            }
        }
        if same.len() == 1 {
            return pick(same[0], "only-of-tag");
        }
        if let Some(e) = el["i"].as_u64().and_then(|i| same.iter().find(|e| e["ordinal"].as_u64() == Some(i)))
            && !e["dynamic"].as_bool().unwrap_or(false)
        {
            return pick(e, "ordinal");
        }
        Some(json!({"file": c.file, "line": Value::Null, "tag": tag, "match": "component"}))
    }
    /// Interactive elements of every component, for the never-used (dead UI) list.
    fn interactive(&self) -> Vec<(String, u64, String, Option<String>)> {
        let mut out = Vec::new();
        for c in &self.comps {
            for e in c.elements.iter().filter(|e| e["interactive"] == true) {
                out.push((c.file.clone(), e["line"].as_u64().unwrap_or(0), e["tag"].as_str().unwrap_or("").into(), e["name"].as_str().map(str::to_string)));
            }
        }
        out
    }
}

/// Joins the backend report with the element map: `source` per element, dead UI, totals per component.
pub(crate) fn enrich(report: &mut Value, resolver: &mut Resolver) {
    let mut used: HashSet<(String, u64)> = HashSet::new();
    let mut per_component: BTreeMap<String, (i64, i64)> = BTreeMap::new();
    if let Some(list) = report["elements"].as_array_mut() {
        for row in list.iter_mut() {
            let source = resolver.resolve(&row["element"]);
            if let Some(s) = &source {
                let file = s["file"].as_str().unwrap_or("").to_string();
                if let Some(line) = s["line"].as_u64() {
                    used.insert((file.clone(), line));
                }
                let entry = per_component.entry(file).or_default();
                entry.0 += row["interactions"].as_i64().unwrap_or(0);
                entry.1 += row["rageClicks"].as_i64().unwrap_or(0) + row["deadClicks"].as_i64().unwrap_or(0);
            }
            row["source"] = source.unwrap_or(Value::Null);
        }
    }
    let never: Vec<Value> = resolver
        .interactive()
        .into_iter()
        .filter(|(file, line, _, _)| !used.contains(&(file.clone(), *line)))
        .map(|(file, line, tag, name)| json!({"file": file, "line": line, "tag": tag, "name": name}))
        .collect();
    report["components"] = per_component
        .into_iter()
        .map(|(file, (n, frustration))| json!({"file": file, "interactions": n, "frustrationClicks": frustration}))
        .collect();
    report["neverUsed"] = Value::Array(never);
    report["notes"] = json!([
        "neverUsed lists interactive elements with no recorded interaction in the window; elements inside {#if}/{#each} may simply not have rendered",
        "source.match: track (data-track), name (accessible name), only-of-tag, ordinal (static markup) or component (element not pinned)"
    ]);
}

fn days(args: &[String], flag: &str, default: u32) -> Result<u32, String> {
    let Some(raw) = crate::flag(args, flag) else { return Ok(default) };
    raw.trim_end_matches('d').parse::<u32>().ok().filter(|d| (1..=3650).contains(d)).ok_or_else(|| format!("{flag} takes days, e.g. 7d (1..3650)"))
}

fn text_report(r: &Value) -> String {
    let mut out = String::new();
    let t = &r["totals"];
    out.push_str(&format!(
        "Last {} days: {} visitors · {} sessions · {} page views · {} events · {} signed-in users\n",
        r["windowDays"], t["visitors"], t["sessions"], t["pageViews"], t["events"], t["signedInUsers"]
    ));
    out.push_str("\nRoutes (views · visitors · avg dwell s · avg scroll % · median time to first interaction s · rage · dead)\n");
    for row in r["routes"].as_array().into_iter().flatten().take(20) {
        let s = |k: &str| row[k].as_f64().map_or("-".into(), |v| format!("{:.1}", v / 1000.0));
        out.push_str(&format!(
            "  {:<32} {:>6} {:>6} {:>7} {:>5} {:>7} {:>4} {:>4}\n",
            row["route"].as_str().unwrap_or(""),
            row["views"], row["visitors"], s("avgDwellMs"),
            row["avgScrollPct"].as_f64().map_or("-".into(), |v| format!("{v:.0}")),
            s("medianTimeToFirstInteractionMs"), row["rageClicks"], row["deadClicks"]
        ));
    }
    out.push_str("\nElements (interactions · visitors · rage · dead → source)\n");
    for row in r["elements"].as_array().into_iter().flatten().take(20) {
        let e = &row["element"];
        let src = row["source"]["file"].as_str().map_or("?".to_string(), |f| match row["source"]["line"].as_u64() {
            Some(l) => format!("{f}:{l}"),
            None => f.to_string(),
        });
        out.push_str(&format!(
            "  <{}> {:<28} {:>6} {:>6} {:>4} {:>4} → {src}\n",
            e["t"].as_str().unwrap_or("?"),
            e["n"].as_str().or(e["x"].as_str()).unwrap_or("").chars().take(28).collect::<String>(),
            row["interactions"], row["visitors"], row["rageClicks"], row["deadClicks"]
        ));
    }
    let never = r["neverUsed"].as_array().map_or(0, Vec::len);
    out.push_str(&format!("\nNever used: {never} interactive elements (see --json neverUsed)\n"));
    for row in r["neverUsed"].as_array().into_iter().flatten().take(10) {
        out.push_str(&format!("  {}:{} <{}> {}\n", row["file"].as_str().unwrap_or(""), row["line"], row["tag"].as_str().unwrap_or(""), row["name"].as_str().unwrap_or("")));
    }
    out
}

/// The enriched usage report as JSON (CLI `--json` and the `brain_analytics_report` MCP tool).
pub(crate) fn report_value(root: &Path, environment: &str, window: u32) -> Result<Value, String> {
    let url = crate::backend::database_url(root, environment)?;
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| e.to_string())?;
    rt.block_on(async {
        let pool = brain_backend::db::connect_as(&url, &crate::backend::pg_name(root, "cli")).await?;
        let mut report = brain_backend::analytics::report(&pool, window).await?;
        pool.close().await;
        enrich(&mut report, &mut Resolver::new(root)?);
        Ok(report)
    })
}

pub fn run(args: &[String]) -> Result<(), String> {
    let root = crate::app_root()?;
    let op = args.first().map(String::as_str).unwrap_or("");
    if op == "map" {
        println!("{}", element_map(&root)?);
        return Ok(());
    }
    if !matches!(op, "report" | "funnel" | "segment" | "export" | "purge" | "erase") {
        return Err(USAGE.into());
    }
    let environment = crate::env::name(args, "development");
    let url = crate::backend::database_url(&root, &environment)?;
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| e.to_string())?;
    rt.block_on(async {
        let pool = brain_backend::db::connect_as(&url, &crate::backend::pg_name(&root, "cli")).await?;
        use brain_backend::analytics as a;
        let out = match op {
            "report" => {
                let mut report = a::report(&pool, days(args, "--since", 7)?).await?;
                enrich(&mut report, &mut Resolver::new(&root)?);
                if args.iter().any(|x| x == "--json") {
                    report.to_string()
                } else {
                    text_report(&report)
                }
            }
            "funnel" => {
                let steps: Vec<String> = args[1..].iter().take_while(|s| s.starts_with('/')).cloned().collect();
                let rows = a::funnel(&pool, &steps, days(args, "--since", 7)?).await?;
                json!({"schemaVersion": 1, "steps": rows}).to_string()
            }
            "segment" => {
                let num = |f: &str| crate::flag(args, f).map(|v| v.parse::<u32>().map_err(|_| format!("{f} must be a number"))).transpose();
                let s = a::Segment {
                    viewed: crate::flag(args, "--viewed").ok_or("segment needs --viewed <route>")?,
                    min_views: num("--min-views")?,
                    within_days: num("--within")?,
                    not_viewed: crate::flag(args, "--not-viewed"),
                    tracked: crate::flag(args, "--tracked"),
                    signed_in: crate::flag(args, "--signed-in").map(|v| v == "true"),
                    limit: num("--limit")?,
                };
                json!({"schemaVersion": 1, "segment": s, "members": a::segment(&pool, &s).await?}).to_string()
            }
            "export" => {
                let limit = crate::flag(args, "--limit").map_or(Ok(10000), |v| v.parse::<u32>().map_err(|_| "--limit must be a number"))?;
                a::export(&pool, days(args, "--since", 30)?, limit, args.iter().any(|x| x == "--csv")).await?
            }
            "purge" => {
                let days = days(args, "--days", a::settings().map_or(a::DEFAULT_RETENTION_DAYS, |c| c.retention_days))?;
                let p = a::purge(&pool, days).await?;
                json!({"schemaVersion": 1, "retentionDays": days, "deleted": {"events": p.events, "sessions": p.sessions, "visitors": p.visitors}}).to_string()
            }
            _ => {
                let user = crate::flag(args, "--user").ok_or("erase needs --user <uuid>")?.parse().map_err(|_| "--user must be a UUID")?;
                json!({"schemaVersion": 1, "erasedVisitors": a::erase_user(&pool, user).await?}).to_string()
            }
        };
        pool.close().await;
        print!("{out}{}", if out.ends_with('\n') { "" } else { "\n" });
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    fn app(name: &str, files: &[(&str, &str)]) -> PathBuf {
        let root = std::env::temp_dir().join(format!("brain-analytics-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for (rel, src) in files {
            let p = root.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, src).unwrap();
        }
        std::fs::write(root.join("brain.json"), "{\"name\":\"shop\"}").unwrap();
        root
    }

    #[test]
    fn map_lists_elements_with_lines_and_resolves_any_hash_width() {
        let root = app("map", &[(
            "views/pricing.brain",
            "<main>\n  <h1>Plans</h1>\n  <button data-track=\"buy_pro\">Buy Pro</button>\n  <button>Compare</button>\n  <a href=\"/faq\">FAQ</a>\n  {#each plans as p}\n    <button on:click={pick}>{p.name}</button>\n  {/each}\n</main>\n",
        )]);
        let map = element_map(&root).unwrap();
        let comp = &map["components"][0];
        assert_eq!(comp["file"], "views/pricing.brain");
        let tags: Vec<_> = comp["elements"].as_array().unwrap().iter().map(|e| (e["tag"].as_str().unwrap().to_string(), e["line"].as_u64().unwrap())).collect();
        assert_eq!(tags, vec![("main".into(), 1), ("h1".into(), 2), ("button".into(), 3), ("button".into(), 4), ("a".into(), 5), ("button".into(), 7)]);
        let mut r = Resolver::new(&root).unwrap();
        let key = brain_codegen::component_scope_key("views/pricing.brain", &std::fs::read_to_string(root.join("views/pricing.brain")).unwrap());
        for width in [4, 6] {
            let c = brain_codegen::component_hash_len(&key, width);
            let got = r.resolve(&json!({"c": c, "t": "button", "x": "buy_pro", "i": 9})).unwrap();
            assert_eq!((got["line"].as_u64(), got["match"].as_str()), (Some(3), Some("track")));
            let got = r.resolve(&json!({"c": c, "t": "button", "n": "Compare", "i": 1})).unwrap();
            assert_eq!((got["line"].as_u64(), got["match"].as_str()), (Some(4), Some("name")));
            let got = r.resolve(&json!({"c": c, "t": "a", "n": "FAQ"})).unwrap();
            assert_eq!(got["line"].as_u64(), Some(5));
            // a button rendered by {#each}: pinned to its component, not to a wrong line
            let got = r.resolve(&json!({"c": c, "t": "button", "n": "Team", "i": 2})).unwrap();
            assert_eq!((got["line"].as_u64(), got["match"].as_str()), (None, Some("component")));
        }
        assert!(r.resolve(&json!({"c": "zzzz", "t": "button"})).is_none());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn report_gains_sources_component_totals_and_never_used_controls() {
        let root = app("report", &[("components/cart.brain", "<section>\n  <button data-track=\"checkout\">Checkout</button>\n  <button>Clear cart</button>\n</section>\n")]);
        let src = std::fs::read_to_string(root.join("components/cart.brain")).unwrap();
        let c = brain_codegen::component_hash_len(&brain_codegen::component_scope_key("components/cart.brain", &src), 4);
        let mut report = json!({"elements": [{"element": {"c": c, "t": "button", "x": "checkout", "n": "Checkout"}, "interactions": 12, "visitors": 5, "rageClicks": 2, "deadClicks": 1}]});
        enrich(&mut report, &mut Resolver::new(&root).unwrap());
        assert_eq!(report["elements"][0]["source"]["line"], 2);
        assert_eq!(report["components"], json!([{"file": "components/cart.brain", "interactions": 12, "frustrationClicks": 3}]));
        let never: Vec<_> = report["neverUsed"].as_array().unwrap().iter().map(|e| (e["line"].as_u64().unwrap(), e["name"].as_str().unwrap().to_string())).collect();
        assert_eq!(never, vec![(3, "Clear cart".to_string())]);
        assert!(text_report(&json!({"totals": {}, "routes": [], "elements": report["elements"], "neverUsed": report["neverUsed"]})).contains("components/cart.brain:2"));
        let _ = std::fs::remove_dir_all(&root);
    }
}
