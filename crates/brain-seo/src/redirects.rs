//! `redirects.json` (INTERNALS §33): `[{ "from": "/old", "to": "/new", "status": 301 }]`. Sources may use `:param`
//! segments and a trailing `*` (the rest of the path, `:splat` in the target). Validated (B1825) and emitted for every
//! host: `_redirects` (Cloudflare Pages / Netlify) in `dist/`, nginx and Caddy snippets for the server configs, and a
//! JSON table for `server.mjs`. Removed URLs since the previous build are reported (B1826) with a suggested rule.

use crate::audit::norm;
use brain_lex::Json;

#[derive(Debug, Clone, PartialEq)]
pub struct Rule {
    pub from: String,
    pub to: String,
    pub status: u16,
    pub params: brain_lex::param_pattern::Params,
}

impl Rule {
    fn is_pattern(&self) -> bool {
        self.from.contains("/:") || self.from.ends_with('*')
    }
    fn external(&self) -> bool {
        self.to.starts_with("http://") || self.to.starts_with("https://")
    }
}

/// Trailing-slash policy (`brain.json` `seo.trailingSlash`): `/about/` and `/about` are one URL for crawlers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Slash {
    /// `/about/` → 301 `/about` (default).
    #[default]
    Never,
    /// `/about` → 301 `/about/` (paths whose last segment has a dot, such as `/feed.xml`, keep their form).
    Always,
    /// Both forms are served (the canonical link still names one).
    Ignore,
}

/// What the servers fold into the same single hop as a redirect rule (§33): the slash policy and, with
/// `seo.canonicalHost`, the canonical origin (`site`) every other host is sent to.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Policy {
    pub slash: Slash,
    pub host: Option<String>,
}

impl Policy {
    pub fn is_default(&self) -> bool {
        self.slash == Slash::Never && self.host.is_none()
    }
    /// The form of a page path the policy serves without a redirect, the one the sitemap and a page's own canonical
    /// name: `never` and `ignore` drop the trailing slash, `always` adds it (the root and file-like paths such as
    /// `/feed.xml` keep their form). A query or fragment stays after the path.
    pub fn path_form(&self, path: &str) -> String {
        brain_lex::url_path::page_path(path,self.slash==Slash::Always)
    }
    fn slash_name(&self) -> &'static str {
        match self.slash {
            Slash::Never => "never",
            Slash::Always => "always",
            Slash::Ignore => "ignore",
        }
    }
}

/// `(message, fix)` of one invalid rule.
pub type Problem = (String, String);

/// Parses `redirects.json`. Shape errors are returned as problems (the file is the author's).
pub fn parse(src: &str) -> Result<Vec<Rule>, Problem> {
    let j = Json::parse(src).map_err(|e| {
        (
            format!("redirects.json does not parse ({e})"),
            "write an array: [{ \"from\": \"/old\", \"to\": \"/new\", \"status\": 301 }]"
                .to_string(),
        )
    })?;
    let Json::Arr(items) = j else {
        return Err((
            "redirects.json must be an array".into(),
            "wrap the rules in [ … ]".into(),
        ));
    };
    let mut out = Vec::new();
    for (i, it) in items.iter().enumerate() {
        if it.get("params").is_some() {
            let Json::Obj(kv)=it else {unreachable!()};
            for(i,(key,_))in kv.iter().enumerate(){if kv[..i].iter().any(|(k,_)|k==key){return Err(("redirect rule with params has a duplicate key".into(),"keep each rule key once".into()));}}
        }
        let s = |k: &str| {
            it.get(k)
                .and_then(Json::as_str)
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(str::to_string)
        };
        let (Some(from), Some(to)) = (s("from"), s("to")) else {
            return Err((
                format!("rule {} needs `from` and `to`", i + 1),
                "each rule is { \"from\": \"/old\", \"to\": \"/new\" }".into(),
            ));
        };
        let status = match it.get("status") {
            None => 301,
            Some(Json::Num(n)) if [301.0, 302.0, 307.0, 308.0].contains(n) => *n as u16,
            Some(v) => {
                return Err((
                    format!("rule {} has status {}", i + 1, v.stringify()),
                    "use 301 (moved for good), 302, 307 or 308".into(),
                ));
            }
        };
        let params=it.get("params").map(|j|brain_lex::param_pattern::parse_params(j,&from,"redirect params")).transpose().map_err(|e|(e,"declare each named parameter once using a bounded pattern".into()))?.unwrap_or_default();
        out.push(Rule { from, to, status, params });
    }
    Ok(out)
}

