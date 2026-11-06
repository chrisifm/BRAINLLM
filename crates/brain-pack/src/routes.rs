//! The one route table (#187): `views/` files → route patterns, their canonical order and how a URL matches them.
//! The build emits it (server manifest: `path` + `rx`, already in match order), `brain run` resolves with the same
//! functions, and `brain map` lists the same patterns, so dev, build and production can never disagree.
//!
//! Files (SPEC «Files»): `views/home.brain` or `views/index.brain` → `/` · `views/a.brain` and `views/a/index.brain`
//! → `/a` · `views/users/[id].brain` → `/users/:id` (one segment) · `views/docs/[...rest].brain` → `/docs/*rest`
//! (the rest of the path, at least one segment) · `views/404.brain` → `*` · names starting with `_` are not routes
//! (`_layout`, `_guard`, `_offline` is a route only through the cache layer).
//!
//! Order: compared segment by segment, a static segment beats `:param`, which beats `*rest`; `*` (not found) is
//! always last. The first match in that order is the route.

use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteDef {
    /// `/users/:id`, `/docs/*rest`, `/` or `*`.
    pub path: String,
    /// The view, relative to the app root (`views/users/[id].brain`).
    pub view: String,
    /// Layouts from outermost to innermost (`views/_layout.brain`, `views/users/_layout.brain`).
    pub layouts: Vec<String>,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Seg {
    Static,
    Param,
    Rest,
}

/// `views/users/[id].brain` → `/users/:id`; None for files that are not routes (`_layout`, other folders, `.jsbrain`).
pub fn route_of(view_rel: &str) -> Option<String> {
    let inner = view_rel.strip_prefix("views/")?.strip_suffix(".brain")?;
    if inner.ends_with(".test") || inner.split('/').any(|s| s.starts_with('_') || s.is_empty()) {
        return None;
    }
    let parts: Vec<&str> = inner.split('/').collect();
    let mut segs: Vec<String> = Vec::new();
    for (i, s) in parts.iter().enumerate() {
        if *s == "index" && i == parts.len() - 1 {
            continue;
        }
        segs.push(match s.strip_prefix('[').and_then(|x| x.strip_suffix(']')) {
            Some(p) => match p.strip_prefix("...") {
                // a catch-all only as the last segment
                Some(r) if i == parts.len() - 1 && valid_name(r) => format!("*{r}"),
                Some(_) => return None,
                None if valid_name(p) => format!(":{p}"),
                None => return None,
            },
            None => s.to_string(),
        });
    }
    Some(match segs.as_slice() {
        [] => "/".into(),
        [h] if h == "home" => "/".into(),
        [h] if h == "404" => "*".into(),
        _ => format!("/{}", segs.join("/")),
    })
}

/// A parameter name usable as `ctx.params.<name>` and as a regex group name.
fn valid_name(n: &str) -> bool {
    let mut c = n.chars();
    c.next().is_some_and(|f| f.is_ascii_alphabetic() || f == '_' || f == '$') && c.all(|x| x.is_ascii_alphanumeric() || x == '_' || x == '$')
}

fn kinds(path: &str) -> Vec<Seg> {
    path.split('/')
        .filter(|s| !s.is_empty())
        .map(|s| if s.starts_with(':') { Seg::Param } else if s.starts_with('*') { Seg::Rest } else { Seg::Static })
        .collect()
}

/// Canonical order of two patterns (smaller = tried first).
pub fn order(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering::*;
    match (a == "*", b == "*") {
        (true, true) => return Equal,
        (true, false) => return Greater,
        (false, true) => return Less,
        _ => {}
    }
    let (ka, kb) = (kinds(a), kinds(b));
    for (x, y) in ka.iter().zip(&kb) {
        match x.cmp(y) {
            Equal => {}
            o => return o,
        }
    }
    // same kinds on the common prefix: the two can never match the same URL (a catch-all is always last), so the
    // shorter goes first only for a readable, stable order; then alphabetical
    ka.len().cmp(&kb.len()).then_with(|| a.cmp(b))
}

/// Every route of the app, in match order. When `x.brain` and `x/index.brain` give the same path, the file wins.
pub fn discover(root: &Path) -> Vec<RouteDef> {
    let mut files = Vec::new();
    walk(&root.join("views"), &mut files);
    files.sort();
    let mut out: Vec<RouteDef> = Vec::new();
    for f in files {
        let rel = f.strip_prefix(root).unwrap_or(&f).to_string_lossy().replace('\\', "/");
        let Some(path) = route_of(&rel) else { continue };
        if let Some(prev) = out.iter_mut().find(|r| r.path == path) {
            // `views/a.brain` over `views/a/index.brain`, `home` over `index` (as `brain run` always resolved)
            let better = |v: &str| !v.ends_with("/index.brain") && !v.ends_with("views/index.brain");
            if better(&rel) && !better(&prev.view) {
                prev.view = rel.clone();
                prev.layouts = layouts_of(root, &rel);
            }
            continue;
        }
        out.push(RouteDef { layouts: layouts_of(root, &rel), view: rel, path });
    }
    out.sort_by(|a, b| order(&a.path, &b.path));
    out
}

fn walk(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        let name = p.file_name().unwrap_or_default().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        if p.is_dir() {
            walk(&p, out);
        } else if name.ends_with(".brain") {
            out.push(p);
        }
    }
}

