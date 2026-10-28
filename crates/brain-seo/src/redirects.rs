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
}

impl Rule {
    fn is_pattern(&self) -> bool {
        self.from.contains("/:") || self.from.ends_with('*')
    }
    fn external(&self) -> bool {
        self.to.starts_with("http://") || self.to.starts_with("https://")
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
        out.push(Rule { from, to, status });
    }
    Ok(out)
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
            Some(name) => vars.push((name.to_string(), g.to_string())),
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

/// `_redirects` for Cloudflare Pages and Netlify (`*` → `:splat`).
pub fn netlify(rules: &[Rule]) -> String {
    rules
        .iter()
        .map(|r| format!("{} {} {}\n", r.from, r.to, r.status))
        .collect()
}

fn regex_of(from: &str) -> (String, Vec<String>) {
    let mut re = String::from("^");
    let mut names = Vec::new();
    for s in segs(from) {
        re.push('/');
        if let Some(n) = s.strip_prefix(':') {
            re.push_str("([^/]+)");
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
    let mut o = String::new();
    for r in rules {
        if r.is_pattern() {
            let (re, names) = regex_of(&r.from);
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
    let mut o = String::new();
    for (i, r) in rules.iter().enumerate() {
        if r.is_pattern() {
            let (re, names) = regex_of(&r.from);
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
            let (re, names) = regex_of(&r.from);
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
        if pages.iter().any(|x| norm(x) == p) || rules.iter().any(|r| apply(r, &p).is_some()) {
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