/// The new diagnostic is opt-in only; all pattern parsing is delegated to the build/runtime rule parser.
pub fn check_constraints(root:&std::path::Path)->Result<(),String> {
    let Ok(source)=std::fs::read_to_string(root.join("redirects.json")) else{return Ok(());};
    let Ok(Json::Arr(items))=Json::parse(&source) else{return Ok(());};
    if !items.iter().any(|rule|rule.get("params").is_some()) {return Ok(());}
    parse(&source).map(|_|()).map_err(|(m,f)|format!("{m}; fix: {f}"))
}

/// Reads `<root>/redirects.json` (empty when absent).
pub fn load(root: &std::path::Path) -> Result<Vec<Rule>, Problem> {
    match std::fs::read_to_string(root.join("redirects.json")) {
        Ok(src) => parse(&src),
        Err(_) => Ok(vec![]),
    }
}

fn segs(p: &str) -> Vec<&str> {
    p.split('/').filter(|s| !s.is_empty()).collect()
}

/// Target of `path` under `rule`, if the rule matches (`:param` and `*` substituted).
pub fn apply(rule: &Rule, path: &str) -> Option<String> {
    let (pat, got) = (segs(&rule.from), segs(path));
    let splat = pat.last() == Some(&"*");
    let fixed = if splat { pat.len() - 1 } else { pat.len() };
    if got.len() < fixed || (!splat && got.len() != fixed) {
        return None;
    }
    let mut vars: Vec<(String, String)> = Vec::new();
    for (p, g) in pat[..fixed].iter().zip(&got) {
        match p.strip_prefix(':') {
            Some(name) => {
                if rule.params.get(name).is_some_and(|p|!p.matches(g)) {return None;}
                vars.push((name.to_string(), g.to_string()));
            },
            None if norm(&format!("/{p}")) == norm(&format!("/{g}")) => {}
            None => return None,
        }
    }
    if splat {
        vars.push(("splat".into(), got[fixed..].join("/")));
    }
    let mut to = rule.to.clone();
    vars.sort_by_key(|(k, _)| std::cmp::Reverse(k.len())); // `:slug` before `:s`
    for (k, v) in &vars {
        to = to.replace(&format!(":{k}"), v);
    }
    Some(to)
}

// Template-identical hops preserve captured values. Only follow constraints already guaranteed by the source;
// different predicates may make a symbolic cycle unreachable and must not produce a false error.
fn constrained_template_loop(start:&Rule,rules:&[Rule])->bool {
    let path=|to:&str|norm(to.split(['?','#']).next().unwrap_or(to));
    let mut seen=vec![norm(&start.from)];let mut current=path(&start.to);
    for _ in 0..=rules.len() {
        if seen.contains(&current) {return true;}
        let Some(next)=rules.iter().find(|r|norm(&r.from)==current)else{return false;};
        if next.external() || !next.params.iter().all(|(key,p)|start.params.get(key)==Some(p)) {return false;}
        seen.push(current);current=path(&next.to);
    }
    false
}