/// `views/users/[id].brain` → the `_layout.brain` files of `views/` and `views/users/` that exist, outermost first.
pub fn layouts_of(root: &Path, view_rel: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut dir = String::new();
    let parts: Vec<&str> = view_rel.split('/').collect();
    for p in &parts[..parts.len().saturating_sub(1)] {
        dir = if dir.is_empty() { p.to_string() } else { format!("{dir}/{p}") };
        let l = format!("{dir}/_layout.brain");
        if root.join(&l).is_file() {
            out.push(l);
        }
    }
    out
}

/// A pattern with params (`:id` or `*rest`): its pages come from `paths()` or from the server.
pub fn is_dynamic(pattern: &str) -> bool {
    pattern.split('/').any(|s| s.starts_with(':') || (s.starts_with('*') && s.len() > 1))
}

/// Params of `url_path` (no query) for `pattern`, or None. Trailing slash optional; segments are not decoded.
/// URL segments that start with `_` or `.` never match (private files, traversal), as `brain run` always did.
pub fn matches(pattern: &str, url_path: &str) -> Option<Vec<(String, String)>> {
    if pattern == "*" {
        return Some(vec![]);
    }
    let url: Vec<&str> = url_path.split('/').filter(|s| !s.is_empty()).collect();
    if url.iter().any(|s| s.starts_with('_') || s.starts_with('.') || s.contains('\\')) {
        return None;
    }
    let pat: Vec<&str> = pattern.split('/').filter(|s| !s.is_empty()).collect();
    let mut params = Vec::new();
    for (i, p) in pat.iter().enumerate() {
        if let Some(name) = p.strip_prefix('*') {
            if url.len() <= i {
                return None;
            }
            params.push((name.to_string(), url[i..].join("/")));
            return Some(params);
        }
        let seg = url.get(i)?;
        match p.strip_prefix(':') {
            Some(name) => params.push((name.to_string(), seg.to_string())),
            None if p == seg => {}
            None => return None,
        }
    }
    (url.len() == pat.len()).then_some(params)
}

/// The first route of `table` (already in match order) that matches, with its params.
pub fn resolve<'a>(table: &'a [RouteDef], url_path: &str) -> Option<(&'a RouteDef, Vec<(String, String)>)> {
    table.iter().find_map(|r| matches(&r.path, url_path).map(|p| (r, p)))
}

/// Load and validate the opt-in against the canonical, filename-discovered frontend route table.
pub fn constraints(root: &Path, table: &[RouteDef]) -> Result<brain_lex::param_pattern::RouteParams, String> {
    bind_constraints(read_constraints(root)?,table)
}
fn read_constraints(root:&Path)->Result<brain_lex::param_pattern::RouteParams,String> {
    let j = match std::fs::read_to_string(root.join("brain.json")) {
        Ok(src) => brain_lex::Json::parse(&src).map_err(|e|format!("brain.json: {e}"))?,
        Err(e) if e.kind()==std::io::ErrorKind::NotFound => return Ok(Default::default()),
        Err(_) => return Err("brain.json cannot be read; fix: check the app configuration file permissions".into()),
    };
    brain_lex::param_pattern::from_config(&j)
}
fn bind_constraints(params:brain_lex::param_pattern::RouteParams,table:&[RouteDef])->Result<brain_lex::param_pattern::RouteParams,String> {
    for route in params.keys() {
        if !table.iter().any(|r| &r.path==route) {return Err(format!("brain.json routeParams route `{route}` does not exist; fix: use its exact brain map path"));}
    }
    Ok(params)
}

