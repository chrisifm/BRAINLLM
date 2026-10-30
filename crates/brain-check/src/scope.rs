//! Name classification shared by the checker and the codegen: which names are signals (read as
//! `.v`), derived values, props, each-items (signals too, per SPEC §6), plain bindings or globals.

use brain_parse::ast::*;
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// `let n = sig(x)` — read/write through `.v`.
    Sig,
    /// `let d = drv(() => …)` — read through `.v`, never written.
    Drv,
    /// `let el = ref()` — compiled as `sig(null)`.
    Ref,
    /// `let f = form(schema, opts)` — `f.v.field` is reactive and writable (bind target).
    Form,
    /// A prop from `<meta props>` — read as `$p.name` (with its default), never written.
    Prop,
    /// `props` in `views/`: the object returned by `load()`.
    LoadProps,
    /// Item or index of `{#each}` — a signal handed in by `list()`.
    EachItem,
    Const,
    Let,
    Func,
    Import,
    /// Function or catch parameter, `for…of` variable, `{:then v}` binding.
    Local,
}

impl Kind {
    /// Names whose reads compile to `name.v`.
    pub fn is_reactive_cell(self) -> bool {
        matches!(self, Kind::Sig | Kind::Drv | Kind::Ref | Kind::EachItem)
    }
}

/// Framework primitives (SPEC §2) — the only framework globals. None needs an import.
pub const PRIMITIVES: &[&str] = &["sig", "drv", "fx", "emit", "ref", "log", "form", "resource", "api", "provide", "use", "key", "store", "bus", "sanitize", "t", "locale", "fmt", "alternates", "suggest", "dismiss", "navigate", "route", "mounted", "watch", "tick", "every", "media", "visible", "debounce", "stream", "liveResource", "online", "http", "ApiError", "ai", "interop", "dateParse", "dateAdd", "dateDiff", "dateRange", "dateZone", "hashText", "shortHash", "database", "storageInfo", "persistStorage", "urlState", "storage", "cookie", "sqlite", "urlDialog", "sharedWorker", "threadCapabilities", "loadWorklet", "messageChannel", "offscreen", "sharedMemory", "transfer"];

/// Primitives that live outside the runtime core, and the module (next to `brain.js`) that exports them.
/// Codegen imports them automatically when used (SPEC: "all primitives need no import").
pub const EXT_PRIMITIVES: &[(&str, &str)] = &[
    ("provide", "brain-ch.js"),
    ("use", "brain-ch.js"),
    ("key", "brain-ch.js"),
    ("store", "brain-ch.js"),
    ("bus", "brain-ch.js"),
    // Emitted by the codegen, never written by app authors.
    ("scope", "brain-ch.js"),
    ("expose", "brain-ch.js"),
    // `$val` is imported as `val as $val`: unwraps a provided signal inside the consumer's `drv`.
    ("$val", "brain-ch.js"),
    ("dateParse", "brain-time.js"),
    ("dateAdd", "brain-time.js"),
    ("dateDiff", "brain-time.js"),
    ("dateRange", "brain-time.js"),
    ("dateZone", "brain-time.js"),
    ("hashText", "brain-hash.js"),
    ("shortHash", "brain-hash.js"),
    ("transfer", "brain-worker.js"),
    ("sharedWorker", "brain-worker.js"),
    ("threadCapabilities", "brain-threads.js"),
    ("loadWorklet", "brain-threads.js"),
    ("messageChannel", "brain-threads.js"),
    ("offscreen", "brain-threads.js"),
    ("sharedMemory", "brain-threads.js"),
    ("sqlite", "brain-sqlite.js"),
    ("storage", "brain-preferences.js"),
    ("cookie", "brain-preferences.js"),
    ("urlState", "brain-url.js"),
    ("urlDialog", "brain-url.js"),
    ("$urlBind", "brain-url.js"),
    ("database", "brain-storage.js"),
    ("storageInfo", "brain-storage.js"),
    ("persistStorage", "brain-storage.js"),
    ("interop", "brain-interop.js"),
    ("form", "brain-form.js"),
    ("resource", "brain-data.js"),
    ("api", "brain-data.js"),
    ("sanitize", "brain-data.js"),
    ("html", "brain-data.js"),
    // Languages (§37): `t('key', vars?)` and `locale()`.
    ("t", "brain-i18n.js"),
    ("locale", "brain-i18n.js"),
    // `fmt.date/number/money/relative` over Intl for the current locale; the kit's LangSwitch uses the rest.
    ("fmt", "brain-i18n.js"),
    ("alternates", "brain-i18n.js"),
    ("suggest", "brain-i18n.js"),
    ("dismiss", "brain-i18n.js"),
    // Client navigation (§40): `navigate('/x', { replace })` keeps the base path. `route()` (path, params, query) lives in
    // brain-env.js so a page without islands loads only the router (RUNTIME #239).
    ("navigate", "brain-router.js"),
    ("route", "brain-env.js"),
    // Lifecycle and environment helpers (§41.1, §41.6). `dyn` (`<component is>`) and `portal` (`<Portal>`) are
    // emitted by the codegen.
    ("mounted", "brain-env.js"),
    ("watch", "brain-env.js"),
    ("tick", "brain-env.js"),
    ("every", "brain-env.js"),
    ("media", "brain-env.js"),
    ("visible", "brain-env.js"),
    ("debounce", "brain-env.js"),
    ("dyn", "brain-env.js"),
    ("portal", "brain-env.js"),
    // HTTP client (§40): `online()` reads a signal; `http()` is the app policy (`<meta http>`, `api/_http.jsbrain`).
    ("online", "brain-data.js"),
    ("http", "brain-data.js"),
    ("ApiError", "brain-data.js"),
    ("stream", "brain-data.js"),
    ("liveResource", "brain-live.js"),
    // Hydration by adoption (#143): node walks in adopt mode, emitted by the codegen.
    ("fc", "brain-hydrate.js"),
    ("nx", "brain-hydrate.js"),
    // `<Chart lazy />` (§22), emitted by the codegen.
    ("lazy", "brain-lazy.js"),
    // `transition:fade|slide|scale` (§41.6), emitted by the codegen.
    ("transition", "brain-anim.js"),
    // Emitted for `<meta tools>` (WebMCP, §32).
    ("registerTools", "brain-mcp.js"),
    // Chrome's built-in AI (Gemini Nano, INTERNALS §19): feature-detected, 'unavailable' elsewhere.
    ("ai", "brain-ai.js"),
];

