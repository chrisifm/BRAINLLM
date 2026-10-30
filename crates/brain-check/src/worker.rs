//! INTERNALS §35:
//! - B0127 DOM or UI state inside a `*.worker.jsbrain` / `*.worker.js` module (it runs in a Web Worker: no
//!   `document`, `window`, storage, dialogs, signals or components).
//! - B1102 `cache: '<n>s|m|h'` of an `api()` method: only on `get`, and only in that format (edit when fixable).

use crate::security::{path, walk};
use crate::Checker;
use brain_parse::ast::*;

/// Globals and primitives a worker does not have.
const NOT_IN_WORKER: &[&str] = &[
    "document", "window", "localStorage", "sessionStorage", "alert", "confirm", "prompt", "sig", "drv", "fx", "ref", "emit",
    "form", "provide", "use", "store", "bus", "liveResource", "navigator.clipboard",
];

pub fn is_worker(file: &str) -> bool {
    file.ends_with(".worker.jsbrain") || file.ends_with(".worker.js")
}

/// `10m` → ok; `10 min`, `10 minutes`, `600sec` → the canonical spelling; anything else → `None`.
fn canonical_cache(v: &str) -> Result<(), Option<String>> {
    let t = v.trim();
    let digits = t.len() - t.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    if digits == 0 {
        return Err(None);
    }
    let (n, unit) = (&t[..digits], t[digits..].trim());
    if matches!(unit, "s" | "m" | "h") && unit.len() == t.len() - digits {
        return Ok(());
    }
    let u = match unit.to_ascii_lowercase().as_str() {
        "s" | "sec" | "secs" | "second" | "seconds" => "s",
        "m" | "min" | "mins" | "minute" | "minutes" => "m",
        "h" | "hr" | "hrs" | "hour" | "hours" => "h",
        _ => return Err(None),
    };
    Err(Some(format!("{n}{u}")))
}

impl Checker<'_> {
    pub(crate) fn worker_rules(&mut self) {
        let c = self.c;
        if is_worker(&c.file) {
            for s in &c.script {
                if let Stmt::Import(i) = s
                    && i.source.ends_with(".brain")
                {
                    self.push("B0127", i.span, format!("a worker cannot import the component `{}`", i.source), "keep components on the page; the worker only computes and returns plain data");
                }
            }
            let mut out = Vec::new();
            walk(&c.script, false, &mut |e, _, _| {
                let name = match e {
                    Expr::Ident(i) => Some(i.name.clone()),
                    Expr::Member { .. } => path(e).filter(|p| p == "navigator.clipboard"),
                    _ => None,
                };
                if let Some(n) = name
                    && NOT_IN_WORKER.contains(&n.as_str())
                {
                    out.push((n, e.span()));
                }
            });
            let mut seen = std::collections::HashSet::new();
            for (n, span) in out {
                if seen.insert(n.clone()) {
                    self.push("B0127", span, format!("`{n}` does not exist in a Web Worker"), "return plain data from the worker function and update the page (signals, DOM) where you call it");
                }
            }
        }
        // `api('/posts', { get: { out: '[Post]', cache: '10m' } })`
        let mut fixes = Vec::new();
        walk(&c.script, false, &mut |e, _, _| {
            let Expr::Call { callee, args, .. } = e else { return };
            if !matches!(callee.unparen(), Expr::Ident(i) if i.name == "api") {
                return;
            }
            let Some(Expr::Object { props, .. }) = args.get(1).map(Expr::unparen) else { return };
            for p in props {
                let ObjProp::KeyValue { key, value: Expr::Object { props: inner, .. }, .. } = p else { continue };
                let method = match key {
                    PropKey::Ident(k) => k.name.clone(),
                    PropKey::Str { value, .. } => value.clone(),
                    _ => continue,
                };
                for q in inner {
                    let ObjProp::KeyValue { key: PropKey::Ident(k), value, span } = q else { continue };
                    if k.name != "cache" {
                        continue;
                    }
                    fixes.push((method.clone(), value.clone(), *span));
                }
            }
        });
        for (method, value, span) in fixes {
            if self.ctx.cache_data_off {
                // §35: the app turned data caching off; the option is dead and the edit removes it.
                let end = self.c.src.get(span.end as usize..).map(|r| if r.trim_start().starts_with(',') { r.find(',').map(|i| i + 1).unwrap_or(0) } else { 0 }).unwrap_or(0);
                self.push_edits("B1102", span, "`cache` is ignored: brain.json sets `\"cache\": { \"data\": false }`", "remove `cache`, or turn data caching back on in brain.json", vec![(span.start, span.end + end as u32, String::new())]);
                if let Some(d) = self.diags.last_mut() {
                    d.level = brain_parse::Severity::Warning;
                }
                continue;
            }
            if method != "get" {
                let end = self.c.src.get(span.end as usize..).map(|r| if r.trim_start().starts_with(',') { r.find(',').map(|i| i + 1).unwrap_or(0) } else { 0 }).unwrap_or(0);
                self.push_edits("B1102", span, format!("`cache` on `{method}`: only GET responses are cached"), "remove `cache` (writes are never cached); put it on the `get` method", vec![(span.start, span.end + end as u32, String::new())]);
                continue;
            }
            match &value {
                Expr::Str { value: v, span: vs, raw } => match canonical_cache(v) {
                    Ok(()) => {}
                    Err(Some(fixed)) => {
                        let q = raw.chars().next().unwrap_or('\'');
                        self.push_edits("B1102", *vs, format!("cache duration `{v}`: write a number and one of s, m, h"), format!("write `cache: '{fixed}'`"), vec![(vs.start, vs.end, format!("{q}{fixed}{q}"))]);
                    }
                    Err(None) => self.push("B1102", *vs, format!("cache duration `{v}` is not valid"), "write a number and one unit: `cache: '30s'`, `'10m'` or `'2h'`"),
                },
                other => self.push("B1102", other.span(), "`cache` takes a literal duration", "write `cache: '10m'` (number + s, m or h)"),
            }
        }
    }
}