/// Asset/runtime requests without this opt-in never walk the frontend source tree.
pub fn validate_request_constraints(root:&Path)->Result<(),String> {
    validate_request_constraints_by(root,||discover(root))
}
fn validate_request_constraints_by(root:&Path,table:impl FnOnce()->Vec<RouteDef>)->Result<(),String> {
    let params=read_constraints(root)?;
    if params.is_empty() {return Ok(());}
    bind_constraints(params,&table()).map(|_|())
}

/// `brain check` adds this new diagnostic only for an explicitly opted-in, parsed configuration.
/// Existing generic JSON diagnostics remain responsible for malformed legacy configuration.
pub fn check_constraints(root:&Path)->Result<(),String> {
    let Ok(source)=std::fs::read_to_string(root.join("brain.json")) else{return Ok(());};
    let Ok(j)=brain_lex::Json::parse(&source) else{return Ok(());};
    if j.get("routeParams").is_none() {return Ok(());}
    constraints(root,&discover(root)).map(|_|())
}

pub fn matches_with(pattern:&str,url_path:&str,constraints:&brain_lex::param_pattern::Params)->Option<Vec<(String,String)>> {
    // Reject an oversized constrained segment before the ordinary matcher clones parameter values.
    for (key,value) in pattern.split('/').filter(|s|!s.is_empty()).zip(url_path.split('/').filter(|s|!s.is_empty())) {
        if key.strip_prefix(':').and_then(|name|constraints.get(name)).is_some_and(|p|!p.matches(value)) {return None;}
    }
    let params=matches(pattern,url_path)?;
    constraints.iter().all(|(name,p)|params.iter().find(|(k,_)|k==name).is_some_and(|(_,v)|p.matches(v))).then_some(params)
}

pub fn resolve_with<'a>(table:&'a [RouteDef], constraints:&brain_lex::param_pattern::RouteParams, url_path:&str)->Option<(&'a RouteDef,Vec<(String,String)>)> {
    resolve_request(table,constraints,url_path,url_path)
}

/// Dev preserves its historical decoded dispatch for legacy candidates; an opted-in route uses the wire path.
pub fn resolve_request<'a>(table:&'a [RouteDef],constraints:&brain_lex::param_pattern::RouteParams,decoded:&str,raw:&str)->Option<(&'a RouteDef,Vec<(String,String)>)> {
    table.iter().find_map(|r| match constraints.get(&r.path).filter(|p|!p.is_empty()) {
        Some(p)=>matches_with(&r.path,raw,p),None=>matches(&r.path,decoded)
    }.map(|p|(r,p)))
}

/// JavaScript regex source for `pattern`, the same rules as `matches` (named groups = params): what server.mjs uses.
pub fn regex_source(pattern: &str) -> String {
    regex_source_with(pattern, &Default::default())
}