/// Server-only collection readers (§33, `brain:content`): usable without import in `load()`, `paths()` and `server:`
/// functions; the codegen imports them into the server module.
pub const CONTENT: &[&str] = &["entries", "entry", "tags", "tagged", "pages", "page", "collection"];

/// `brain/ui` kit (INTERNALS §14): usable in markup without import.
pub const UI_KIT: &[&str] = &["Button", "Input", "Select", "Checkbox", "Dialog", "Table", "Tabs", "Toast", "Menu", "Field", "LangSwitch", "ErrorState", "Stack", "Row", "Grid", "Center", "Container", "Sidebar", "Spacer"];

/// Standard JS and Web globals that BrainScript may use without declaring.
pub const KNOWN_GLOBALS: &[&str] = &[
    "undefined", "NaN", "Infinity", "globalThis", "console", "Math", "JSON", "Date", "Promise", "Array", "Object",
    "String", "Number", "Boolean", "Symbol", "BigInt", "Map", "Set", "WeakMap", "WeakSet", "WeakRef", "RegExp",
    "Error", "TypeError", "RangeError", "SyntaxError", "AggregateError", "Intl", "Reflect", "Proxy", "ArrayBuffer",
    "DataView", "Uint8Array", "Int8Array", "Uint16Array", "Int16Array", "Uint32Array", "Int32Array", "Float32Array",
    "Float64Array", "BigInt64Array", "BigUint64Array", "parseInt", "parseFloat", "isNaN", "isFinite",
    "encodeURIComponent", "decodeURIComponent", "encodeURI", "decodeURI", "atob", "btoa", "fetch", "setTimeout",
    "clearTimeout", "setInterval", "clearInterval", "requestAnimationFrame", "cancelAnimationFrame",
    "queueMicrotask", "structuredClone", "alert", "confirm", "prompt", "navigator", "location", "history",
    "localStorage", "sessionStorage", "crypto", "performance", "document", "window", "URL", "URLSearchParams",
    "FormData", "Headers", "Request", "Response", "AbortController", "AbortSignal", "Blob", "File", "FileReader",
    "TextEncoder", "TextDecoder", "Event", "CustomEvent", "KeyboardEvent", "MouseEvent", "EventTarget",
    "IntersectionObserver", "ResizeObserver", "MutationObserver", "WebSocket", "EventSource", "Worker",
    "BroadcastChannel", "Image", "Audio", "Notification", "matchMedia", "getComputedStyle", "scrollTo",
    "innerWidth", "innerHeight", "devicePixelRatio", "indexedDB", "caches", "reportError", "scheduler",
    "ServiceWorkerRegistration", "PushManager", "crossOriginIsolated", "isSecureContext", "SharedWorker", "MessageChannel", "MessagePort", "SharedArrayBuffer", "Atomics", "OffscreenCanvas", "WebAssembly", "AudioContext", "AudioWorkletNode", "WorkletAnimation", "ImageBitmap", "createImageBitmap", "CSS", "requestIdleCallback", "cancelIdleCallback", "IDBKeyRange", "self",
];

/// Top-level names of a component and their kind.
pub fn top_bindings(c: &Component) -> HashMap<String, Kind> {
    let mut m = HashMap::new();
    for p in &c.props {
        m.insert(p.name.clone(), Kind::Prop);
    }
    if c.level == Level::View {
        m.insert("props".to_string(), Kind::LoadProps);
    }
    for s in &c.script {
        declare_stmt(s, &mut m);
    }
    m
}

