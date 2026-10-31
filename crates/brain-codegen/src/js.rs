//! BrainScript → JS printer with the reactive rewrites of SPEC §6:
//! `n` → `n.v` for sig/drv/ref/each-items, props → `($p.x ?? default)`, `props` → `$p` in views,
//! `ref()` → `sig(null)`, `emit('x', d)` → `$p["on:x"]?.(d)`, `log` → `console.log`.
//! Grouping parentheses are explicit `Paren` nodes in the AST, and every substitution is a member
//! access or a parenthesized expression, so printing without extra parentheses is precedence-safe.

use crate::Gen;
use brain_check::scope::{self, Kind};
use brain_parse::ast::*;
use std::collections::HashMap;

impl Gen<'_> {
    pub(crate) fn budget(&mut self, name: &'static str) -> String {
        self.budget_used.insert(name);
        format!("{}{name}", self.budget_prefix)
    }

    pub(crate) fn budget_site(&self, span: brain_parse::Span) -> String {
        format!("{{ file: {}, component: {}, line: {}, column: {} }}", js_str(&self.budget_file), js_str(&self.budget_component), span.line, span.col)
    }

    pub(crate) fn budget_frame(&mut self) -> String {
        let prefix = format!("{}Frame", self.budget_prefix);
        self.fresh(&prefix)
    }

    /// The `server:` function an arrow `submit` hands its values to (only for forms that post to one).
    fn submit_server(&self, f: &Func) -> Option<String> {
        let server: std::collections::HashSet<&str> = self.forms.values().map(String::as_str).collect();
        crate::submit_target(f, &server)
    }

    pub(crate) fn ident(&mut self, id: &Ident) -> String {
        let name = id.name.as_str();
        if let Some((js, _)) = self.frames.last().and_then(|f| f.props.get(name)) {
            return format!("({js})");
        }
        match self.scopes.lookup(name) {
            Some(k) if k.is_reactive_cell() => format!("{name}.v"),
            Some(Kind::Prop) => self.prop_read(name),
            Some(Kind::LoadProps) => "$p".into(),
            Some(_) => name.to_string(),
            None => match name {
                "log" => "console.log".into(),
                "sig" | "drv" | "fx" => self.rt(name),
                _ => name.to_string(),
            },
        }
    }

    /// Methods of an `api()` schema with `in`/`out` type names (`'User'`, `'[User]'`) turned into references.
    fn api_schema(&mut self, e: &Expr) -> String {
        let Expr::Object { props, .. } = e.unparen() else { return self.expr(e) };
        let mut methods = Vec::new();
        for p in props {
            match p {
                ObjProp::KeyValue { key, value: Expr::Object { props: inner, .. }, .. } => {
                    let k = self.key(key);
                    let mut fields = Vec::new();
                    for q in inner {
                        match q {
                            ObjProp::KeyValue { key: PropKey::Ident(f), value: Expr::Str { value: v, .. }, .. } if matches!(f.name.as_str(), "in" | "out") && brain_check::types::type_ref(v).is_some() => {
                                let (name, list) = brain_check::types::type_ref(v).unwrap_or_default();
                                self.type_refs.insert(name.clone());
                                fields.push(format!("{}: {}", f.name, if list { format!("[{name}]") } else { name }));
                            }
                            other => fields.push(self.obj_prop(other)),
                        }
                    }
                    methods.push(format!("{k}: {{ {} }}", fields.join(", ")));
                }
                other => methods.push(self.obj_prop(other)),
            }
        }
        format!("{{ {} }}", methods.join(", "))
    }

    fn obj_prop(&mut self, p: &ObjProp) -> String {
        match p {
            ObjProp::KeyValue { key, value, .. } => {
                let k = self.key(key);
                format!("{k}: {}", self.expr(value))
            }
            ObjProp::Shorthand(id) => {
                let val = self.ident(id);
                if val == id.name { val } else { format!("{}: {val}", id.name) }
            }
            ObjProp::Method { key, func, .. } => {
                let k = self.key(key);
                let head = if func.is_async { "async " } else { "" };
                let (params, body) = self.func_parts(func);
                format!("{head}{k}({params}) {body}")
            }
            ObjProp::Spread { arg, .. } => format!("...{}", self.expr(arg)),
        }
    }

    /// Value of `import.meta.env.KEY` / `process.env.KEY` from the build's `.brain_env` (§36).
    fn public_env(&self, obj: &Expr, key: &str) -> Option<String> {
        let root = brain_parse::dump::expr(obj);
        if root != "import.meta.env" && root != "process.env" {
            return None;
        }
        self.opts.public_env.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone())
    }

    /// The text of `t('key')` / `t('key', { a, b: x })` for the locale being built, or `None` to keep the call.
    fn static_t(&mut self, args: &[Expr]) -> Option<String> {
        let (_, texts) = self.opts.i18n.as_ref()?;
        let Expr::Str { value: key, .. } = args[0].unparen() else { return None };
        let text = texts.get(key)?.clone();
        let mut vars: Vec<(String, String)> = Vec::new();
        if let Some(a) = args.get(1) {
            let Expr::Object { props, .. } = a.unparen() else { return None };
            for p in props {
                match p {
                    ObjProp::Shorthand(i) => vars.push((i.name.clone(), self.ident(i))),
                    ObjProp::KeyValue { key: PropKey::Ident(k), value, .. } => {
                        let v = self.expr(value);
                        vars.push((k.name.clone(), v));
                    }
                    _ => return None,
                }
            }
        }
        if !text.contains('{') {
            return Some(js_str(&text));
        }
        // `Hi {name}` → `Hi ${name}` (every placeholder must be passed).
        let mut out = String::from("`");
        let mut rest = text.as_str();
        while let Some(i) = rest.find('{') {
            out.push_str(&rest[..i].replace('\\', "\\\\").replace('`', "\\`").replace("${", "\\${"));
            let end = rest[i..].find('}')? + i;
            let name = rest[i + 1..end].trim();
            let (_, v) = vars.iter().find(|(n, _)| n == name)?;
            out.push_str(&format!("${{{v}}}"));
            rest = &rest[end + 1..];
        }
        out.push_str(&rest.replace('\\', "\\\\").replace('`', "\\`").replace("${", "\\${"));
        out.push('`');
        Some(out)
    }

    pub(crate) fn prop_read(&self, name: &str) -> String {
        match self.prop_defaults.get(name) {
            Some(d) => format!("($p.{name} ?? {d})"),
            None => format!("$p.{name}"),
        }
    }

    pub(crate) fn key(&mut self, k: &PropKey) -> String {
        match k {
            PropKey::Ident(i) => i.name.clone(),
            PropKey::Str { raw, .. } | PropKey::Num { raw, .. } => raw.clone(),
            PropKey::Computed(e) => format!("[{}]", self.expr(e)),
        }
    }

    pub(crate) fn pat(&mut self, p: &Pat) -> String {
        match p {
            Pat::Ident(i) => i.name.clone(),
            Pat::Object { props, rest, .. } => {
                let mut v = Vec::new();
                for pp in props {
                    if pp.shorthand {
                        v.push(self.pat(&pp.value));
                    } else {
                        let k = self.key(&pp.key);
                        let val = self.pat(&pp.value);
                        v.push(format!("{k}: {val}"));
                    }
                }
                if let Some(r) = rest {
                    v.push(format!("...{}", self.pat(r)));
                }
                format!("{{ {} }}", v.join(", "))
            }
            Pat::Array { elems, rest, .. } => {
                let mut v: Vec<String> = elems.iter().map(|e| e.as_ref().map(|p| self.pat(p)).unwrap_or_default()).collect();
                if let Some(r) = rest {
                    v.push(format!("...{}", self.pat(r)));
                }
                format!("[{}]", v.join(", "))
            }
            Pat::Default { pat, default, .. } => {
                let p = self.pat(pat);
                format!("{p} = {}", self.expr(default))
            }
            Pat::Rest { arg, .. } => format!("...{}", self.pat(arg)),
        }
    }

    fn args(&mut self, args: &[Expr]) -> String {
        args.iter().map(|a| self.expr(a)).collect::<Vec<_>>().join(", ")
    }

    pub(crate) fn expr(&mut self, e: &Expr) -> String {
        match e {
            Expr::Ident(id) => self.ident(id),
            Expr::This(_) => "this".into(),
            Expr::Num { raw, .. } | Expr::Str { raw, .. } | Expr::Regex { raw, .. } => raw.clone(),
            Expr::Bool { value, .. } => value.to_string(),
            Expr::Null(_) => "null".into(),
            Expr::Template { tag, quasis, exprs, .. } => {
                let mut s = tag.as_ref().map(|t| self.expr(t)).unwrap_or_default();
                s.push('`');
                for (i, q) in quasis.iter().enumerate() {
                    s.push_str(q);
                    if let Some(x) = exprs.get(i) {
                        s.push_str("${");
                        s.push_str(&self.expr(x));
                        s.push('}');
                    }
                }
                s.push('`');
                s
            }
            Expr::Array { elems, .. } => {
                let v: Vec<String> = elems.iter().map(|x| x.as_ref().map(|x| self.expr(x)).unwrap_or_default()).collect();
                let trailing_hole = matches!(elems.last(), Some(None));
                format!("[{}{}]", v.join(", "), if trailing_hole { "," } else { "" })
            }
            Expr::Object { props, .. } => {
                if props.is_empty() {
                    return "{}".into();
                }
                let v: Vec<String> = props.iter().map(|p| self.obj_prop(p)).collect();
                format!("{{ {} }}", v.join(", "))
            }
            Expr::Func(f) => self.func_expr(f),
            Expr::Unary { op, arg, .. } => {
                let a = self.expr(arg);
                let alpha = op.chars().all(|c| c.is_alphabetic());
                let clash = !alpha && (a.starts_with(op) || (*op == "-" && a.starts_with("--")) || (*op == "+" && a.starts_with("++")));
                format!("{op}{}{a}", if alpha || clash { " " } else { "" })
            }
            Expr::Update { op, arg, .. } if self.bindable_prop(arg).is_some() => {
                let name = self.bindable_prop(arg).unwrap_or_default();
                let read = self.expr(arg);
                format!("$p[{}]?.({read} {} 1)", js_str(&format!("bind:{name}")), &op[..1])
            }
            Expr::Update { op, prefix, arg, .. } => {
                let a = self.expr(arg);
                if *prefix { format!("{op}{a}") } else { format!("{a}{op}") }
            }
            Expr::Binary { op, left, right, .. } => {
                let l = self.expr(left);
                let r = self.expr(right);
                format!("{l} {op} {r}")
            }
            Expr::Assign { op, target, value, .. } if self.bindable_prop(target).is_some() => {
                // Writing a `~` prop hands the new value to the parent's binding.
                let name = self.bindable_prop(target).unwrap_or_default();
                let v = self.expr(value);
                let v = if *op == "=" { v } else { format!("{} {} ({v})", self.expr(target), op.trim_end_matches('=')) };
                format!("$p[{}]?.({v})", js_str(&format!("bind:{name}")))
            }
            Expr::Assign { op, target, value, .. } => {
                let t = self.expr(target);
                let v = self.expr(value);
                format!("{t} {op} {v}")
            }
            Expr::Cond { test, cons, alt, .. } => {
                let t = self.expr(test);
                let c = self.expr(cons);
                let a = self.expr(alt);
                format!("{t} ? {c} : {a}")
            }
            Expr::Call { callee, args, optional, .. } => self.call(callee, args, *optional),
            Expr::New { callee, args, .. } => {
                let c = self.expr(callee);
                format!("new {c}({})", self.args(args))
            }
            Expr::Member { prop: MemberProp::Name(k), obj, .. } if k.name.starts_with("PUBLIC_") && self.public_env(obj, &k.name).is_some() => {
                js_str(&self.public_env(obj, &k.name).unwrap_or_default())
            }
            // `wide.v` written by the author (B0130) is the same single read as `wide`.
            Expr::Member { obj, prop: MemberProp::Name(n), .. }
                if n.name == "v" && matches!(obj.unparen(), Expr::Ident(i) if matches!(self.scopes.lookup(&i.name), Some(Kind::Sig | Kind::Drv | Kind::Ref))) =>
            {
                self.expr(obj)
            }
            // `posts.refetch()`, `posts.more()`, `posts.done`, `feed.connected` live on the node, not on its value.
            Expr::Member { obj, prop: MemberProp::Name(n), optional, .. }
                if matches!(n.name.as_str(), "refetch" | "more" | "done" | "connected")
                    && matches!(obj.unparen(), Expr::Ident(i) if self.node_vars.contains(&i.name) && self.scopes.lookup(&i.name) == Some(Kind::Drv)) =>
            {
                let Expr::Ident(i) = obj.unparen() else { unreachable!() };
                format!("{}{}{}", i.name, if *optional { "?." } else { "." }, n.name)
            }
            Expr::Member { obj, prop, optional, .. } => {
                let o = self.expr(obj);
                // `1.toString()` would be a syntax error.
                let o = if matches!(obj.unparen(), Expr::Num { .. }) && !o.contains(['.', 'e', 'x']) { format!("({o})") } else { o };
                match prop {
                    MemberProp::Name(n) => format!("{o}{}{}", if *optional { "?." } else { "." }, n.name),
                    MemberProp::Computed(p) => format!("{o}{}[{}]", if *optional { "?." } else { "" }, self.expr(p)),
                }
            }
            Expr::Seq { exprs, .. } => self.args(exprs),
            Expr::Spread { arg, .. } => format!("...{}", self.expr(arg)),
            Expr::Await { arg, .. } => {
                // `suspend(frame)` runs before the awaited expression, so its native work is not charged.
                let value = self.expr(arg);
                if let Some(frame) = self.budget_frames.last().cloned() {
                    let suspend = self.budget("budgetSuspend");
                    let resume = self.budget("budgetResume");
                    format!("{resume}({frame}, await {suspend}({frame})({value}))")
                } else { format!("await {value}") }
            }
            Expr::Paren { expr, .. } => format!("({})", self.expr(expr)),
        }
    }

    /// Name of a `~` prop when `e` is a plain read of it.
    pub(crate) fn bindable_prop(&self, e: &Expr) -> Option<String> {
        match e.unparen() {
            Expr::Ident(id) if self.bindable.contains(&id.name) && self.scopes.lookup(&id.name) == Some(Kind::Prop) => Some(id.name.clone()),
            _ => None,
        }
    }

    /// Closed, side-effect-free schema subset. All other syntax keeps runtime form/rule.
    fn static_form_schema(&self, value: &Expr) -> Option<(String, bool)> {
        let Expr::Object { props, .. } = value.unparen() else { return None; };
        if props.len()>4096 { return None; }
        let mut seen = std::collections::HashSet::new();
        let mut fields = Vec::new();
        let mut scalar = false;
        for prop in props {
            let ObjProp::KeyValue { key, value, .. } = prop else { return None; };
            let key = match key { PropKey::Ident(id)=>id.name.as_str(), PropKey::Str {value,..}=>value.as_str(), _=>return None };
            if key.len()>256 || ["__proto__","prototype","constructor"].contains(&key) || !seen.insert(key) { return None; }
            let Expr::Str { value, .. } = value.unparen() else { return None; };
            let (kind,required) = match value.as_str() {
                "str"=>("str",false), "str!"=>("str",true), "email"=>("email",false), "email!"=>("email",true), "bool"=>("bool",false), "bool!"=>("bool",true), "num"=>("num",false), "num!"=>("num",true), _=>return None,
            };
            scalar |= matches!(kind, "bool" | "num");
            fields.push(format!("{}:[{},{}]",js_str(key),js_str(kind),required));
        }
        Some((format!("{{{}}}",fields.join(",")), scalar))
    }

    // The literal options cannot escape through a dynamic callback receiver. Unknown
    // options, methods, getters, spreads and cross-field/async validation retain the full engine.
    fn synchronous_form_options(&self, args: &[Expr]) -> bool {
        if args.len()>2 { return false; }
        let Some(options)=args.get(1) else { return true; };
        let Expr::Object {props,..}=options.unparen() else { return false; };
        let mut seen=std::collections::HashSet::new();
        props.iter().all(|property| {
            let ObjProp::KeyValue {key,value,..}=property else { return false; };
            let name=match key {PropKey::Ident(id)=>id.name.as_str(),PropKey::Str {value,..}=>value.as_str(),_=>return false};
            if !seen.insert(name) {return false;}
            match name {
                "init"=>true,
                "submit"=>match value.unparen() {
                    Expr::Func(func)=>func.is_arrow,
                    Expr::Ident(id)=>self.forms.values().any(|server|server==&id.name) && !self.scopes.frames.iter().skip(1).any(|scope|scope.contains_key(&id.name)),
                    _=>false,
                },
                _=>false,
            }
        })
    }
    fn static_form_call(&mut self, args: &[Expr]) -> Option<String> {
        let (schema, scalar) = self.static_form_schema(args.first()?)?;
        let sync=self.synchronous_form_options(args);
        match (scalar,sync) {
            (true,true)=>self.scalar_form_sync=true,
            (true,false)=>self.scalar_form=true,
            (false,true)=>self.static_form_sync=true,
            (false,false)=>self.static_form=true,
        }
        let mut values = vec![schema];
        if let Some(options) = args.get(1) {
            let target = match options.unparen() {
                Expr::Object { props, .. } => props.iter().find_map(|p| match p {
                    ObjProp::KeyValue { key: PropKey::Ident(k), value: Expr::Func(f), .. } if k.name == "submit" => self.submit_server(f),
                    _ => None,
                }), _ => None,
            };
            let value = self.expr(options);
            values.push(match target { Some(target) => format!("((o) => (o.submit.id = {target}.id, o))({value})"), None => value });
        }
        values.extend(args.iter().skip(2).map(|arg| self.expr(arg)));
        Some(format!("{}{}({})", self.budget_prefix, match (scalar,sync) {(true,true)=>"SyncScalarForm",(true,false)=>"ScalarForm",(false,true)=>"SyncForm",(false,false)=>"Form"}, values.join(", ")))
    }

    /// Only absent options or a closed empty literal prove that no transport is configured.
    fn plain_store_call(&mut self, args: &[Expr]) -> Option<String> {
        if !(1..=2).contains(&args.len()) || args.iter().any(|arg|matches!(arg.unparen(),Expr::Spread{..})) { return None; }
        if let Some(options)=args.get(1) {
            if !matches!(options.unparen(),Expr::Object{props,..} if props.is_empty()) { return None; }
        }
        self.plain_store=true;
        let init=self.expr(&args[0]);
        let id=self.store_id.as_deref().map(|id|format!(", {}",js_str(id))).unwrap_or_default();
        Some(format!("{}PlainStore({init}{id})",self.budget_prefix))
    }

    fn call(&mut self, callee: &Expr, args: &[Expr], optional: bool) -> String {
        if let Expr::Ident(id) = callee.unparen()
            && self.scopes.lookup(&id.name).is_none()
        {
            if id.name == "store" && !optional {
                if let Some(call)=self.plain_store_call(args) { return call; }
            }
            if id.name == "form" {
                if let Some(call) = self.static_form_call(args) { return call; }
            }
            match id.name.as_str() {
                // `provide(K, theme)` with a signal passes the signal itself so consumers stay reactive.
                "provide" if args.len() == 2 => {
                    if let Expr::Ident(v) = args[1].unparen()
                        && self.scopes.lookup(&v.name).is_some_and(|k| k.is_reactive_cell())
                    {
                        return format!("provide({}, {})", self.expr(&args[0]), v.name);
                    }
                }
                // `store(init, opts?)` in channels/ gets its file id as sync key (brain-ch.js).
                "store" if self.store_id.is_some() && args.len() < 3 => {
                    let mut a: Vec<String> = args.iter().map(|x| self.expr(x)).collect();
                    if a.is_empty() {
                        a.push("undefined".into());
                    }
                    if a.len() < 2 {
                        a.push("undefined".into());
                    }
                    a.push(js_str(self.store_id.as_deref().unwrap_or_default()));
                    return format!("store({})", a.join(", "));
                }
                "ref" => return format!("{}(null)", self.rt("sig")),
                // §38: `submit: async (v) => { … await login(v) … }` posts to `login` without JS; form() reads the
                // id of that server function from `submit.id` to restore the posted values and errors.
                "form" if args.len() >= 2
                    && matches!(args[1].unparen(), Expr::Object { props, .. } if props.iter().any(|p| matches!(p, ObjProp::KeyValue { key: PropKey::Ident(k), value: Expr::Func(f), .. } if k.name == "submit" && self.submit_server(f).is_some()))) =>
                {
                    let Expr::Object { props, .. } = args[1].unparen() else { unreachable!() };
                    let target = props
                        .iter()
                        .find_map(|p| match p {
                            ObjProp::KeyValue { key: PropKey::Ident(k), value: Expr::Func(f), .. } if k.name == "submit" => self.submit_server(f),
                            _ => None,
                        })
                        .unwrap_or_default();
                    let rest: String = args[2..].iter().map(|x| format!(", {}", self.expr(x))).collect();
                    return format!("form({}, ((o) => (o.submit.id = {target}.id, o))({}){rest})", self.expr(&args[0]), self.expr(&args[1]));
                }
                // Resource methods and its current Promise live on the node, not the awaited value.
                "liveResource" if matches!(args.first().map(Expr::unparen), Some(Expr::Ident(r)) if self.node_vars.contains(&r.name) && self.scopes.lookup(&r.name) == Some(Kind::Drv)) => {
                    let Some(Expr::Ident(r)) = args.first().map(Expr::unparen) else { unreachable!() };
                    let rest: String = args[1..].iter().map(|x| format!(", {}", self.expr(x))).collect();
                    return format!("liveResource({}{rest})", r.name);
                }
                // `visible(box)` takes the ref itself: the runtime reads it inside an effect, after `bind:ref` fills it.
                "visible" if matches!(args.first().map(Expr::unparen), Some(Expr::Ident(r)) if self.scopes.lookup(&r.name) == Some(Kind::Ref)) => {
                    let Some(Expr::Ident(r)) = args.first().map(Expr::unparen) else { unreachable!() };
                    let rest: String = args[1..].iter().map(|x| format!(", {}", self.expr(x))).collect();
                    return format!("visible({}{rest})", r.name);
                }
                // SPEC writes `debounce(q, '300ms')`; the runtime takes a getter so the read stays tracked.
                "debounce" if !args.is_empty() && !matches!(args[0].unparen(), Expr::Func(_)) => {
                    let rest: Vec<String> = args[1..].iter().map(|x| self.expr(x)).collect();
                    let sep = if rest.is_empty() { "" } else { ", " };
                    return format!("debounce(() => {}{sep}{})", self.expr(&args[0]), rest.join(", "));
                }
                // §22: guards, middleware, `load` and `server:` return or throw these plain objects (RUNTIME runChain,
                // server.mjs and the client router read them); no import.
                "redirect" if !args.is_empty() => {
                    let to = self.expr(&args[0]);
                    let status = args.get(1).map(|s| self.expr(s)).unwrap_or_else(|| "303".into());
                    return format!("({{ redirect: {to}, status: {status} }})");
                }
                // `throw notFound()` (or return it) from `load`: the server answers 404 with views/404.brain (F11).
                // It takes no arguments: any given (`notFound('why')`) are ignored, never left as an undefined call.
                "notFound" => return "({ notFound: true, status: 404 })".into(),
                // `throw gone()` (§33): the record was removed for good → 410. `notFound: true` keeps older readers on
                // the not-found path; server.mjs and the dev server read `gone`. A local `gone` is the author's own.
                "gone" if self.scopes.lookup("gone").is_none() => return "({ notFound: true, gone: true, status: 410 })".into(),
                "deny" => {
                    let why = args.first().map(|r| self.expr(r)).unwrap_or_else(|| "\"forbidden\"".into());
                    return format!("({{ deny: {why} }})");
                }
                "locale" => self.uses_i18n = true,
                // `t('key', vars?)` (§37): resolved at build when the text is known and plain; otherwise the key
                // is recorded so the per-view locale chunk carries it.
                "t" if !args.is_empty() => {
                    if let Some(s) = self.static_t(args) {
                        return s;
                    }
                    self.uses_i18n = true;
                    match args[0].unparen() {
                        Expr::Str { value, .. } => self.i18n_keys.insert(value.clone()),
                        _ => self.i18n_keys.insert("*".to_string()),
                    };
                }
                // `api(path, { get: { out: '[User]' } })`: type names become the objects of types.jsbrain (§15).
                "api" if args.len() == 2 && matches!(args[1].unparen(), Expr::Object { .. }) => {
                    let path = self.expr(&args[0]);
                    let schema = self.api_schema(&args[1]);
                    return format!("api({path}, {schema})");
                }
                "emit" => {
                    if let Some(Expr::Str { value, .. }) = args.first().map(Expr::unparen) {
                        let rest = self.args(&args[1..]);
                        return format!("$p[{}]?.({rest})", js_str(&format!("on:{value}")));
                    }
                }
                _ => {}
            }
        }
        let c = self.expr(callee);
        format!("{c}{}({})", if optional { "?." } else { "" }, self.args(args))
    }

    /// `(params)` and `{ body }`/`expr`, with a scope for the parameters.
    fn func_parts(&mut self, f: &Func) -> (String, String) {
        self.scopes.push();
        for p in &f.params {
            self.scopes.declare_pat(p, Kind::Local);
        }
        let params: Vec<String> = f.params.iter().map(|p| self.pat(p)).collect();
        let frame = self.budget_frame();
        self.budget_frames.push(frame.clone());
        let body = match &f.body {
            FnBody::Expr(e) => format!("{{ return {}; }}", self.expr(e)),
            FnBody::Block(b) => self.block(b),
        };
        self.budget_frames.pop();
        let enter = self.budget("budgetEnter");
        let step = self.budget("budgetStep");
        let leave = self.budget("budgetLeave");
        let owner = if self.budget_capture_owner { format!("{}Owner", self.budget_prefix) } else { "null".into() };
        let site = self.budget_site(f.span);
        let body = format!("{{ const {frame} = {enter}({site}, {owner}); try {{ {step}({frame}); {body} }} finally {{ {leave}({frame}); }} }}");
        self.scopes.pop();
        (params.join(", "), body)
    }

    pub(crate) fn func_expr(&mut self, f: &Func) -> String {
        let asy = if f.is_async { "async " } else { "" };
        let (params, body) = self.func_parts(f);
        if f.is_arrow {
            format!("{asy}({params}) => {body}")
        } else {
            let name = f.name.as_ref().map(|n| n.name.as_str()).unwrap_or("");
            format!("{asy}function {name}({params}) {body}")
        }
    }

    /// `{ … }` with its own scope; indented one level deeper than `self.indent`.
    pub(crate) fn block(&mut self, body: &[Stmt]) -> String {
        self.scopes.push();
        let mut m = HashMap::new();
        for s in body {
            scope::declare_stmt(s, &mut m);
        }
        for (k, v) in m {
            self.scopes.declare(&k, if v.is_reactive_cell() { Kind::Let } else { v });
        }
        self.indent += 1;
        let mut out = String::from("{\n");
        for s in body {
            // Source-map marker per nested statement too (handlers, async bodies), §18.
            out.push_str(&crate::srcmap::mark(s.span().line));
            out.push_str(&self.stmt(s));
        }
        self.indent -= 1;
        out.push_str(&self.pad());
        out.push('}');
        self.scopes.pop();
        out
    }

    pub(crate) fn pad(&self) -> String {
        "  ".repeat(self.indent)
    }

    /// One statement, indented and newline-terminated.
    pub(crate) fn stmt(&mut self, s: &Stmt) -> String {
        let p = self.pad();
        match s {
            Stmt::Var(v) => {
                let top = self.module && self.indent == 0;
                format!("{p}{};\n", self.var_decl(v, top))
            }
            // Folded effect (§13): reads nothing reactive and returns no cleanup → its body runs inline.
            Stmt::Expr { expr: Expr::Call { args, span, .. }, .. }
                if self.folded_fx.contains(&span.start)
                    && let Some(Expr::Func(f)) = args.first().map(Expr::unparen)
                    && let FnBody::Block(b) = &f.body =>
            {
                format!("{p}{}\n", self.block(b))
            }
            Stmt::Expr { expr, .. } => {
                let e = self.expr(expr);
                // Parenthesize ambiguous statement starts, including regex after an emitter-added block.
                let e = if e.starts_with(['{', '/']) || e.starts_with("function") || e.starts_with("async function") { format!("({e})") } else { e };
                format!("{p}{e};\n")
            }
            Stmt::If { test, cons, alt, .. } => {
                let t = self.expr(test);
                let c = self.inline_stmt(cons);
                let mut out = format!("{p}if ({t}) {c}");
                if let Some(a) = alt {
                    out.push_str(" else ");
                    out.push_str(&self.inline_stmt(a));
                }
                out.push('\n');
                out
            }
            Stmt::ForOf { kind, pat, iter, body, span } => {
                let it = self.expr(iter);
                let own = self.budget_frames.is_empty();
                let frame = if own { self.budget_frame() } else { self.budget_frames.last().unwrap().clone() };
                if own { self.budget_frames.push(frame.clone()); }
                self.scopes.push();
                self.scopes.declare_pat(pat, Kind::Local);
                let pt = self.pat(pat);
                let b = self.inline_stmt(body);
                self.scopes.pop();
                if own { self.budget_frames.pop(); }
                let k = if *kind == VarKind::Const { "const" } else { "let" };
                let step = self.budget("budgetStep");
                let site = self.budget_site(*span);
                let loop_js = format!("for ({k} {pt} of {it}) {{ {step}({frame}, {site}); {b} }}");
                if own {
                    let enter = self.budget("budgetEnter");
                    let leave = self.budget("budgetLeave");
                    let owner = if self.budget_capture_owner { format!("{}Owner", self.budget_prefix) } else { "null".into() };
                    format!("{p}{{ const {frame} = {enter}({site}, {owner}); try {{ {loop_js} }} finally {{ {leave}({frame}); }} }}\n")
                } else { format!("{p}{loop_js}\n") }
            }
            Stmt::Return { arg, .. } => match arg {
                Some(a) => format!("{p}return {};\n", self.expr(a)),
                None => format!("{p}return;\n"),
            },
            Stmt::Block { body, .. } => format!("{p}{}\n", self.block(body)),
            Stmt::Func(f) => format!("{p}{}\n", self.func_expr(f)),
            Stmt::Try { block, param, handler, finalizer, .. } => {
                let mut out = format!("{p}try {}", self.block(block));
                if let Some(h) = handler {
                    self.scopes.push();
                    let par = match param {
                        Some(pt) => {
                            self.scopes.declare_pat(pt, Kind::Local);
                            format!(" ({})", self.pat(pt))
                        }
                        None => String::new(),
                    };
                    out.push_str(&format!(" catch{par} {}", self.block(h)));
                    self.scopes.pop();
                }
                if let Some(f) = finalizer {
                    out.push_str(&format!(" finally {}", self.block(f)));
                }
                out.push('\n');
                out
            }
            Stmt::Throw { arg, .. } => format!("{p}throw {};\n", self.expr(arg)),
            Stmt::Break { .. } => format!("{p}break;\n"),
            Stmt::Continue { .. } => format!("{p}continue;\n"),
            // Imports are hoisted to the module and `export` is only meaningful for `load`.
            Stmt::Import(_) => String::new(),
            Stmt::Export { decl, .. } => {
                if self.module {
                    format!("{p}export {}", self.stmt(decl).trim_start())
                } else {
                    self.stmt(decl)
                }
            }
            Stmt::ExportDefault { expr, .. } => format!("{p}export default {};\n", self.expr(expr)),
            // Server functions are printed into `server_js`; the client keeps a module-level stub.
            Stmt::Empty { .. } | Stmt::Server { .. } => String::new(),
        }
    }

    /// Statement used as the body of `if`/`for` (no leading indent, no trailing newline).
    fn inline_stmt(&mut self, s: &Stmt) -> String {
        match s {
            Stmt::Block { body, .. } => self.block(body),
            other => {
                let saved = self.indent;
                self.indent += 1;
                let inner = self.stmt(other);
                self.indent = saved;
                format!("{{\n{inner}{}}}", self.pad())
            }
        }
    }

    /// `let/const …`. At the top level, `sig`/`drv`/`ref` declarations become `const` and in dev
    /// mode `sig` gets its variable name (SPEC §6.2 HMR).
    pub(crate) fn var_decl(&mut self, v: &VarDecl, top: bool) -> String {
        let mut all_cells = true;
        let mut parts = Vec::new();
        for d in &v.decls {
            let folded = top && matches!(&d.pat, Pat::Ident(id) if self.folded.contains(&id.name));
            let kind = if top && !folded { scope::init_kind(d.init.as_ref()) } else { None };
            if kind.is_none() && !folded {
                all_cells = false;
            }
            let pat = self.pat(&d.pat);
            match (&d.init, kind, &d.pat) {
                (Some(Expr::Call { callee, args, .. }), Some(Kind::Sig), Pat::Ident(_)) if matches!(callee.unparen(), Expr::Ident(k) if k.name == "urlState") => {
                    parts.push(format!("{pat} = urlState({})", self.args(args)));
                }
                (Some(Expr::Call { args, .. }), Some(Kind::Sig | Kind::Ref), Pat::Ident(id)) => {
                    let sig = self.rt("sig");
                    let mut a = if kind == Some(Kind::Ref) { "null".to_string() } else { self.args(args) };
                    if a.is_empty() {
                        a = "undefined".into();
                    }
                    if self.opts.dev {
                        parts.push(format!("{pat} = {sig}({a}, {})", js_str(&id.name)));
                    } else {
                        parts.push(format!("{pat} = {sig}({a})"));
                    }
                }
                (Some(Expr::Call { callee, args, .. }), Some(Kind::Drv), Pat::Ident(_)) if matches!(callee.unparen(), Expr::Ident(u) if u.name == "use") => {
                    // Context read: `use` runs once in setup; the drv unwraps a provided signal reactively.
                    let drv = self.rt("drv");
                    parts.push(format!("{pat} = {drv}(((u) => () => $val(u))(use({})))", self.args(args)));
                }
                // Folded (§13): `sig(v)` → `v`, `drv(() => e)` → `e` computed once.
                (Some(Expr::Call { callee, args, .. }), None, Pat::Ident(_)) if folded => {
                    let v = match callee.unparen() {
                        Expr::Ident(k) if k.name == "drv" => match args.first().map(Expr::unparen) {
                            Some(Expr::Func(f)) => match &f.body {
                                FnBody::Expr(e) => self.expr(e),
                                FnBody::Block(_) => format!("({})()", self.expr(&args[0])),
                            },
                            _ => format!("({})()", self.args(args)),
                        },
                        _ if args.is_empty() => "undefined".into(),
                        _ => self.expr(&args[0]),
                    };
                    parts.push(format!("{pat} = {v}"));
                }
                (Some(init), _, _) => parts.push(format!("{pat} = {}", self.expr(init))),
                (None, _, _) => parts.push(pat),
            }
        }
        let k = match v.kind {
            VarKind::Const => "const",
            _ if top && all_cells => "const",
            _ => "let",
        };
        format!("{k} {}", parts.join(", "))
    }
}

/// JS string literal with double quotes.
pub fn js_str(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    o.push('"');
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            '\u{2028}' => o.push_str("\\u2028"),
            '\u{2029}' => o.push_str("\\u2029"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\x{:02x}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    // `</script>` inside an inline module would close the tag.
    o.replace("</script", "<\\/script")
}