pub fn regex_source_with(pattern: &str, params: &brain_lex::param_pattern::Params) -> String {
    if pattern == "*" {
        return "^/.*$".into();
    }
    let mut s = String::from("^");
    let segs: Vec<&str> = pattern.split('/').filter(|s| !s.is_empty()).collect();
    for p in &segs {
        s.push('/');
        if let Some(n) = p.strip_prefix(':') {
            match params.get(n) {
                Some(p) => s.push_str(&format!("(?<{n}>(?![_.]){})",p.regex_source())),
                None => s.push_str(&format!("(?<{n}>(?![_.])[^/\\\\]+)")),
            }
        } else if let Some(n) = p.strip_prefix('*') {
            s.push_str(&format!("(?<{n}>(?![_.])[^/\\\\]+(?:/(?![_.])[^/\\\\]+)*)"));
        } else {
            for c in p.chars() {
                if ".*+?^${}()|[]\\/".contains(c) {
                    s.push('\\');
                }
                s.push(c);
            }
        }
    }
    s.push_str("/?$");
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_to_patterns() {
        assert_eq!(route_of("views/home.brain").as_deref(), Some("/"));
        assert_eq!(route_of("views/index.brain").as_deref(), Some("/"));
        assert_eq!(route_of("views/users/[id].brain").as_deref(), Some("/users/:id"));
        assert_eq!(route_of("views/users/index.brain").as_deref(), Some("/users"));
        assert_eq!(route_of("views/[org]/index.brain").as_deref(), Some("/:org"));
        assert_eq!(route_of("views/docs/[...rest].brain").as_deref(), Some("/docs/*rest"));
        assert_eq!(route_of("views/404.brain").as_deref(), Some("*"));
        assert_eq!(route_of("views/users/_layout.brain"), None);
        assert_eq!(route_of("views/home.test.brain"), None);
        assert_eq!(route_of("views/[...a]/x.brain"), None, "catch-all only last");
        assert_eq!(route_of("views/[a-b].brain"), None, "param must be an identifier");
        assert_eq!(route_of("lib/x.brain"), None);
    }

    #[test]
    fn canonical_order_and_resolution() {
        let mut v = vec!["*", "/docs/*rest", "/:a/:b", "/users/:id", "/users/new", "/", "/:slug", "/docs/intro"];
        v.sort_by(|a, b| order(a, b));
        assert_eq!(v, ["/", "/docs/intro", "/users/new", "/users/:id", "/docs/*rest", "/:slug", "/:a/:b", "*"]);
        let table: Vec<RouteDef> = v.iter().map(|p| RouteDef { path: p.to_string(), view: p.to_string(), layouts: vec![] }).collect();
        let r = |u: &str| resolve(&table, u).map(|(r, p)| (r.path.clone(), p));
        assert_eq!(r("/users/new").unwrap().0, "/users/new");
        assert_eq!(r("/users/7/").unwrap(), ("/users/:id".into(), vec![("id".into(), "7".into())]));
        assert_eq!(r("/docs/a/b").unwrap(), ("/docs/*rest".into(), vec![("rest".into(), "a/b".into())]));
        assert_eq!(r("/docs").unwrap().0, "/:slug", "a catch-all needs at least one segment");
        assert_eq!(r("/_private").unwrap().0, "*");
        assert_eq!(r("/x/y/z").unwrap().0, "*");
        // encoded segments are matched as they arrive: `%2F` stays inside one segment, values are not decoded
        assert_eq!(r("/users/a%2Fb").unwrap(), ("/users/:id".into(), vec![("id".into(), "a%2Fb".into())]));
        assert_eq!(r("/docs/a%20b/c").unwrap().1, vec![("rest".to_string(), "a%20b/c".to_string())]);
        // ranking: a static route and a one-segment param both beat the catch-all of the same folder
        let t2: Vec<RouteDef> = {
            let mut v = vec!["/docs/*rest", "/docs/:id", "/docs/intro"];
            v.sort_by(|a, b| order(a, b));
            assert_eq!(v, ["/docs/intro", "/docs/:id", "/docs/*rest"]);
            v.iter().map(|p| RouteDef { path: p.to_string(), view: p.to_string(), layouts: vec![] }).collect()
        };
        assert_eq!(resolve(&t2, "/docs/intro").unwrap().0.path, "/docs/intro");
        assert_eq!(resolve(&t2, "/docs/x").unwrap().0.path, "/docs/:id");
        assert_eq!(resolve(&t2, "/docs/x/y").unwrap().0.path, "/docs/*rest");
    }

    #[test]
    fn regex_mirrors_matches() {
        assert_eq!(regex_source("/users/:id"), "^/users/(?<id>(?![_.])[^/\\\\]+)/?$");
        assert_eq!(regex_source("/"), "^/?$");
        assert_eq!(regex_source("*"), "^/.*$");
        assert!(regex_source("/docs/*rest").contains("(?<rest>"));
    }

    #[test]
    fn discovers_with_layouts_and_prefers_files() {
        let d = std::env::temp_dir().join(format!("brain-routes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        for f in ["views/home.brain", "views/_layout.brain", "views/a.brain", "views/a/index.brain", "views/users/_layout.brain", "views/users/[id].brain", "views/404.brain", "views/users/_guard.jsbrain"] {
            let p = d.join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, "").unwrap();
        }
        let t = discover(&d);
        let paths: Vec<&str> = t.iter().map(|r| r.path.as_str()).collect();
        assert_eq!(paths, ["/", "/a", "/users/:id", "*"]);
        assert_eq!(t[1].view, "views/a.brain");
        assert_eq!(t[2].layouts, ["views/_layout.brain", "views/users/_layout.brain"]);
        let _ = std::fs::remove_dir_all(&d);
    }
}