/// The primitive a declaration is initialized with (`sig`, `drv`, `ref`), if any.
pub fn init_kind(init: Option<&Expr>) -> Option<Kind> {
    let Some(Expr::Call { callee, .. }) = init.map(Expr::unparen) else { return None };
    let Expr::Ident(id) = callee.unparen() else { return None };
    match id.name.as_str() {
        "sig" | "urlState" => Some(Kind::Sig),
        "drv" => Some(Kind::Drv),
        "ref" => Some(Kind::Ref),
        "form" => Some(Kind::Form),
        // `let theme = use(Theme)`: read-only and reactive when the provider passed a signal.
        "use" => Some(Kind::Drv),
        // `let users = resource(fetcher)`: read-only node whose `.v` is a Promise, replaced on revalidation (§35).
        "resource" => Some(Kind::Drv),
        // §41.6 helpers return a read-only node (`.v`) like a `drv`; `stream()` is a signal fed by the server.
        "media" | "visible" | "debounce" | "stream" => Some(Kind::Drv),
        _ => None,
    }
}

/// Declares the names introduced by one statement (block-level hoisting, no TDZ).
pub fn declare_stmt(s: &Stmt, m: &mut HashMap<String, Kind>) {
    match s {
        Stmt::Var(v) => {
            for d in &v.decls {
                let kind = match &d.pat {
                    Pat::Ident(_) => init_kind(d.init.as_ref()).unwrap_or(if v.kind == VarKind::Const { Kind::Const } else { Kind::Let }),
                    _ => {
                        if v.kind == VarKind::Const {
                            Kind::Const
                        } else {
                            Kind::Let
                        }
                    }
                };
                let mut names = Vec::new();
                d.pat.bound_names(&mut names);
                for n in names {
                    m.insert(n.name.clone(), kind);
                }
            }
        }
        Stmt::Func(f) | Stmt::Server { func: f, .. } => {
            if let Some(n) = &f.name {
                m.insert(n.name.clone(), Kind::Func);
            }
        }
        Stmt::Import(i) => {
            for id in i.default.iter().chain(i.namespace.iter()).chain(i.named.iter().map(|(_, l)| l)) {
                m.insert(id.name.clone(), Kind::Import);
            }
        }
        Stmt::Export { decl, .. } => declare_stmt(decl, m),
        _ => {}
    }
}

/// A stack of scopes; the innermost wins.
#[derive(Default, Clone, Debug)]
pub struct Scopes {
    pub frames: Vec<HashMap<String, Kind>>,
}

impl Scopes {
    pub fn new(top: HashMap<String, Kind>) -> Self {
        Scopes { frames: vec![top] }
    }
    pub fn lookup(&self, name: &str) -> Option<Kind> {
        self.frames.iter().rev().find_map(|f| f.get(name).copied())
    }
    pub fn push(&mut self) {
        self.frames.push(HashMap::new());
    }
    pub fn pop(&mut self) {
        self.frames.pop();
    }
    pub fn declare(&mut self, name: &str, k: Kind) {
        if let Some(f) = self.frames.last_mut() {
            f.insert(name.to_string(), k);
        }
    }
    pub fn declare_pat(&mut self, p: &Pat, k: Kind) {
        let mut names = Vec::new();
        p.bound_names(&mut names);
        for n in names {
            self.declare(&n.name, k);
        }
    }
    pub fn all_names(&self) -> Vec<String> {
        let mut v: Vec<String> = self.frames.iter().flat_map(|f| f.keys().cloned()).collect();
        v.sort();
        v.dedup();
        v
    }
}

/// Levenshtein distance, for "did you mean" fixes.
pub fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut cur = vec![i; b.len() + 1];
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
        }
        prev = cur;
    }
    prev[b.len()]
}

/// Closest candidate within distance 2 (or a case-insensitive match).
pub fn closest<'a>(name: &str, candidates: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    candidates
        .into_iter()
        .map(|c| (if c.eq_ignore_ascii_case(name) { 0 } else { edit_distance(name, c) }, c))
        .filter(|(d, c)| *d <= 2 && *d < name.len().max(c.len()))
        .min_by_key(|(d, _)| *d)
        .map(|(_, c)| c)
}

include!(concat!(env!("OUT_DIR"), "/runtime_exports.rs"));

/// `src` is one of the framework's `runtime/ui/*.brain` kit files, byte for byte (line endings aside).
pub fn kit_source(src: &str) -> bool {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in src.replace("\r\n", "\n").bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    KIT_HASHES.contains(&h)
}

/// Whether the runtime ships `name` in `module` (always true when the runtime folder was not available at build).
pub fn runtime_has(module: &str, name: &str) -> bool {
    !RUNTIME_KNOWN || RUNTIME_EXPORTS.iter().any(|(m, n)| *m == module && *n == name)
}