/// Validates the rules against the built routes. `exists(path)` answers whether a target is served.
pub fn validate(rules: &[Rule], pages: &[String], exists: &dyn Fn(&str) -> bool) -> Vec<Problem> {
    let mut out = Vec::new();
    for (i, r) in rules.iter().enumerate() {
        if !r.from.starts_with('/') {
            out.push((
                format!("`from: {}` is not a site path", r.from),
                format!("write it as \"/{}\"", r.from.trim_start_matches('/')),
            ));
            continue;
        }
        if rules[..i].iter().any(|o| norm(&o.from) == norm(&r.from)) {
            out.push((
                format!("`{}` is redirected twice", r.from),
                "keep one rule per source".into(),
            ));
        }
        if !r.is_pattern() && pages.iter().any(|p| norm(p) == norm(&r.from)) {
            out.push((
                format!("`{}` is still a page, so the redirect never runs", r.from),
                "delete or rename the view, or remove the rule".into(),
            ));
        }
        if !r.params.is_empty() && constrained_template_loop(r,rules) {
            out.push((format!("`{}` redirects in a constrained parameter loop",r.from),"point the rules at a distinct served target".into()));
        }
        if r.external() || r.to.contains(':') && r.is_pattern() {
            continue;
        }
        if !r.to.starts_with('/') {
            out.push((
                format!("`to: {}` is neither a site path nor an http(s) URL", r.to),
                "use \"/path\" or \"https://…\"".into(),
            ));
            continue;
        }
        // follow the chain from this target
        let mut seen = vec![norm(&r.from)];
        let mut cur = norm(&r.to);
        let mut hops = 0;
        while let Some(next) = rules
            .iter()
            .find(|o| !o.is_pattern() && norm(&o.from) == cur)
        {
            if seen.contains(&cur) {
                break;
            }
            seen.push(cur.clone());
            hops += 1;
            if next.external() {
                cur = next.to.clone();
                break;
            }
            cur = norm(&next.to);
        }
        if seen.contains(&cur) {
            out.push((
                format!(
                    "`{}` redirects in a loop ({} → {cur})",
                    r.from,
                    seen.join(" → ")
                ),
                "point the rules at a page that exists".into(),
            ));
        } else if hops > 0 {
            out.push((
                format!(
                    "`{}` → `{}` is a chain of {} redirects",
                    r.from,
                    r.to,
                    hops + 1
                ),
                format!("point `{}` straight to `{cur}`", r.from),
            ));
        } else if !cur.contains("://") && !exists(&cur) {
            out.push((
                format!(
                    "`{}` redirects to `{cur}`, which the build does not produce",
                    r.from
                ),
                "fix `to` or create the view".into(),
            ));
        }
    }
    out
}

/// Host files are optional hints. A restricted rule cannot be silently widened on a host with different URI semantics.
pub fn has_constraints(rules:&[Rule])->bool {rules.iter().any(|r|!r.params.is_empty())}
pub fn unsupported_hint()->String {"# Unsupported constrained redirects: use the BRAINLLM server runtime; no host rules were emitted.\n".into()}
pub fn capabilities(rules:&[Rule])->Json {
    Json::obj([("schemaVersion",Json::Num(1.0)),("runtime",Json::Bool(true)),("unsupported",Json::Arr(if has_constraints(rules){["netlify","cloudflare-pages","nginx","caddy"].into_iter().map(|name|Json::obj([("format",Json::str(name)),("reason",Json::str("raw-segment constraint equivalence is not implemented"))])).collect()}else{vec![]}))])
}
/// Explicit host export is an error rather than an empty or broader configuration.
pub fn export(rules:&[Rule],format:&str)->Result<String,String>{
    if has_constraints(rules) {return Err(format!("constrained redirects cannot be exported to {format}; fix: use the server runtime or explicit literal rules"));}
    match format {"netlify"|"cloudflare-pages"=>Ok(netlify(rules)),"nginx"=>Ok(nginx(rules)),"caddy"=>Ok(caddy(rules)),_=>Err("unknown redirect export format; fix: use nginx, caddy, netlify or cloudflare-pages".into())}
}

/// `_redirects` for Cloudflare Pages and Netlify (`*` → `:splat`).
pub fn netlify(rules: &[Rule]) -> String {
    netlify_with(rules, &Policy::default())
}

