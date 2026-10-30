//! Markup → `tpl(html)` + DOM wiring over the runtime (SPEC §6).
//!
//! Each fragment (component body, if/each/await branch, component children) becomes one module-level
//! `tpl("…")` whose dynamic holes are `<!>` comment anchors. All node references are taken right after
//! cloning — before any `insert` moves siblings — and then every hole is wired.

use crate::Gen;
use crate::js::js_str;
use brain_check::scope::Kind;
use brain_parse::ast::*;
use crate::inline;
use std::collections::{BTreeMap, HashMap};

const VOID: &[&str] = &["area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "source", "track", "wbr"];
/// Attributes that hold id references and are rewritten with the instance suffix (SPEC §8).
/// WebMCP declarative attributes: what apps write → what the browser reads. The spec is an origin trial (Chrome 149+);
/// when it settles, only this table changes.
const WEBMCP_ATTRS: &[(&str, &str)] = &[("tool", "toolname"), ("tool-description", "tooldescription"), ("tool-param-description", "toolparamdescription")];

const ID_REFS: &[&str] = &["for", "aria-labelledby", "aria-describedby", "aria-controls", "aria-owns", "popovertarget", "list", "form"];

enum Item<'n> {
    Text(String),
    Node(&'n Node),
}

/// Whitespace rules (rendering parity with HTML): runs containing a newline or several spaces
/// collapse to one space, whitespace at the start/end of a fragment or element disappears, and
/// `<pre>`/`<textarea>` keep everything.
fn normalize(nodes: &[Node], pre: bool) -> Vec<Item<'_>> {
    let mut out = Vec::new();
    for n in nodes {
        match n {
            Node::Text(t) => {
                if pre {
                    out.push(Item::Text(t.value.clone()));
                    continue;
                }
                let mut s = String::new();
                let mut run = String::new();
                for c in t.value.chars() {
                    if c.is_whitespace() {
                        run.push(c);
                    } else {
                        if !run.is_empty() {
                            s.push_str(if run.contains('\n') || run.len() > 1 { " " } else { &run });
                            run.clear();
                        }
                        s.push(c);
                    }
                }
                if !run.is_empty() {
                    s.push_str(if run.contains('\n') || run.len() > 1 { " " } else { &run });
                }
                // Adjacent text (e.g. around a dropped comment) merges before trimming.
                match out.last_mut() {
                    Some(Item::Text(prev)) => {
                        prev.push_str(&s);
                        if !pre && prev.ends_with("  ") {
                            prev.pop();
                        }
                    }
                    _ => out.push(Item::Text(s)),
                }
            }
            other => out.push(Item::Node(other)),
        }
    }
    if !pre {
        // Edge whitespace that is indentation (it has a newline) goes; a space on the same line is content:
        // `Pro{#if hot} <b>Most popular</b>{/if}` keeps the space before `Most` (QA #21 F6).
        let lead_nl = matches!(nodes.first(), Some(Node::Text(t)) if t.value.chars().take_while(|c| c.is_whitespace()).any(|c| c == '\n'));
        let trail_nl = matches!(nodes.last(), Some(Node::Text(t)) if t.value.chars().rev().take_while(|c| c.is_whitespace()).any(|c| c == '\n'));
        let only_ws = |t: &str| t.trim().is_empty();
        if let Some(Item::Text(t)) = out.first_mut() {
            if lead_nl || only_ws(t) && nodes.len() == 1 {
                *t = t.trim_start().to_string();
            }
        }
        if let Some(Item::Text(t)) = out.last_mut() {
            if trail_nl || only_ws(t) && nodes.len() == 1 {
                *t = t.trim_end().to_string();
            }
        }
        out.retain(|i| !matches!(i, Item::Text(t) if t.is_empty()));
    }
    out
}

fn esc_attr(v: &str) -> String {
    v.replace('"', "&quot;")
}

/// One fragment under construction.
struct Frag {
    html: String,
    /// path (child indices from the template's root list) → variable name
    vars: BTreeMap<Vec<usize>, String>,
    actions: Vec<String>,
    /// The fragment has exactly one root node, so `tpl()` returns that node itself.
    single_root: bool,
}

impl Frag {
    fn var(&mut self, g: &mut Gen, path: &[usize]) -> String {
        if let Some(v) = self.vars.get(path) {
            return v.clone();
        }
        let v = g.fresh("e");
        self.vars.insert(path.to_vec(), v.clone());
        v
    }
}