#[cfg(test)]
mod constraint_tests {
    use super::*;
    #[test]
    fn constrained_candidates_keep_static_catchall_and_raw_encoding() {
        let mut table=vec![RouteDef{path:"/:code".into(),view:"views/[code].brain".into(),layouts:vec![]},RouteDef{path:"/2abcdef".into(),view:"views/2abcdef.brain".into(),layouts:vec![]},RouteDef{path:"/*rest".into(),view:"views/[...rest].brain".into(),layouts:vec![]}];
        table.sort_by(|a,b|order(&a.path,&b.path));
        let j=brain_lex::Json::parse(r#"{"routeParams":{"/:code":{"code":"[2-9a-z]{7}"}}}"#).unwrap();
        let constraints=brain_lex::param_pattern::from_config(&j).unwrap();
        assert_eq!(resolve_with(&table,&constraints,"/2abcdef").unwrap().0.path,"/2abcdef");
        assert_eq!(resolve_with(&table,&constraints,"/3abcdef").unwrap().0.path,"/:code");
        for path in ["/zzz","/%32abcde","/a/b"] {assert_eq!(resolve_with(&table,&constraints,path).unwrap().0.path,"/*rest");}
        for path in ["/_hidden","/.hidden","/ab\\cd"] {assert!(resolve_with(&table,&constraints,path).is_none());}
        assert_eq!(matches("/:code","/%32abcde").unwrap()[0].1,"%32abcde");
        assert!(regex_source_with("/:code",constraints.get("/:code").unwrap()).contains("\\x32-\\x39"));
        assert_eq!(regex_source_with("/:code",&Default::default()),regex_source("/:code"));
    }
    #[test]
    fn unused_constraints_never_discover_routes_during_asset_requests() {
        let root=std::env::temp_dir().join(format!("brain-constraint-no-opt-in-{}",std::process::id()));
        std::fs::create_dir_all(&root).unwrap();std::fs::write(root.join("brain.json"),"{}").unwrap();
        assert!(validate_request_constraints_by(&root,||panic!("unused route table was discovered")).is_ok());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn missing_route_is_an_actionable_configuration_error() {
        let root=std::env::temp_dir().join(format!("brain-constraint-config-{}",std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("brain.json"),r#"{"routeParams":{"/:id":{"id":"[a-z]{2}"}}}"#).unwrap();
        assert!(constraints(&root,&[]).unwrap_err().contains("does not exist"));
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod wire_constraint_tests {
    use super::*;
    #[test]
    fn request_metadata_changes_only_opted_in_candidate_matching() {
        let table=vec![RouteDef{path:"/:code".into(),view:"views/[code].brain".into(),layouts:vec![]}];
        let j=brain_lex::Json::parse(r#"{"routeParams":{"/:code":{"code":"[2-9a-z]{7}"}}}"#).unwrap();let cfg=brain_lex::param_pattern::from_config(&j).unwrap();
        assert!(resolve_request(&table,&cfg,"/2abcdef","/%32abcdef").is_none());
        assert_eq!(resolve_request(&table,&Default::default(),"/2abcdef","/%32abcdef").unwrap().1[0].1,"2abcdef");
        assert_eq!(resolve_request(&table,&cfg,"/2abcdef","/2abcdef").unwrap().1[0].1,"2abcdef");
    }
}
