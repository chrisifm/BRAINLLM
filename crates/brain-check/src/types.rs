//! INTERNALS §15: types for `api()` schemas live in the app's `types.jsbrain`
//! (`export const User = { id: 'num!', name: 'str!' }`). In a schema, `in`/`out` name them as `'User'` or `'[User]'`.
//! - B1103 a type name in an `api()` schema that neither the file nor types.jsbrain declares.

use crate::security::walk;
use crate::Checker;
use brain_parse::ast::*;
use std::collections::HashSet;
use std::path::Path;

/// `'User'` → `("User", false)`, `'[User]'` → `("User", true)`; lowercase mini-language types (`'num'`) → `None`.
pub fn type_ref(v: &str) -> Option<(String, bool)> {
    let t = v.trim();
    let (inner, list) = match t.strip_prefix('[').and_then(|x| x.strip_suffix(']')) {
        Some(x) => (x.trim(), true),
        None => (t, false),
    };
    (inner.starts_with(|c: char| c.is_ascii_uppercase()) && inner.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')).then(|| (inner.to_string(), list))
}

/// Names exported by `<root>/types.jsbrain` (`None` when the file does not exist).
pub fn exported(root: &Path) -> Option<HashSet<String>> {
    let p = root.join("types.jsbrain");
    let src = std::fs::read_to_string(&p).ok()?;
    let c = brain_parse::parse(&src, Path::new("types.jsbrain")).ok()?;
    let mut out = HashSet::new();
    for s in &c.script {
        if let Stmt::Export { decl, .. } = s {
            match decl.as_ref() {
                Stmt::Var(v) => {
                    for d in &v.decls {
                        if let Pat::Ident(i) = &d.pat {
                            out.insert(i.name.clone());
                        }
                    }
                }
                Stmt::Func(Func { name: Some(n), .. }) => {
                    out.insert(n.name.clone());
                }
                _ => {}
            }
        }
    }
    Some(out)
}

impl Checker<'_> {
    pub(crate) fn api_types(&mut self) {
        let c = self.c;
        let local: HashSet<String> = crate::scope::top_bindings(c).into_keys().collect();
        let mut found = Vec::new();
        walk(&c.script, false, &mut |e, _, _| {
            let Expr::Call { callee, args, .. } = e else { return };
            if !matches!(callee.unparen(), Expr::Ident(i) if i.name == "api") {
                return;
            }
            let Some(Expr::Object { props, .. }) = args.get(1).map(Expr::unparen) else { return };
            for p in props {
                let ObjProp::KeyValue { value: Expr::Object { props: inner, .. }, .. } = p else { continue };
                for q in inner {
                    if let ObjProp::KeyValue { key: PropKey::Ident(f), value: Expr::Str { value, span, .. }, .. } = q
                        && matches!(f.name.as_str(), "in" | "out")
                        && let Some((name, _)) = type_ref(value)
                    {
                        found.push((name, *span));
                    }
                }
            }
        });
        for (name, span) in found {
            if local.contains(&name) || self.ctx.types.as_ref().is_some_and(|t| t.contains(&name)) {
                continue;
            }
            let fix = match &self.ctx.types {
                Some(t) if !t.is_empty() => {
                    let mut names: Vec<&String> = t.iter().collect();
                    names.sort();
                    format!("declare `export const {name} = {{ id: 'num!' }}` in types.jsbrain, or use one of: {}", names.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", "))
                }
                _ => format!("create `types.jsbrain` at the app root with `export const {name} = {{ id: 'num!', name: 'str!' }}`"),
            };
            self.push("B1103", span, format!("type `{name}` is not declared"), fix);
        }
    }
}