/// [`netlify`] plus the policy as a leading comment line (hosts ignore `#` lines; `brain start` reads it).
pub fn netlify_with(rules: &[Rule], policy: &Policy) -> String {
    if has_constraints(rules) {return unsupported_hint();}
    let mut o = String::new();
    if !policy.is_default() {
        o.push_str(&format!("# brain trailingSlash={}", policy.slash_name()));
        if let Some(h) = &policy.host {
            o.push_str(&format!(" host={h}"));
        }
        o.push('\n');
    }
    for r in rules {
        o.push_str(&format!("{} {} {}\n", r.from, r.to, r.status));
    }
    o
}

/// Reads a `_redirects` file written by [`netlify_with`] (the static server of `brain start`).
pub fn parse_netlify(src: &str) -> (Vec<Rule>, Policy) {
    let mut policy = Policy::default();
    let mut rules = Vec::new();
    for line in src.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if let Some(c) = line.strip_prefix("# brain ") {
            for kv in c.split_whitespace() {
                match kv.split_once('=') {
                    Some(("trailingSlash", "always")) => policy.slash = Slash::Always,
                    Some(("trailingSlash", "ignore")) => policy.slash = Slash::Ignore,
                    Some(("host", h)) if h.starts_with("http") => policy.host = Some(h.to_string()),
                    _ => {}
                }
            }
            continue;
        }
        if line.starts_with('#') {
            continue;
        }
        let mut it = line.split_whitespace();
        if let (Some(from), Some(to)) = (it.next(), it.next()) {
            let status = it.next().and_then(|s| s.parse().ok()).filter(|s| [301, 302, 307, 308].contains(s)).unwrap_or(301);
            rules.push(Rule { from: from.into(), to: to.into(), status, params:Default::default() });
        }
    }
    (rules, policy)
}

/// The single redirect a server answers for `path` + `query` (§33): the matching rule, then the slash policy on
/// its target, then the canonical host when `host` (the request's `Host`) is another non-loopback host.
/// `None` when the URL is already canonical. `(location, status)`; the query is kept unless the rule's target
/// has its own.
pub fn resolve(rules: &[Rule], policy: &Policy, path: &str, query: &str, host: Option<&str>) -> Option<(String, u16)> {
    resolve_request(rules,policy,path,path,query,host)
}
/// Dev keeps decoded legacy rule semantics while constrained sources compare the original wire path.
pub fn resolve_request(rules:&[Rule],policy:&Policy,path:&str,raw:&str,query:&str,host:Option<&str>)->Option<(String,u16)> {
    let original=if has_constraints(rules){raw}else{path};
    let (mut p, mut q, mut status, mut moved) = (original.to_string(), query.to_string(), 301, false);
    // exact sources first, then patterns (as server.mjs does)
    let hit = |pattern: bool| rules.iter().filter(|r| r.is_pattern() == pattern).find_map(|r| apply(r, if r.params.is_empty(){path}else{raw}).map(|t| (r, t)));
    if let Some((rule, to)) = hit(false).or_else(|| hit(true)) {
        if rule.external() {
            return Some((to, rule.status));
        }
        match to.split_once('?') {
            Some((a, b)) => {
                p = a.to_string();
                q = format!("?{b}");
            }
            None => p = to,
        }
        status = rule.status;
        moved = true;
    }
    if p != "/" {
        match policy.slash {
            Slash::Never if p.ends_with('/') => p = p.trim_end_matches('/').to_string(),
            Slash::Always if !p.ends_with('/') && !segs(&p).last().is_some_and(|l| l.contains('.')) => p.push('/'),
            _ => {}
        }
        if p.is_empty() {
            p = "/".into();
        }
    }
    let loopback = |h: &str| {
        let name = h.rsplit_once(':').filter(|(a, _)| !a.ends_with(']') || h.starts_with('[')).map_or(h, |(a, _)| a);
        name.is_empty() || name.eq_ignore_ascii_case("localhost") || name.starts_with("127.") || name == "[::1]"
    };
    let other = policy.host.as_deref().and_then(|origin| {
        let want = origin.split("://").nth(1)?.trim_end_matches('/');
        host.filter(|h| !loopback(h) && !h.eq_ignore_ascii_case(want)).map(|_| origin.trim_end_matches('/'))
    });
    if !moved && other.is_none() && p == original {
        return None;
    }
    Some((format!("{}{p}{q}", other.unwrap_or("")), status))
}