impl Gen<'_> {
    /// JS expression for an attribute value that references ids of this component, which get a per-instance
    /// suffix `$u`: ID-list attributes (`for`, `aria-*`…), `href`/`xlink:href="#id"` (SVG `use`, `textPath`) and
    /// `url(#id)` anywhere (`fill`, `stroke`, `clip-path`, `mask`, `filter`, `marker-*`, `style`). `None` when the
    /// value references none.
    pub(crate) fn id_ref(&self, name: &str, t: &str) -> Option<String> {
        if self.ids.is_empty() {
            return None;
        }
        if ID_REFS.contains(&name) {
            if !t.split_whitespace().any(|x| self.ids.contains(x)) {
                return None;
            }
            let parts: Vec<String> = t.split_whitespace().map(|x| if self.ids.contains(x) { format!("{} + $u", js_str(x)) } else { js_str(x) }).collect();
            return Some(parts.join(" + \" \" + "));
        }
        if matches!(name, "href" | "xlink:href") {
            return t.strip_prefix('#').filter(|x| self.ids.contains(*x)).map(|_| format!("{} + $u", js_str(t)));
        }
        // `url(#id)`, `url('#id')`, `url("#id")`: split the text around each local id.
        let mut parts: Vec<String> = Vec::new();
        let mut last = 0;
        let mut found = false;
        let mut i = 0;
        while let Some(off) = t[i..].find("url(") {
            let start = i + off + 4;
            let rest = &t[start..];
            let q = rest.len() - rest.trim_start_matches(['\'', '"']).len();
            let body = &rest[q..];
            i = start;
            let Some(id) = body.strip_prefix('#') else { continue };
            let len = id.find([')', '\'', '"', ' ']).unwrap_or(id.len());
            if !self.ids.contains(&id[..len]) {
                continue;
            }
            let end = start + q + 1 + len;
            parts.push(js_str(&t[last..end]));
            parts.push("$u".into());
            last = end;
            i = end;
            found = true;
        }
        if !found {
            return None;
        }
        if last < t.len() {
            parts.push(js_str(&t[last..]));
        }
        Some(parts.join(" + "))
    }

    /// JS for an attribute value computed at run time (`s`), as `() => …` when `dynamic`. An `id` or ID reference
    /// (`for`, `aria-controls`…) goes through `$sx` so the instance suffix reaches it like a static one (#308).
    fn id_wrap(&mut self, name: &str, s: String, dynamic: bool) -> String {
        let wrap = (name == "id" && self.dyn_ids) || (ID_REFS.contains(&name) && (self.dyn_ids || !self.ids.is_empty()));
        let s = if wrap {
            self.uses_sx = true;
            format!("$sx({s}, $u)")
        } else {
            s
        };
        if dynamic { format!("() => {s}") } else { s }
    }

    /// Static-or-reactive test: does evaluating `e` read reactive state?
    pub(crate) fn is_dynamic(&self, e: &Expr) -> bool {
        match e {
            Expr::Ident(id) if self.frames.last().is_some_and(|f| f.props.contains_key(&id.name)) => {
                self.frames.last().and_then(|f| f.props.get(&id.name)).is_some_and(|(_, d)| *d)
            }
            Expr::Ident(id) => match self.scopes.lookup(&id.name) {
                Some(k) => k.is_reactive_cell() || matches!(k, Kind::Prop | Kind::LoadProps | Kind::Form) || (self.node_vars.contains(&id.name) && matches!(k, Kind::Const | Kind::Let)) || (k == Kind::Import && self.channel_names.contains(&id.name)),
                None => false,
            },
            Expr::Num { .. } | Expr::Str { .. } | Expr::Bool { .. } | Expr::Null(_) | Expr::Regex { .. } | Expr::This(_) | Expr::Func(_) => false,
            // `t('key')` resolved at build is a constant (§37).
            Expr::Call { callee, args, .. }
                if matches!(callee.unparen(), Expr::Ident(i) if i.name == "t" && self.scopes.lookup("t").is_none())
                    && matches!(args.first().map(Expr::unparen), Some(Expr::Str { value, .. }) if self.opts.i18n.as_ref().is_some_and(|(_, m)| m.contains_key(value)))
                    && args.get(1).is_none_or(|a| !self.is_dynamic(a)) =>
            {
                false
            }
            Expr::Call { .. } | Expr::New { .. } | Expr::Await { .. } | Expr::Assign { .. } | Expr::Update { .. } => true,
            Expr::Template { tag, exprs, .. } => tag.is_some() || exprs.iter().any(|x| self.is_dynamic(x)),
            Expr::Array { elems, .. } => elems.iter().flatten().any(|x| self.is_dynamic(x)),
            Expr::Object { props, .. } => props.iter().any(|p| match p {
                ObjProp::KeyValue { value, key, .. } => self.is_dynamic(value) || matches!(key, PropKey::Computed(k) if self.is_dynamic(k)),
                ObjProp::Shorthand(id) => self.is_dynamic(&Expr::Ident(id.clone())),
                ObjProp::Spread { arg, .. } => self.is_dynamic(arg),
                ObjProp::Method { .. } => false,
            }),
            Expr::Unary { arg, .. } | Expr::Spread { arg, .. } | Expr::Paren { expr: arg, .. } => self.is_dynamic(arg),
            Expr::Binary { left, right, .. } => self.is_dynamic(left) || self.is_dynamic(right),
            Expr::Cond { test, cons, alt, .. } => self.is_dynamic(test) || self.is_dynamic(cons) || self.is_dynamic(alt),
            Expr::Member { obj, prop, .. } => self.is_dynamic(obj) || matches!(prop, MemberProp::Computed(p) if self.is_dynamic(p)),
            Expr::Seq { exprs, .. } => exprs.iter().any(|x| self.is_dynamic(x)),
        }
    }

    /// `() => expr` when reactive, the plain value otherwise.
    fn value_or_fn(&mut self, e: &Expr) -> String {
        let s = self.expr(e);
        if self.is_dynamic(e) { format!("() => {s}") } else { s }
    }

    /// Builds a fragment and returns the JS statements that create it, ending in `return <root>`.
    /// `root` marks fragments whose elements are roots of the component (they get `data-b`).
    pub(crate) fn fragment(&mut self, nodes: &[Node], root: bool) -> String {
        let items = normalize(nodes, false);
        let lone_anchor = items.len() == 1 && matches!(items[0], Item::Node(n) if !matches!(n, Node::Element(_)));
        let mut f = Frag { html: String::new(), vars: BTreeMap::new(), actions: Vec::new(), single_root: items.len() == 1 && !lone_anchor };
        self.items(&items, &[], &mut f, root, false);
        if lone_anchor {
            // A lone comment cannot host `insertBefore`; a second one forces a DocumentFragment.
            f.html.push_str("<!>");
        }
        if items.is_empty() {
            return format!("{}return null;\n", self.pad());
        }
        let t = self.template_decl(&f.html);
        let r = self.fresh("r");
        let p = self.pad();
        let mut out = format!("{p}const {r} = {t}();\n");
        // Declare every node reference in document order, each from its nearest declared
        // ancestor or previous sibling, before any action can move nodes.
        let paths: Vec<(Vec<usize>, String)> = f.vars.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        let mut known: BTreeMap<Vec<usize>, String> = BTreeMap::new();
        for (path, var) in &paths {
            let rel: &[usize] = if f.single_root { &path[1..] } else { path };
            let expr = if rel.is_empty() { r.clone() } else { access(&r, rel, &known, self.opts.adopt) };
            known.insert(rel.to_vec(), var.clone());
            out.push_str(&format!("{p}const {var} = {expr};\n"));
        }
        for a in &f.actions {
            out.push_str(&p);
            out.push_str(a);
            out.push('\n');
        }
        out.push_str(&format!("{p}return {r};\n"));
        out
    }

    /// `() => { …fragment… }`
    pub(crate) fn fragment_fn(&mut self, nodes: &[Node], root: bool, prelude: &[String]) -> String {
        self.indent += 1;
        let mut body = String::new();
        for l in prelude {
            body.push_str(&format!("{}{l}\n", self.pad()));
        }
        body.push_str(&self.fragment(nodes, root));
        self.indent -= 1;
        format!("{{\n{body}{}}}", self.pad())
    }

    fn items(&mut self, items: &[Item], prefix: &[usize], f: &mut Frag, root: bool, pre: bool) {
        for (idx, it) in items.iter().enumerate() {
            let mut path = prefix.to_vec();
            path.push(idx);
            match it {
                Item::Text(t) => f.html.push_str(t),
                // `<component is={C} …props>` (§41.6): a hole that remounts when `C` changes (RUNTIME brain-env.js `dyn`).
                Item::Node(n @ Node::Element(e)) if e.tag == "component" && self.scopes.lookup("component").is_none() => {
                    f.html.push_str("<!>");
                    let anchor = f.var(self, &path);
                    let code = self.dynamic_component(e, &anchor);
                    f.actions.push(format!("{}{code}", crate::srcmap::mark(n.span().line)));
                }
                Item::Node(Node::Element(e)) => self.element(e, &path, f, root, pre),
                Item::Node(n) => {
                    f.html.push_str("<!>");
                    let anchor = f.var(self, &path);
                    let code = self.hole(n, &anchor, root);
                    f.actions.push(format!("{}{code}", crate::srcmap::mark(n.span().line)));
                }
            }
        }
    }

    fn element(&mut self, e: &Element, path: &[usize], f: &mut Frag, root: bool, pre: bool) {
        // Declarative WebMCP (§32): BRAINLLM's attribute names → the ones the current spec reads.
        let renamed;
        let e = if e.attrs.iter().any(|a| WEBMCP_ATTRS.iter().any(|(ours, _)| *ours == a.name)) {
            let mut c = e.clone();
            for a in &mut c.attrs {
                if let Some((_, theirs)) = WEBMCP_ATTRS.iter().find(|(ours, _)| *ours == a.name) {
                    a.name = theirs.to_string();
                }
            }
            renamed = c;
            &renamed
        } else {
            e
        };
        let tag = e.tag.as_str();
        // §38: a `form()` that submits to a `server:` function is a real POST form; it works before the JS loads.
        let post_fn = if tag == "form" {
            e.attrs.iter().find_map(|a| match (&a.kind, &a.value) {
                (AttrKind::On, AttrValue::Expr(Expr::Member { obj, prop: MemberProp::Name(p), .. })) if a.name == "submit" && p.name == "submit" => match obj.unparen() {
                    Expr::Ident(v) => self.forms.get(&v.name).cloned(),
                    _ => None,
                },
                _ => None,
            })
        } else {
            None
        };
        // `bind:value={f.v.address.city}` on a form field → `name="address.city"` so a JS-less POST carries it.
        let field_name = if matches!(tag, "input" | "select" | "textarea") && !e.attrs.iter().any(|a| a.kind == AttrKind::Plain && a.name == "name") {
            e.attrs.iter().find_map(|a| match (&a.kind, &a.value) {
                (AttrKind::Bind, AttrValue::Expr(x)) if matches!(a.name.as_str(), "value" | "checked") => form_field(x).filter(|(v, _)| self.scopes.lookup(v) == Some(Kind::Form)).map(|(_, p)| p),
                _ => None,
            })
        } else {
            None
        };
        f.html.push('<');
        f.html.push_str(tag);
        if post_fn.is_some() && !e.attrs.iter().any(|a| a.kind == AttrKind::Plain && a.name == "method") {
            f.html.push_str(" method=\"post\"");
        }
        if let Some(n) = &field_name {
            f.html.push_str(&format!(" name=\"{}\"", esc_attr(n)));
        }
        if root {
            f.html.push_str(&format!(" data-b=\"{}\"", self.hash));
        }
        let needs_var = e.attrs.iter().any(|a| match (a.kind, &a.value) {
            (AttrKind::Plain, AttrValue::Text(t)) => (a.name == "id" && self.ids.contains(t)) || self.id_ref(&a.name, t).is_some(),
            (AttrKind::Plain, AttrValue::None) => false,
            (AttrKind::Plain, AttrValue::Expr(x)) => !matches!(x.unparen(), Expr::Str { .. } | Expr::Num { .. } | Expr::Bool { .. } | Expr::Null(_)),
            _ => true,
        });
        let el = if needs_var { f.var(self, path) } else { String::new() };
        let mut actions = Vec::new();
        for a in &e.attrs {
            match (a.kind, &a.value) {
                (AttrKind::Plain, AttrValue::Text(t)) if a.name == "id" && self.ids.contains(t) => {
                    let attr = self.rt("attr");
                    actions.push(format!("{attr}({el}, \"id\", {} + $u);", js_str(t)));
                }
                (AttrKind::Plain, AttrValue::Text(t)) if self.id_ref(&a.name, t).is_some() => {
                    let v = self.id_ref(&a.name, t).unwrap_or_default();
                    // SVG 2 `href` replaces the namespaced `xlink:href`, which `setAttribute` cannot create.
                    let name = if a.name == "xlink:href" { "href" } else { a.name.as_str() };
                    let attr = self.rt("attr");
                    actions.push(format!("{attr}({el}, {}, {v});", js_str(name)));
                }
                (AttrKind::Plain, AttrValue::Text(t)) => f.html.push_str(&format!(" {}=\"{}\"", a.name, esc_attr(t))),
                (AttrKind::Plain, AttrValue::None) => f.html.push_str(&format!(" {}", a.name)),
                (AttrKind::Plain, AttrValue::Expr(x)) => match x.unparen() {
                    Expr::Str { value, .. } => f.html.push_str(&format!(" {}=\"{}\"", a.name, esc_attr(&html_escape(value)))),
                    Expr::Num { raw, .. } => f.html.push_str(&format!(" {}=\"{raw}\"", a.name)),
                    Expr::Bool { value: true, .. } => f.html.push_str(&format!(" {}", a.name)),
                    Expr::Bool { value: false, .. } | Expr::Null(_) => {}
                    _ => {
                        let dynamic = self.is_dynamic(x);
                        let s = self.expr(x);
                        let v = self.id_wrap(&a.name, s, dynamic);
                        let attr = self.rt("attr");
                        actions.push(format!("{attr}({el}, {}, {v});", js_str(&a.name)));
                    }
                },
                (AttrKind::Plain, AttrValue::Mixed(parts)) => {
                    let mut tpl = String::from("`");
                    let mut dynamic = false;
                    for p in parts {
                        match p {
                            AttrPart::Text(t) => tpl.push_str(&t.replace('\\', "\\\\").replace('`', "\\`").replace("${", "\\${")),
                            AttrPart::Expr(x) => {
                                dynamic |= self.is_dynamic(x);
                                tpl.push_str(&format!("${{{}}}", self.expr(x)));
                            }
                        }
                    }
                    tpl.push('`');
                    let v = self.id_wrap(&a.name, tpl, dynamic);
                    let attr = self.rt("attr");
                    actions.push(format!("{attr}({el}, {}, {v});", js_str(&a.name)));
                }
                (AttrKind::Class, AttrValue::Expr(x)) => {
                    let v = self.value_or_fn(x);
                    let cls = self.rt("cls");
                    actions.push(format!("{cls}({el}, {}, {v});", js_str(&a.name)));
                }
                (AttrKind::Style, AttrValue::Expr(x)) => {
                    let v = self.value_or_fn(x);
                    let attr = self.rt("attr");
                    actions.push(format!("{attr}({el}, {}, {v});", js_str(&format!("style:{}", a.name))));
                }
                (AttrKind::Style, AttrValue::Text(t)) => {
                    let attr = self.rt("attr");
                    actions.push(format!("{attr}({el}, {}, {});", js_str(&format!("style:{}", a.name)), js_str(t)));
                }
                // §41.6: in on connect, out before `when`/`list` remove the node (RUNTIME brain-anim.js).
                (AttrKind::Transition, v) => {
                    let opts = match v {
                        AttrValue::Expr(x) => format!(", {}", self.expr(x)),
                        _ => String::new(),
                    };
                    actions.push(format!("transition({el}, {}{opts});", js_str(&a.name)));
                }
                (AttrKind::On, AttrValue::Expr(x)) => {
                    let h = self.handler(x, a.name == "submit");
                    let on = self.rt("on");
                    actions.push(format!("{on}({el}, {}, {h});", js_str(&a.name)));
                }
                (AttrKind::Bind, AttrValue::Expr(x)) => {
                    let target = self.expr(x);
                    let checked = self.bind_write(x, "e.target.checked");
                    let value = self.bind_write(x, "e.target.value");
                    let (attr, on) = (self.rt("attr"), self.rt("on"));
                    match a.name.as_str() {
                        "ref" => actions.push(format!("{target} = {el};")),
                        "checked" => {
                            actions.push(format!("{attr}({el}, \"checked\", () => {target});"));
                            actions.push(format!("{on}({el}, \"change\", (e) => {{ {checked}; }});"));
                        }
                        _ => {
                            let ev = if tag == "select" { "change" } else { "input" };
                            actions.push(format!("{attr}({el}, \"value\", () => {target});"));
                            actions.push(format!("{on}({el}, \"{ev}\", (e) => {{ {value}; }});"));
                        }
                    }
                }
                _ => {}
            }
        }
        f.html.push('>');
        let at = crate::srcmap::mark(e.span.line);
        f.actions.extend(actions.into_iter().map(|a| format!("{at}{a}")));
        let lower = tag.to_ascii_lowercase();
        if VOID.contains(&lower.as_str()) {
            return;
        }
        let pre = pre || lower == "pre" || lower == "textarea";
        let items = normalize(&e.children, pre);
        self.items(&items, path, f, false, pre);
        // Last child, so the paths to the authored children stay the same.
        if let Some(func) = &post_fn {
            f.html.push_str(&format!("<input type=\"hidden\" name=\"_brain\" value=\"{}/{func}\">", self.hash));
        }
        f.html.push_str(&format!("</{tag}>"));
    }

    /// Write side of `bind:`: `n = v`, or for a field of a sig object an immutable copy:
    /// `bind:value={form.user.name}` → `form = { ...form, user: { ...form.user, name: v } }`.
    fn bind_write(&mut self, x: &Expr, val: &str) -> String {
        // `form()` fields are reactive setters: `f.v.email = v`.
        if root_of(x).is_some_and(|r| self.scopes.lookup(r) == Some(Kind::Form)) {
            return format!("{} = {val}", self.expr(x));
        }
        match x.unparen() {
            Expr::Member { obj, prop, .. } => {
                let o = self.expr(obj);
                let key = match prop {
                    MemberProp::Name(n) => n.name.clone(),
                    MemberProp::Computed(k) => format!("[{}]", self.expr(k)),
                };
                self.bind_write(obj, &format!("{{ ...{o}, {key}: {val} }}"))
            }
            other if self.bindable_prop(other).is_some() => {
                let name = self.bindable_prop(other).unwrap_or_default();
                format!("$p[{}]?.({val})", js_str(&format!("bind:{name}")))
            }
            other => format!("{} = {val}", self.expr(other)),
        }
    }

    /// Event handler (SPEC §3): a bare identifier (or member/function) is a function reference;
    /// anything else is a statement. `on:submit` calls `preventDefault()` first.
    fn handler(&mut self, x: &Expr, submit: bool) -> String {
        let is_ref = matches!(x.unparen(), Expr::Ident(_) | Expr::Member { .. } | Expr::Func(_));
        let body = if is_ref {
            let f = self.expr(x);
            if !submit {
                return f;
            }
            format!("({f})(e)")
        } else {
            self.expr(x)
        };
        if submit { format!("(e) => {{ e.preventDefault(); {body}; }}") } else { format!("() => {{ {body}; }}") }
    }

    /// Code for one dynamic hole at `anchor`.
    fn hole(&mut self, n: &Node, anchor: &str, root: bool) -> String {
        match n {
            Node::Interp(i) if i.html => {
                // `html` sets the content of an element: a `display: contents` host keeps the layout untouched.
                let v = self.expr(&i.expr);
                let h = self.fresh("h");
                format!(
                    "const {h} = document.createElement(\"b-html\"); {h}.style.display = \"contents\"; {}(null, {h}, {anchor}); html({h}, () => {v});",
                    self.rt("insert")
                )
            }
            Node::Interp(i) => {
                let v = self.value_or_fn(&i.expr);
                format!("{}(null, {v}, {anchor});", self.rt("insert"))
            }
            Node::Slot(s) if s.name.is_none() && !self.frames.is_empty() => {
                // Default slot of an inlined component: the parent's children, generated in the parent's context.
                let Some(frame) = self.frames.pop() else { return String::new() };
                let child_hash = std::mem::replace(&mut self.hash, frame.parent_hash.clone());
                let child_ids = std::mem::replace(&mut self.ids, frame.parent_ids.clone());
                let code = if normalize(&frame.children, false).is_empty() {
                    String::new()
                } else {
                    let f = self.fragment_fn(&frame.children, true, &[]);
                    format!("{}(null, (() => {f})(), {anchor});", self.rt("insert"))
                };
                self.hash = child_hash;
                self.ids = child_ids;
                self.frames.push(frame);
                code
            }
            Node::Slot(s) => {
                let key = match &s.name {
                    Some(n) => format!("$p[{}]", js_str(&format!("slot:{n}"))),
                    None => "$p.children".to_string(),
                };
                // Slot props (§41.6): getters, so the parent's `{#slot item}` stays reactive.
                let props: Vec<String> = s
                    .attrs
                    .iter()
                    .filter_map(|a| match &a.value {
                        AttrValue::Expr(x) => Some(format!("get {}() {{ return {}; }}", js_str(&a.name), self.expr(x))),
                        AttrValue::Text(t) => Some(format!("{}: {}", js_str(&a.name), js_str(t))),
                        AttrValue::None => Some(format!("{}: true", js_str(&a.name))),
                        AttrValue::Mixed(_) => None,
                    })
                    .collect();
                let arg = if props.is_empty() { String::new() } else { format!("{{ {} }}", props.join(", ")) };
                format!("{}(null, {key} ? {key}({arg}) : null, {anchor});", self.rt("insert"))
            }
            // Only valid as the default slot of a component (handled by `component_call`); the checker reports B0218.
            Node::SlotScope(_) => String::new(),
            // `<Chart lazy />`, `lazy="idle"`, `lazy="interaction"` (§22): a placeholder the runtime swaps for the
            // component once the trigger fires; the chunk is imported then, not with the page.
            Node::ComponentRef(r) if r.attrs.iter().any(|a| a.kind == AttrKind::Plain && a.name == "lazy") && self.lazy_src.contains_key(&r.name) => {
                let mode = match r.attrs.iter().find(|a| a.kind == AttrKind::Plain && a.name == "lazy").map(|a| &a.value) {
                    Some(AttrValue::Text(t)) if matches!(t.as_str(), "idle" | "interaction") => t.clone(),
                    _ => "visible".to_string(),
                };
                let src = self.lazy_src[&r.name].clone();
                let rest = ComponentRef { name: String::new(), attrs: r.attrs.iter().filter(|a| a.name != "lazy").cloned().collect(), children: r.children.clone(), span: r.span };
                let props = self.component_call(&rest);
                let el = self.fresh("l");
                let tag = if mode == "interaction" { "button" } else { "div" };
                let setup = if mode == "interaction" {
                    format!("{el}.setAttribute(\"type\", \"button\"); {el}.textContent = {};", js_str("Load component"))
                } else { format!("{el}.setAttribute(\"role\", \"status\"); {el}.textContent = \"Loading component…\";") };
                format!(
                    "const {el} = document.createElement(\"{tag}\"); {el}.setAttribute(\"data-lazy\", \"\"); {setup} {}(null, {el}, {anchor}); lazy({el}, () => import({}), {}, {});",
                    self.rt("insert"),
                    js_str(&src),
                    js_str(&mode),
                    props.trim_start_matches('(').trim_end_matches(')')
                )
            }
            // `<Portal to="#layer">` (§41.6): the children render into the target and keep this component's scope.
            Node::ComponentRef(r) if r.name == "Portal" && self.scopes.lookup("Portal").is_none() => {
                let to = match r.attrs.iter().find(|a| a.kind == AttrKind::Plain && a.name == "to").map(|a| &a.value) {
                    Some(AttrValue::Text(t)) => js_str(t),
                    Some(AttrValue::Expr(x)) => self.expr(x),
                    _ => "null".to_string(),
                };
                self.indent += 1;
                let f = self.fragment_fn(&r.children, true, &[]);
                self.indent -= 1;
                format!("{}(null, portal({to}, () => {f}), {anchor});", self.rt("insert"))
            }
            Node::ComponentRef(r) if self.inline.get(&r.name).is_some_and(|i| inline::use_ok(r, &i.comp)) => self.inline_use(r, anchor),
            Node::ComponentRef(r) => {
                let call = self.component_call(r);
                format!("{}(null, {call}, {anchor});", self.rt("insert"))
            }
            Node::If(b) => self.if_block(&b.branches, b.otherwise.as_deref(), anchor, root),
            Node::Each(b) => self.each_block(b, anchor, root),
            Node::Await(b) => self.await_block(b, anchor, root),
            Node::Element(_) | Node::Text(_) => unreachable!(),
        }
    }

    fn if_block(&mut self, branches: &[IfBranch], otherwise: Option<&[Node]>, anchor: &str, root: bool) -> String {
        let when = self.rt("when");
        let first = &branches[0];
        let test = self.expr(&first.test);
        let a = self.fragment_fn(&first.body, root, &[]);
        let b = if branches.len() > 1 {
            // `{:else if}` → a nested `when` inside the else fragment.
            self.indent += 1;
            let r = self.fresh("r");
            let t = self.template_decl("<!><!>");
            let inner_anchor = if self.opts.adopt { format!("fc({r})") } else { format!("{r}.firstChild") };
            let inner = self.if_block(&branches[1..], otherwise, &inner_anchor, root);
            let p = self.pad();
            self.indent -= 1;
            format!("() => {{\n{p}const {r} = {t}();\n{p}{inner}\n{p}return {r};\n{}}}", self.pad())
        } else if let Some(o) = otherwise {
            format!("() => {}", self.fragment_fn(o, root, &[]))
        } else {
            "null".into()
        };
        format!("{when}(null, () => {test}, () => {a}, {b}, {anchor});")
    }

    fn each_block(&mut self, b: &EachBlock, anchor: &str, root: bool) -> String {
        let list = self.rt("list");
        let items = self.expr(&b.list);
        let idx = b.index.as_ref().map(|i| i.name.clone());
        // keyFn receives raw values.
        self.scopes.push();
        self.scopes.declare_pat(&b.item, Kind::Local);
        if let Some(i) = &idx {
            self.scopes.declare(i, Kind::Local);
        }
        let key_params = match &idx {
            Some(i) => format!("{}, {i}", self.pat(&b.item)),
            None => self.pat(&b.item),
        };
        let key = match &b.key {
            Some(k) => self.expr(k),
            None => "undefined".into(),
        };
        self.scopes.pop();
        // render receives signals.
        self.scopes.push();
        let mut prelude = Vec::new();
        let item_param = match &b.item {
            Pat::Ident(id) => {
                self.scopes.declare(&id.name, Kind::EachItem);
                id.name.clone()
            }
            pat => {
                let s = self.fresh("s");
                let drv = self.rt("drv");
                let mut names = Vec::new();
                pat.bound_names(&mut names);
                let p = self.pat(pat);
                for n in names {
                    prelude.push(format!("const {0} = {drv}(() => {{ const {p} = {s}.v; return {0}; }});", n.name));
                    self.scopes.declare(&n.name, Kind::Drv);
                }
                s
            }
        };
        if let Some(i) = &idx {
            self.scopes.declare(i, Kind::EachItem);
        }
        let render_params = match &idx {
            Some(i) => format!("{item_param}, {i}"),
            None => item_param,
        };
        let body = self.fragment_fn(&b.body, root, &prelude);
        self.scopes.pop();
        let empty = match &b.empty {
            Some(e) => format!(", () => {}", self.fragment_fn(e, root, &[])),
            None => String::new(),
        };
        format!("{list}(null, () => {items}, ({key_params}) => {key}, ({render_params}) => {body}, {anchor}{empty});")
    }

    /// `{#await}` compiled with sig + fx + when (RUNTIME keeps no awaitBlock).
    fn await_block(&mut self, b: &AwaitBlock, anchor: &str, root: bool) -> String {
        let (sig, fx, batch, when) = (self.rt("sig"), self.rt("fx"), self.rt("batch"), self.rt("when"));
        let st = self.fresh("st");
        let val = self.fresh("av");
        let tok = self.fresh("tk");
        let promise = self.expr(&b.promise);
        let p = self.pad();
        let mut out = format!("const {st} = {sig}(0), {val} = {sig}();\n{p}let {tok} = 0;\n");
        // §42: without `{:catch}` a rejection reaches the nearest component boundary instead of rendering nothing.
        let escalate = if b.catch.is_none() {
            let rt = self.rt("_rt");
            let ow = self.fresh("ow");
            out.push_str(&format!("{p}const {ow} = {rt}().owner();\n"));
            format!("; if (t === {tok}) {rt}().oops(e, {ow}, 'resource')")
        } else {
            String::new()
        };
        out.push_str(&format!(
            "{p}{fx}(() => {{ const t = ++{tok}; {st}.v = 0; Promise.resolve({promise}).then((v) => {{ if (t === {tok}) {batch}(() => {{ {val}.v = v; {st}.v = 1; }}); }}, (e) => {{ if (t === {tok}) {batch}(() => {{ {val}.v = e; {st}.v = 2; }}){escalate}; }}); }});\n"
        ));
        let pending = self.fragment_fn(&b.pending, root, &[]);
        let branch = |g: &mut Self, br: &Option<AwaitBranch>| -> String {
            match br {
                None => "null".into(),
                Some(br) => {
                    g.scopes.push();
                    let mut prelude = Vec::new();
                    match &br.binding {
                        Some(Pat::Ident(id)) => {
                            prelude.push(format!("const {} = {val};", id.name));
                            g.scopes.declare(&id.name, Kind::EachItem);
                        }
                        Some(pat) => {
                            let drv = g.rt("drv");
                            let mut names = Vec::new();
                            pat.bound_names(&mut names);
                            let pp = g.pat(pat);
                            for n in names {
                                prelude.push(format!("const {0} = {drv}(() => {{ const {pp} = {val}.v; return {0}; }});", n.name));
                                g.scopes.declare(&n.name, Kind::Drv);
                            }
                        }
                        None => {}
                    }
                    let f = g.fragment_fn(&br.body, root, &prelude);
                    g.scopes.pop();
                    format!("() => {f}")
                }
            }
        };
        let then = branch(self, &b.then);
        let catch = branch(self, &b.catch);
        // pending | settled → then/catch
        self.indent += 1;
        let r = self.fresh("r");
        let t = self.template_decl("<!><!>");
        let pi = self.pad();
        self.indent -= 1;
        let first = if self.opts.adopt { format!("fc({r})") } else { format!("{r}.firstChild") };
        let settled = format!(
            "() => {{\n{pi}const {r} = {t}();\n{pi}{when}(null, () => {st}.v === 1, {then}, {catch}, {first});\n{pi}return {r};\n{p}}}"
        );
        out.push_str(&format!("{p}{when}(null, () => {st}.v === 0, () => {pending}, {settled}, {anchor});"));
        out
    }

    /// `Child({ get x() { return … }, y: "lit", "on:evt": handler, children: () => … })`
    /// Inlined `ui/` component (INTERNALS §13): its fragment is built right here, with the child's scope hash on
    /// its roots and every prop read replaced by the parent's expression.
    fn inline_use(&mut self, r: &ComponentRef, anchor: &str) -> String {
        let Some(inl) = self.inline.get(&r.name) else { return String::new() };
        let (comp, child_hash, source) = (inl.comp.clone(), inl.hash.clone(), inl.source.clone());
        let mut props = HashMap::new();
        for p in &comp.props {
            let given = r.attrs.iter().find(|a| a.name == p.name);
            let v = match given.map(|a| &a.value) {
                Some(AttrValue::Text(t)) => (js_str(t), false),
                Some(AttrValue::None) => ("true".to_string(), false),
                Some(AttrValue::Expr(x)) => (self.expr(x), self.is_dynamic(x)),
                Some(AttrValue::Mixed(parts)) => {
                    let mut tpl = String::from("`");
                    let mut dynamic = false;
                    for part in parts {
                        match part {
                            AttrPart::Text(t) => tpl.push_str(&t.replace('\\', "\\\\").replace('`', "\\`").replace("${", "\\${")),
                            AttrPart::Expr(x) => {
                                dynamic |= self.is_dynamic(x);
                                tpl.push_str(&format!("${{{}}}", self.expr(x)));
                            }
                        }
                    }
                    tpl.push('`');
                    (tpl, dynamic)
                }
                None => match &p.default {
                    Some(d) => (self.expr(d), false),
                    None => ("undefined".to_string(), false),
                },
            };
            props.insert(p.name.clone(), v);
        }
        self.inlined_sources.insert(source);
        let parent_hash = std::mem::replace(&mut self.hash, child_hash);
        let parent_ids = std::mem::take(&mut self.ids);
        self.frames.push(inline::Frame { props, children: r.children.clone(), parent_hash, parent_ids });
        self.indent += 1;
        let body = self.fragment(&comp.template, true);
        self.indent -= 1;
        let frame = self.frames.pop();
        if let Some(f) = frame {
            self.hash = f.parent_hash;
            self.ids = f.parent_ids;
        }
        // `fragment` ends in `return <root>;`: mount it at the anchor instead.
        let body = match body.rfind("return ") {
            Some(i) => {
                let root = body[i + 7..].trim_end().trim_end_matches(';').to_string();
                if root == "null" {
                    return String::new();
                }
                format!("{}{}(null, {root}, {anchor});\n", &body[..i], self.rt("insert"))
            }
            None => body,
        };
        format!("{{\n{body}{}}}", self.pad())
    }

    fn dynamic_component(&mut self, e: &Element, anchor: &str) -> String {
        let is = e.attrs.iter().find_map(|a| match (&a.kind, &a.value) {
            (AttrKind::Plain, AttrValue::Expr(x)) if a.name == "is" => Some(x.clone()),
            _ => None,
        });
        let c = is.map(|x| self.expr(&x)).unwrap_or_else(|| "null".into());
        let r = ComponentRef { name: String::new(), attrs: e.attrs.iter().filter(|a| a.name != "is").cloned().collect(), children: e.children.clone(), span: e.span };
        let props = self.component_call(&r);
        format!("dyn(null, () => {c}, {}, {anchor});", props.trim_start_matches('(').trim_end_matches(')'))
    }

    fn component_call(&mut self, r: &ComponentRef) -> String {
        let mut props = Vec::new();
        if r.name == "Dialog" {
            let ordinal=self.dialog_index;self.dialog_index+=1;
            if !r.attrs.iter().any(|a|a.name=="urlKey") {
                let identity=r.attrs.iter().find_map(|a|match (&a.kind,a.name.as_str(),&a.value) {
                    (AttrKind::Bind,"ref",AttrValue::Expr(Expr::Ident(id)))=>Some(format!("ref.{}",id.name)),
                    (AttrKind::Plain,"id",AttrValue::Text(id))=>Some(format!("id.{id}")),
                    _=>None,
                }).unwrap_or_else(||ordinal.to_string());
                props.push(format!("urlKey: {}",js_str(&format!("ui.dialog.{}.{}",self.url_namespace,identity))));
            }
        }
        for a in &r.attrs {
            match (a.kind, &a.value) {
                (AttrKind::Plain, AttrValue::Text(t)) => props.push(format!("{}: {}", js_str(&a.name), js_str(t))),
                (AttrKind::Plain, AttrValue::None) => props.push(format!("{}: true", js_str(&a.name))),
                (AttrKind::Plain, AttrValue::Expr(x)) => {
                    let v = self.expr(x);
                    if self.is_dynamic(x) {
                        props.push(format!("get {}() {{ return {v}; }}", js_str(&a.name)));
                    } else {
                        props.push(format!("{}: {v}", js_str(&a.name)));
                    }
                }
                (AttrKind::Plain, AttrValue::Mixed(parts)) => {
                    let mut tpl = String::from("`");
                    for p in parts {
                        match p {
                            AttrPart::Text(t) => tpl.push_str(&t.replace('`', "\\`").replace("${", "\\${")),
                            AttrPart::Expr(x) => tpl.push_str(&format!("${{{}}}", self.expr(x))),
                        }
                    }
                    tpl.push('`');
                    props.push(format!("get {}() {{ return {tpl}; }}", js_str(&a.name)));
                }
                // `bind:api={m}` receives the child's exposed API (null on unmount).
                (AttrKind::Bind, AttrValue::Expr(x)) if a.name == "api" => {
                    let w = self.bind_write(x, "a");
                    props.push(format!("\"bind:api\": (a) => {{ {w}; }}"));
                }
                // `bind:value={n}` on a component: getter + the setter the child calls when it writes its `~` prop.
                (AttrKind::Bind, AttrValue::Expr(x)) if a.name != "ref" => {
                    let read = self.expr(x);
                    let w = self.bind_write(x, "v");
                    props.push(format!("get {}() {{ return {read}; }}", js_str(&a.name)));
                    props.push(format!("{}: (v) => {{ {w}; }}", js_str(&format!("bind:{}", a.name))));
                }
                (AttrKind::On, AttrValue::Expr(x)) => {
                    let is_ref = matches!(x.unparen(), Expr::Ident(_) | Expr::Member { .. } | Expr::Func(_));
                    let h = if is_ref { self.expr(x) } else { format!("() => {{ {}; }}", self.expr(x)) };
                    props.push(format!("{}: {h}", js_str(&format!("on:{}", a.name))));
                }
                _ => {}
            }
        }
        // Children with `slot="x"` fill `<slot name="x" />` (passed as `"slot:x"`); the rest is the default slot.
        let mut named: Vec<(String, Vec<Node>)> = Vec::new();
        let mut rest = Vec::new();
        for n in &r.children {
            let slot = match n {
                Node::Element(e) => e.attrs.iter().position(|a| a.kind == AttrKind::Plain && a.name == "slot").map(|i| (i, &e.attrs[i])),
                _ => None,
            };
            match (n, slot) {
                (Node::Element(e), Some((i, a))) => {
                    let name = match &a.value {
                        AttrValue::Text(t) => t.clone(),
                        _ => String::new(),
                    };
                    let mut e = e.clone();
                    e.attrs.remove(i);
                    match named.iter_mut().find(|(k, _)| *k == name) {
                        Some((_, v)) => v.push(Node::Element(e)),
                        None => named.push((name, vec![Node::Element(e)])),
                    }
                }
                _ => rest.push(n.clone()),
            }
        }
        for (name, nodes) in &named {
            self.indent += 1;
            // Slot content keeps the scope of the component that wrote it (INTERNALS §7): the child's CSS never reaches it.
            let f = self.fragment_fn(nodes, true, &[]);
            self.indent -= 1;
            props.push(format!("{}: () => {f}", js_str(&format!("slot:{name}"))));
        }
        if let Some(Node::SlotScope(b)) = rest.iter().find(|n| matches!(n, Node::SlotScope(_))) {
            // `{#slot item}…{/slot}`: the child calls `children({ get item() {…} })`; each name is a read-only drv.
            self.indent += 1;
            self.scopes.push();
            let drv = self.rt("drv");
            let mut prelude = Vec::new();
            for n in &b.names {
                prelude.push(format!("const {0} = {drv}(() => $s?.[{1}]);", n.name, js_str(&n.name)));
                self.scopes.declare(&n.name, Kind::Drv);
            }
            let f = self.fragment_fn(&b.body, true, &prelude);
            self.scopes.pop();
            self.indent -= 1;
            props.push(format!("children: ($s) => {f}"));
        } else if !normalize(&rest, false).is_empty() {
            self.indent += 1;
            let f = self.fragment_fn(&rest, true, &[]);
            self.indent -= 1;
            props.push(format!("children: () => {f}"));
        }
        format!("{}({{ {} }})", r.name, props.join(", "))
    }
}