fn regex_of(rule: &Rule) -> (String, Vec<String>) {
    let from=&rule.from;
    let mut re = String::from("^");
    let mut names = Vec::new();
    for s in segs(from) {
        re.push('/');
        if let Some(n) = s.strip_prefix(':') {
            match rule.params.get(n) {Some(p)=>re.push_str(&format!("({})",p.regex_source())),None=>re.push_str("([^/]+)")};
            names.push(n.to_string());
        } else if s == "*" {
            re.push_str("(.*)");
            names.push("splat".into());
        } else {
            for c in s.chars() {
                if ".+?()[]{}|^$\\".contains(c) {
                    re.push('\\');
                }
                re.push(c);
            }
        }
    }
    if re == "^" {
        re.push('/');
    }
    re.push_str("/?$");
    (re, names)
}

fn target_with(to: &str, names: &[String], group: &dyn Fn(usize) -> String) -> String {
    let mut t = to.to_string();
    let mut order: Vec<(usize, &String)> = names.iter().enumerate().collect();
    order.sort_by_key(|(_, n)| std::cmp::Reverse(n.len()));
    for (i, n) in order {
        t = t.replace(&format!(":{n}"), &group(i + 1));
    }
    t
}

/// nginx `server { }` snippet.
pub fn nginx(rules: &[Rule]) -> String {
    if has_constraints(rules) {return unsupported_hint();}
    let mut o = String::new();
    for r in rules {
        if r.is_pattern() {
            let (re, names) = regex_of(r);
            o.push_str(&format!(
                "location ~ {re} {{ return {} {}; }}\n",
                r.status,
                target_with(&r.to, &names, &|i| format!("${i}"))
            ));
        } else {
            o.push_str(&format!(
                "location = {} {{ return {} {}; }}\n",
                r.from, r.status, r.to
            ));
        }
    }
    o
}

/// Caddyfile snippet (inside the site block).
pub fn caddy(rules: &[Rule]) -> String {
    if has_constraints(rules) {return unsupported_hint();}
    let mut o = String::new();
    for (i, r) in rules.iter().enumerate() {
        if r.is_pattern() {
            let (re, names) = regex_of(r);
            let n = format!("r{}", i + 1);
            o.push_str(&format!(
                "@{n} path_regexp {n} {re}\nredir @{n} {} {}\n",
                target_with(&r.to, &names, &|k| format!("{{re.{n}.{k}}}")),
                r.status
            ));
        } else {
            o.push_str(&format!("redir {} {} {}\n", r.from, r.to, r.status));
        }
    }
    o
}

/// [`server_json`] plus the policy (`slash`, `host`) when it is not the default.
pub fn server_json_with(rules: &[Rule], policy: &Policy) -> Json {
    let mut j = server_json(rules);
    if let Json::Obj(kv) = &mut j {
        if policy.slash != Slash::Never {
            kv.push(("slash".into(), Json::str(policy.slash_name())));
        }
        if let Some(h) = &policy.host {
            kv.push(("host".into(), Json::str(h)));
        }
    }
    j
}

/// Table for `server.mjs`: exact sources in a map, patterns as regex + target with `$1`.
pub fn server_json(rules: &[Rule]) -> Json {
    let exact = rules
        .iter()
        .filter(|r| !r.is_pattern())
        .map(|r| {
            (
                norm(&r.from),
                Json::Arr(vec![Json::str(&r.to), Json::Num(f64::from(r.status))]),
            )
        })
        .collect();
    let patterns = rules
        .iter()
        .filter(|r| r.is_pattern())
        .map(|r| {
            let (re, names) = regex_of(r);
            Json::obj([
                ("re", Json::str(re)),
                (
                    "to",
                    Json::str(target_with(&r.to, &names, &|i| format!("${i}"))),
                ),
                ("status", Json::Num(f64::from(r.status))),
            ])
        })
        .collect();
    Json::obj([
        ("exact", Json::Obj(exact)),
        ("patterns", Json::Arr(patterns)),
    ])
}

/// URLs in the previous build's sitemap that are neither built now nor redirected: `(old path, suggested target)`.
/// The suggestion is the current route sharing the longest last-segment prefix (renames usually keep the slug start).
pub fn removed(
    previous_sitemap: &str,
    site: &str,
    pages: &[String],
    rules: &[Rule],
) -> Vec<(String, Option<String>)> {
    let mut out = Vec::new();
    for loc in previous_sitemap
        .split("<loc>")
        .skip(1)
        .filter_map(|x| x.split("</loc>").next())
    {
        let loc = loc.replace("&amp;", "&");
        let Some(path) = loc.strip_prefix(site.trim_end_matches('/')).map(|p| {
            if p.is_empty() {
                "/".to_string()
            } else {
                p.to_string()
            }
        }) else {
            continue;
        };
        let p = norm(&path);
        if pages.iter().any(|x| norm(x) == p) || rules.iter().any(|r| apply(r, if r.params.is_empty(){&p}else{&path}).is_some()) {
            continue;
        }
        let last = segs(&p).last().copied().unwrap_or("").to_string();
        let score = |r: &String| {
            let l = segs(r).last().copied().unwrap_or("").to_string();
            let common = last
                .chars()
                .zip(l.chars())
                .take_while(|(a, b)| a == b)
                .count();
            (common, std::cmp::Reverse(r.len()))
        };
        let best = pages
            .iter()
            .filter(|r| *r != "/")
            .max_by_key(|r| score(r))
            .filter(|r| score(r).0 >= 3)
            .cloned();
        out.push((p, best));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules() -> Vec<Rule> {
        parse(r#"[{"from":"/old-pricing","to":"/pricing"},{"from":"/blog/:slug","to":"/posts/:slug","status":308},{"from":"/docs/*","to":"https://docs.acme.dev/:splat","status":302}]"#).unwrap()
    }

    #[test]
    fn parse_and_apply() {
        let r = rules();
        assert_eq!(r[0].status, 301);
        assert_eq!(apply(&r[1], "/blog/hello"), Some("/posts/hello".into()));
        assert_eq!(apply(&r[1], "/blog/hello/x"), None);
        assert_eq!(
            apply(&r[2], "/docs/a/b"),
            Some("https://docs.acme.dev/a/b".into())
        );
        assert!(
            parse(r#"[{"from":"/a","to":"/b","status":200}]"#)
                .unwrap_err()
                .0
                .contains("status 200")
        );
        assert!(parse(r#"{"from":"/a"}"#).is_err());
    }

    #[test]
    fn validation() {
        let pages = vec!["/".to_string(), "/pricing".into(), "/about".into()];
        let exists = |p: &str| pages.iter().any(|x| norm(x) == p);
        let bad = parse(r#"[{"from":"/about","to":"/pricing"},{"from":"/a","to":"/b"},{"from":"/b","to":"/pricing"},{"from":"/x","to":"/y"},{"from":"/y","to":"/x"},{"from":"/gone","to":"/nowhere"},{"from":"/a","to":"/pricing"}]"#).unwrap();
        let p: Vec<String> = validate(&bad, &pages, &exists)
            .into_iter()
            .map(|x| x.0)
            .collect();
        assert!(
            p.iter()
                .any(|m| m == "`/about` is still a page, so the redirect never runs"),
            "{p:?}"
        );
        assert!(
            p.iter()
                .any(|m| m == "`/a` → `/b` is a chain of 2 redirects"),
            "{p:?}"
        );
        assert!(
            p.iter().any(|m| m.starts_with("`/x` redirects in a loop")),
            "{p:?}"
        );
        assert!(
            p.iter()
                .any(|m| m == "`/gone` redirects to `/nowhere`, which the build does not produce"),
            "{p:?}"
        );
        assert!(p.iter().any(|m| m == "`/a` is redirected twice"), "{p:?}");
        assert!(validate(&rules(), &pages, &|p: &str| p == "/pricing").is_empty());
    }

    #[test]
    fn emitters() {
        let r = rules();
        assert_eq!(
            netlify(&r),
            "/old-pricing /pricing 301\n/blog/:slug /posts/:slug 308\n/docs/* https://docs.acme.dev/:splat 302\n"
        );
        assert_eq!(
            nginx(&r),
            "location = /old-pricing { return 301 /pricing; }\nlocation ~ ^/blog/([^/]+)/?$ { return 308 /posts/$1; }\nlocation ~ ^/docs/(.*)/?$ { return 302 https://docs.acme.dev/$1; }\n"
        );
        assert!(
            caddy(&r)
                .contains("@r2 path_regexp r2 ^/blog/([^/]+)/?$\nredir @r2 /posts/{re.r2.1} 308\n")
        );
        assert_eq!(
            server_json(&r).get("exact").unwrap().stringify(),
            r#"{"/old-pricing":["/pricing",301]}"#
        );
    }

    #[test]
    fn one_hop_rule_slash_and_host() {
        let r = parse(r#"[{"from":"/old","to":"/new/"},{"from":"/blog/:slug","to":"/posts/:slug?from=blog","status":308},{"from":"/ext","to":"https://other.dev/x","status":302}]"#).unwrap();
        let never = Policy::default();
        assert_eq!(resolve(&r, &never, "/old/", "?a=1", None), Some(("/new?a=1".into(), 301)));
        assert_eq!(resolve(&r, &never, "/blog/hi", "?a=1", None), Some(("/posts/hi?from=blog".into(), 308)));
        assert_eq!(resolve(&r, &never, "/ext", "", None), Some(("https://other.dev/x".into(), 302)));
        assert_eq!(resolve(&r, &never, "/about/", "", None), Some(("/about".into(), 301)));
        assert_eq!(resolve(&r, &never, "/about", "", None), None);
        assert_eq!(resolve(&r, &never, "/", "", None), None);
        let always = Policy { slash: Slash::Always, host: Some("https://shop.example".into()) };
        assert_eq!(resolve(&r, &always, "/about", "?x=1", Some("www.shop.example")), Some(("https://shop.example/about/?x=1".into(), 301)));
        assert_eq!(resolve(&r, &always, "/about/", "", Some("shop.example")), None);
        assert_eq!(resolve(&r, &always, "/about/", "", Some("127.0.0.1:3000")), None, "loopback is never redirected");
        assert_eq!(resolve(&r, &always, "/feed.xml", "", None), None, "files keep their form");
        let ignore = Policy { slash: Slash::Ignore, host: None };
        assert_eq!(resolve(&r, &ignore, "/about/", "", None), None);
    }

    #[test]
    fn policy_round_trips_through_redirects_file() {
        let r = rules();
        let p = Policy { slash: Slash::Always, host: Some("https://shop.example".into()) };
        let text = netlify_with(&r, &p);
        assert!(text.starts_with("# brain trailingSlash=always host=https://shop.example\n"), "{text}");
        assert_eq!(parse_netlify(&text), (r.clone(), p.clone()));
        assert_eq!(netlify_with(&r, &Policy::default()), netlify(&r), "the default adds nothing");
        assert_eq!(server_json_with(&[], &p).stringify(), r#"{"exact":{},"patterns":[],"slash":"always","host":"https://shop.example"}"#);
    }

    #[test]
    fn removed_urls_get_a_suggestion() {
        let prev = "<urlset><url><loc>https://acme.dev/</loc></url><url><loc>https://acme.dev/guides/offline-sync</loc></url><url><loc>https://acme.dev/old-pricing</loc></url><url><loc>https://acme.dev/team</loc></url></urlset>";
        let pages = vec![
            "/".to_string(),
            "/docs/offline-sync-guide".into(),
            "/pricing".into(),
        ];
        let r = removed(prev, "https://acme.dev", &pages, &rules());
        assert_eq!(
            r,
            vec![
                (
                    "/guides/offline-sync".into(),
                    Some("/docs/offline-sync-guide".into())
                ),
                ("/team".into(), None)
            ]
        );
    }
}

#[cfg(test)]
mod constraint_tests {
    use super::*;
    #[test]
    fn constrained_redirects_preserve_raw_query_slash_and_export_capabilities() {
        let rules=parse(r#"[{"from":"/:code","params":{"code":"[2-9a-z]{7}"},"to":"/items/:code","status":308}]"#).unwrap();
        assert_eq!(resolve(&rules,&Policy::default(),"/2abcdef/","?q=one",None),Some(("/items/2abcdef?q=one".into(),308)));
        for path in ["/zzz","/%32abcde","/0abcdef","/2abcdefg"] {assert_eq!(resolve(&rules,&Policy::default(),path,"",None),None);}
        assert!(has_constraints(&rules));
        for format in ["nginx","caddy","netlify","cloudflare-pages"] {assert!(export(&rules,format).unwrap_err().contains("server runtime"));}
        for output in [netlify(&rules),nginx(&rules),caddy(&rules)] {assert_eq!(output,unsupported_hint());assert!(!output.contains("/:code"));}
        assert!(matches!(capabilities(&rules).get("unsupported"),Some(Json::Arr(items)) if items.len()==4));
        assert!(server_json(&rules).stringify().contains("\\\\x32"));
        let override_query=parse(r#"[{"from":"/:id","params":{"id":"[a-z]{2}"},"to":"/item/:id?fixed=1"}]"#).unwrap();
        assert_eq!(resolve(&override_query,&Policy{slash:Slash::Always,host:Some("https://example.com".into())},"/ab","?q=one",Some("other.example")),Some(("https://example.com/item/ab/?fixed=1".into(),301)));
    }
    #[test]
    fn new_opt_in_rejects_ambiguous_or_unsupported_declarations() {
        for source in [r#"[{"from":"/:id","params":{"id":"a","id":"b"},"to":"/x"}]"#,r#"[{"from":"/:id","params":{"missing":"a"},"to":"/x"}]"#,r#"[{"from":"/:id","params":{"id":"a+"},"to":"/x"}]"#,r#"[{"from":"/:id","from":"/:other","params":{"id":"a"},"to":"/x"}]"#,r#"[{"from":"/:id/:id","params":{"id":"a"},"to":"/x"}]"#] {assert!(parse(source).is_err(),"{source}");}
        for source in [r#"[{"from":"/:id","params":{"id":"[a-z]{2}"},"to":"/:id?again=1"}]"#,r#"[{"from":"/a/:id","params":{"id":"[a-z]{2}"},"to":"/b/:id"},{"from":"/b/:id","params":{"id":"[a-z]{2}"},"to":"/a/:id"}]"#] {
            let rules=parse(source).unwrap();assert!(validate(&rules,&[],&|_|true).iter().any(|(m,_)|m.contains("loop")));
        }
        let loops=parse(r#"[{"from":"/x","to":"/y"},{"from":"/y","to":"/x"}]"#).unwrap();
        assert!(validate(&loops,&[],&|_|false).iter().any(|(m,_)|m.contains("loop")));
    }
}

#[cfg(test)]
mod wire_constraint_tests {
    use super::*;
    #[test]
    fn dev_uses_wire_only_for_explicit_rule_constraints_and_retains_wire_canonicalization() {
        let constrained=parse(r#"[{"from":"/:code","params":{"code":"[2-9a-z]{7}"},"to":"/items/:code"}]"#).unwrap();
        assert_eq!(resolve_request(&constrained,&Policy::default(),"/2abcdef","/%32abcdef","",None),None);
        assert_eq!(resolve_request(&constrained,&Policy::default(),"/2abcdef/","/%32abcdef/","?q=one",None),Some(("/%32abcdef?q=one".into(),301)));
        let legacy=parse(r#"[{"from":"/:code","to":"/items/:code"}]"#).unwrap();
        assert_eq!(resolve_request(&legacy,&Policy::default(),"/2abcdef","/%32abcdef","",None),Some(("/items/2abcdef".into(),301)));
    }
}