/// `f.v.address.city` → (`f`, `address.city`): the field path of a `form()` binding.
fn form_field(x: &Expr) -> Option<(String, String)> {
    let mut path = Vec::new();
    let mut e = x.unparen();
    loop {
        match e {
            Expr::Member { obj, prop: MemberProp::Name(p), .. } => {
                path.push(p.name.clone());
                e = obj.unparen();
            }
            Expr::Ident(v) => {
                path.reverse();
                return (path.first().map(String::as_str) == Some("v") && path.len() > 1).then(|| (v.name.clone(), path[1..].join(".")));
            }
            _ => return None,
        }
    }
}

/// Builds `base.firstChild.nextSibling…` for `rel`, starting from the nearest known ancestor or
/// previous sibling.
/// With `adopt` (#143) the same walk uses `fc(…)`/`nx(…)` from brain-hydrate.js.
fn access(root: &str, rel: &[usize], known: &BTreeMap<Vec<usize>, String>, adopt: bool) -> String {
    let next = |mut e: String, n: usize| {
        if adopt && n > 0 {
            let mut out = String::new();
            for _ in 0..n { out.push_str("nx("); }
            out.push_str(&e);
            for _ in 0..n { out.push(')'); }
            out
        } else {
            // Append once per hop; repeatedly formatting the growing path copies its entire prefix each time.
            for _ in 0..n { e.push_str(".nextSibling"); }
            e
        }
    };
    let (parent, last) = rel.split_at(rel.len() - 1);
    let last = last[0];
    for j in (0..last).rev() {
        let mut sib = parent.to_vec();
        sib.push(j);
        if let Some(v) = known.get(&sib) {
            return next(v.clone(), last - j);
        }
    }
    let base = if parent.is_empty() { root.to_string() } else if let Some(v) = known.get(parent) { v.clone() } else { access(root, parent, known, adopt) };
    let first = if adopt { format!("fc({base})") } else { format!("{base}.firstChild") };
    next(first, last)
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;")
}

fn root_of(e: &Expr) -> Option<&str> {
    match e.unparen() {
        Expr::Ident(i) => Some(&i.name),
        Expr::Member { obj, .. } => root_of(obj),
        _ => None,
    }
}
