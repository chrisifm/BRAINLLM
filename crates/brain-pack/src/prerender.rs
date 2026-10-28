//! Build-time prerender: runs the already-built chunks in Node on the runtime's DOM shim (`runtime/shim/dom.js`) and
//! returns each page's HTML, so `dist/` pages ship real content (SEO, first paint without JS). Chunks are imported
//! from a temporary copy of `dist/` through a module resolve hook that maps `/_b/<hash>.js` to files.

use std::collections::BTreeMap;
use std::io::Write;
use std::process::{Command, Stdio};

pub struct Job {
    pub route: String,
    /// `/_b/<hash>.js` URLs.
    pub core: String,
    pub view: String,
    pub layouts: Vec<String>,
    /// `file://` URL of the view's server module (`load`, `seo`, `head`), if it has one.
    pub server: Option<String>,
    /// Route params as a JSON object (`{}` for static routes).
    pub params: String,
    /// Run `load({params, …})` at build time (only for `paths()` routes, INTERNALS §32).
    pub load: bool,
    /// i18n (§37): the page's locale, its dictionary chunk (loaded before mounting) and its alternates as JSON
    /// (`[{"locale":"es","href":"/es/x"}]`), exposed as `__brainLocale` / `__brainAlternates` while rendering.
    pub locale: Option<String>,
    pub i18n: Option<String>,
    pub alternates: String,
    /// Runaway probe (§30.2, #284): seconds of virtual time to step the mounted page; 0 = no probe.
    pub probe: u32,
    /// Hydration by adoption (#287, RUNTIME #285): stamp the render with brain-hydrate.js (`data-hk`, regions).
    pub hydrate: bool,
}

/// What a job produced: the view's HTML plus its evaluated metadata.
#[derive(Debug, Clone, PartialEq)]
pub struct Rendered {
    pub html: String,
    /// `seo(props)` as (key, value) pairs; objects (jsonld) come JSON-encoded.
    pub seo: Vec<(String, String)>,
    /// `head(props)` HTML.
    pub head: String,
    /// The props the view was mounted with, as JSON (`{}` without load).
    pub props: String,
    /// The render called `fetch` (a `resource()`/`api()` read): those requests are left pending at build time, so the
    /// HTML shows the loading state and the page mounts on load to fetch for real (never a frozen build-time error).
    pub fetched: bool,
    /// Components that threw while rendering and showed their fallback (brain-guard, §42): `comp (where): message`.
    pub failed: Vec<String>,
    /// B0907 findings of the runaway probe: `comp|what|growth|msg|fix`.
    pub probe: Vec<String>,
}

/// Upper bound for the whole prerender run.
const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

const SCRIPT: &str = r#"
import * as M from 'node:module'
import { pathToFileURL } from 'node:url'
const [dist, shim, mode] = process.argv.slice(1)
const base = pathToFileURL(dist + '/').href
// `/_b/x.js` or, with brain.json `base`, `/app/_b/x.js` → dist/_b/x.js
// `format: 'module'`: chunks may use the own extension `.jsbrain` (#26), which Node does not know
const resolve = (s, c, n) => s.startsWith('/') && s.includes('/_b/') ? { url: new URL('.' + s.slice(s.indexOf('/_b/')), base).href, format: 'module', shortCircuit: true } : n(s, c)
// registerHooks is synchronous and replaces register(), deprecated in Node 26 (DEP0205)
if (M.registerHooks) M.registerHooks({ resolve })
else M.register('data:text/javascript,' + encodeURIComponent(`export async function resolve(s, c, n) { return s.startsWith('/') && s.includes('/_b/') ? { url: new URL('.' + s.slice(s.indexOf('/_b/')), ${JSON.stringify(base)}).href, format: 'module', shortCircuit: true } : n(s, c) }`))
let input = ''
for await (const c of process.stdin) input += c
const out = []
if (mode === 'paths') {
  // one entry per view: its params, one URL-encoded object per line
  for (const j of JSON.parse(input)) {
    try {
      const S = await import(j.server)
      const list = typeof S.paths === 'function' ? await S.paths() : []
      out.push({ route: j.view, html: (list || []).map(p => new URLSearchParams(Object.entries(p || {}).map(([k, v]) => [k, String(v)])).toString()).join('\n') })
    } catch (e) {
      out.push({ route: j.view, error: String(e && e.stack || e) })
    }
  }
  process.stdout.write(JSON.stringify(out))
  process.exit(0)
}
const { install } = await import(pathToFileURL(shim).href)
const enc = seo => Object.entries(seo || {}).filter(([, v]) => v != null && v !== '').map(([k, v]) => k + '\u001f' + (typeof v === 'object' ? JSON.stringify(v) : String(v))).join('\u001e')
for (const j of JSON.parse(input)) {
  const doc = install()
  const root = doc.createElement('div')
  try {
    const S = j.server ? await import(j.server) : {}
    let props = {}
    if (j.load && typeof S.load === 'function') {
      const r = await S.load({ params: j.params, query: {}, url: j.route, cookies: {}, user: null, env: process.env, req: { method: 'GET', headers: {}, ip: '' } })
      if (r && r.redirect) throw Error(`load() of ${j.route} redirects to ${r.redirect}; a prerendered page cannot redirect`)
      props = r || {}
    }
    // requests made while rendering stay pending: resource() renders its loading branch and the client fetches
    // (the build has no origin, so a real fetch would only freeze an error into the page, QA #124)
    let fetched = 0
    const realFetch = globalThis.fetch
    globalThis.fetch = () => (fetched++, new Promise(() => {}))
    // route() reads the page's path and params from here while rendering (RUNTIME #169), as under server.mjs
    globalThis.__brainRoute = { path: j.route, params: j.params || {}, query: {} }
    // i18n (§37): the page's language and its dictionary, so t() renders in it; <LangSwitch> reads the alternates
    globalThis.__brainLocale = j.locale || null
    globalThis.__brainAlternates = j.alternates || null
    if (j.i18n) await import(j.i18n)
    // components that fall back while rendering report here (the same contract server.mjs logs from, QA #176)
    const failed = (globalThis.__brainSSR = [])
    const { m } = await import(j.core)
    const V = (await import(j.view)).default
    const L = []
    for (const l of j.layouts) L.push((await import(l)).default)
    let tree = () => V(props)
    for (let i = L.length - 1; i > 0; i--) { const inner = tree, C = L[i]; tree = () => C({ children: inner }) }
    // #287: the build of an adopting app stamps its HTML (data-hk, <!--[-->…<!--]-->) like server.mjs does
    const C0 = j.hydrate ? await import(j.core) : null
    // the unminified module (the client chunk in _b/ only keeps fc/nx): written next to dom.js as hydrate.js
    const unstamp = C0 && C0._rt ? (await import(new URL('./hydrate.js', base).href)).stamp(C0._rt()) : null
    try {
      if (L.length) m(L[0], root, { children: tree }); else m(V, root, props)
    } finally {
      if (unstamp) unstamp()
    }
    await new Promise(r => setTimeout(r, 0))
    // bind:value/checked set properties, which innerHTML does not serialize: copy them into the markup so the page
    // shows the right content before the island resumes
    const walk = n => {
      for (const c of n.childNodes || []) {
        if (c.nodeType !== 1) continue
        const t = (c.tagName || '').toLowerCase()
        if (t === 'textarea' && c.value != null) c.textContent = String(c.value)
        else if (t === 'input' && c.type !== 'password') {
          if (c.value != null && c.value !== '') c.setAttribute('value', String(c.value))
          if (c.checked === true) c.setAttribute('checked', '')
        }
        walk(c)
      }
    }
    walk(root)
    const seo = typeof S.seo === 'function' ? enc(S.seo(props)) : ''
    const head = typeof S.head === 'function' ? String(S.head(props) || '') : ''
    // §30.2 runaway probe (#284): the same page mounted again with fake timers for j.probe virtual seconds
    let probed = ''
    if (j.probe > 0) {
      try {
        const { probe } = await import(new URL('./probe.js', base).href)
        const C = { ...(await import(j.core)) }
        for (const [long, short] of (await import(new URL('./probe-names.js', base).href)).default) if (C[long] === undefined && C[short] !== undefined) C[long] = C[short]
        const mountIt = r => (L.length ? C.m(L[0], r, { children: tree }) : C.m(V, r, props))
        const found = await probe({ core: C, mount: mountIt, seconds: j.probe, document: doc })
        probed = (found || []).map(f => [f.comp || '', f.what || '', f.growth || '', String(f.msg || '').replace(/[|\u001e]/g, ' '), String(f.fix || '').replace(/[|\u001e]/g, ' ')].join('|')).join('\u001e')
      } catch (e) {
        probed = '|probe|0|the runaway probe failed: ' + String((e && e.message) || e).split('\n')[0] + '|'
      }
    }
    globalThis.fetch = realFetch
    globalThis.__brainRoute = null
    globalThis.__brainSSR = null
    globalThis.__brainLocale = globalThis.__brainAlternates = null
    const fails = failed.map(f => `${f.comp || 'component'}${f.w ? ' (' + f.w + ')' : ''}: ${String((f.e && f.e.message) || f.e).split('\n')[0]}`).join('\u001e')
    out.push({ route: j.route, html: root.innerHTML, seo, head, props: JSON.stringify(props), fetch: fetched ? '1' : '', failed: fails, probe: probed })
  } catch (e) {
    out.push({ route: j.route, error: String(e && e.stack || e) })
  }
}
// pending requests may hold timers (api() timeouts): exit once the result is flushed
process.stdout.write(JSON.stringify(out), () => process.exit(0))
"#;

fn json_str(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

/// Runs the prerender script in `mode` ("render" or "paths") over a temporary copy of the `_b/` chunks, feeding
/// `input` on stdin; returns stdout. Killed after TIMEOUT.
fn node(files: &BTreeMap<String, String>, shim: &str, mode: &str, input: &str) -> Result<String, String> {
    node_with(files, shim, &Aux::default(), mode, input)
}

/// RUNTIME modules the render script imports unminified, written next to dom.js.
#[derive(Default, Clone, Copy)]
pub struct Aux<'a> {
    /// runtime/shim/probe.js (#284).
    pub probe: Option<&'a str>,
    /// runtime/brain-hydrate.js (#287).
    pub hydrate: Option<&'a str>,
}

fn node_with(files: &BTreeMap<String, String>, shim: &str, aux: &Aux, mode: &str, input: &str) -> Result<String, String> {
    let tmp = std::env::temp_dir().join(format!("brain-prerender-{}-{mode}-{}", std::process::id(), crate::content_hash(input)));
    let _ = std::fs::remove_dir_all(&tmp);
    for (p, c) in files.iter().filter(|(p, _)| p.starts_with("_b/")) {
        let f = tmp.join(p);
        std::fs::create_dir_all(f.parent().unwrap()).map_err(|e| e.to_string())?;
        std::fs::write(&f, c).map_err(|e| e.to_string())?;
    }
    std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
    let shim_path = tmp.join("dom.js");
    std::fs::write(&shim_path, shim).map_err(|e| e.to_string())?;
    if let Some(h) = aux.hydrate {
        std::fs::write(tmp.join("hydrate.js"), h).map_err(|e| e.to_string())?;
    }
    if let Some(p) = aux.probe {
        std::fs::write(tmp.join("probe.js"), p).map_err(|e| e.to_string())?;
        // the probe reads the core by its readable export names; the build core is published with the short ones
        let names: Vec<String> = brain_min::RUNTIME_MAP.iter().map(|(a, b)| format!("[\"{a}\",\"{b}\"]")).collect();
        std::fs::write(tmp.join("probe-names.js"), format!("export default [{}]\n", names.join(","))).map_err(|e| e.to_string())?;
    }
    let node = std::env::var("BRAIN_NODE").unwrap_or_else(|_| "node".into());
    let mut child = Command::new(&node)
        .args(["--input-type=module", "-e", SCRIPT, &tmp.to_string_lossy(), &shim_path.to_string_lossy(), mode])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("prerender needs Node ({e}); fix: install Node 22+ or set BRAIN_NODE"))?;
    child.stdin.take().unwrap().write_all(input.as_bytes()).map_err(|e| e.to_string())?;
    // Read both pipes on threads and poll the child, so a view that never settles is killed after TIMEOUT instead of
    // hanging `brain build` (INTERNALS §25: every command terminates).
    let drain = |r: Option<Box<dyn std::io::Read + Send>>| {
        std::thread::spawn(move || {
            let mut b = Vec::new();
            if let Some(mut r) = r {
                let _ = r.read_to_end(&mut b);
            }
            b
        })
    };
    let out_t = drain(child.stdout.take().map(|r| Box::new(r) as Box<dyn std::io::Read + Send>));
    let err_t = drain(child.stderr.take().map(|r| Box::new(r) as Box<dyn std::io::Read + Send>));
    let start = std::time::Instant::now();
    let status = loop {
        match child.try_wait().map_err(|e| e.to_string())? {
            Some(s) => break s,
            None if start.elapsed() > TIMEOUT => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = std::fs::remove_dir_all(&tmp);
                return Err(format!(
                    "prerender did not finish in {} s; fix: a view keeps work pending at mount (an unresolved load or an endless timer); check the views changed last with `brain run` and the browser console",
                    TIMEOUT.as_secs()
                ));
            }
            None => std::thread::sleep(std::time::Duration::from_millis(20)),
        }
    };
    let (stdout, stderr) = (out_t.join().unwrap_or_default(), err_t.join().unwrap_or_default());
    let _ = std::fs::remove_dir_all(&tmp);
    if !status.success() {
        return Err(format!("prerender failed: {}", String::from_utf8_lossy(&stderr).lines().take(8).collect::<Vec<_>>().join("\n")));
    }
    Ok(String::from_utf8_lossy(&stdout).into_owned())
}

/// Renders every job. Returns route → rendered page, or route → `Err(stack)` for a view that threw.
pub fn run(files: &BTreeMap<String, String>, shim: &str, jobs: &[Job]) -> Result<BTreeMap<String, Result<Rendered, String>>, String> {
    run_probed(files, shim, &Aux::default(), jobs)
}

/// `run` with RUNTIME's runaway probe (`runtime/shim/probe.js`) for the jobs that ask for it (#284).
pub fn run_probed(files: &BTreeMap<String, String>, shim: &str, aux: &Aux, jobs: &[Job]) -> Result<BTreeMap<String, Result<Rendered, String>>, String> {
    if jobs.is_empty() {
        return Ok(BTreeMap::new());
    }
    let input = format!(
        "[{}]",
        jobs.iter()
            .map(|j| format!(
                "{{\"route\":{},\"core\":{},\"view\":{},\"layouts\":[{}],\"server\":{},\"params\":{},\"load\":{},\"locale\":{},\"i18n\":{},\"alternates\":{},\"probe\":{},\"hydrate\":{}}}",
                json_str(&j.route),
                json_str(&j.core),
                json_str(&j.view),
                j.layouts.iter().map(|l| json_str(l)).collect::<Vec<_>>().join(","),
                j.server.as_deref().map_or("null".to_string(), json_str),
                if j.params.trim().is_empty() { "{}" } else { &j.params },
                j.load,
                j.locale.as_deref().map_or("null".to_string(), json_str),
                j.i18n.as_deref().map_or("null".to_string(), json_str),
                if j.alternates.trim().is_empty() { "null" } else { &j.alternates },
                j.probe,
                j.hydrate
            ))
            .collect::<Vec<_>>()
            .join(",")
    );
    let out = node_with(files, shim, aux, "render", &input)?;
    let mut res = BTreeMap::new();
    for (route, r) in brain_min_json::parse(&out)? {
        res.insert(
            route,
            r.map(|f| {
                let get = |k: &str| f.iter().find(|(a, _)| a == k).map(|(_, v)| v.clone()).unwrap_or_default();
                let seo = get("seo")
                    .split('\u{1e}')
                    .filter(|x| !x.is_empty())
                    .filter_map(|kv| kv.split_once('\u{1f}').map(|(k, v)| (k.to_string(), v.to_string())))
                    .collect();
                let props = get("props");
                Rendered { html: get("html"), seo, head: get("head"), props: if props.is_empty() { "{}".into() } else { props }, fetched: get("fetch") == "1", failed: get("failed").split('\u{1e}').filter(|x| !x.is_empty()).map(String::from).collect(), probe: get("probe").split('\u{1e}').filter(|x| !x.is_empty()).map(String::from).collect() }
            }),
        );
    }
    Ok(res)
}

/// Calls `paths()` of each `(view, server module URL)`. Returns view → list of params (key, value) per page.
pub fn paths(files: &BTreeMap<String, String>, shim: &str, views: &[(String, String)]) -> Result<BTreeMap<String, Result<Vec<Vec<(String, String)>>, String>>, String> {
    if views.is_empty() {
        return Ok(BTreeMap::new());
    }
    let input = format!("[{}]", views.iter().map(|(v, s)| format!("{{\"view\":{},\"server\":{}}}", json_str(v), json_str(s))).collect::<Vec<_>>().join(","));
    let out = node(files, shim, "paths", &input)?;
    let mut res = BTreeMap::new();
    for (view, r) in brain_min_json::parse(&out)? {
        res.insert(
            view,
            r.map(|f| {
                let lines = f.iter().find(|(k, _)| k == "html").map(|(_, v)| v.clone()).unwrap_or_default();
                lines.lines().filter(|l| !l.is_empty()).map(|l| l.split('&').filter_map(|kv| kv.split_once('=')).map(|(k, v)| (form_decode(k), form_decode(v))).collect()).collect()
            }),
        );
    }
    Ok(res)
}

/// `application/x-www-form-urlencoded` decoding (what URLSearchParams.toString() produces).
fn form_decode(s: &str) -> String {
    let b = s.replace('+', " ");
    let b = b.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or("zz"), 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Tiny reader for the fixed result shape (an array of flat objects with string fields), to avoid a JSON dependency.
/// Each item becomes (route, Ok(fields) | Err(error)).
mod brain_min_json {
    type Item = (String, Result<Vec<(String, String)>, String>);

    pub fn parse(s: &str) -> Result<Vec<Item>, String> {
        let b: Vec<char> = s.trim().chars().collect();
        let mut i = 0;
        let mut out = Vec::new();
        let expect = |i: &mut usize, c: char| -> Result<(), String> {
            while *i < b.len() && b[*i].is_whitespace() {
                *i += 1;
            }
            if b.get(*i) == Some(&c) {
                *i += 1;
                Ok(())
            } else {
                Err(format!("prerender output: expected `{c}` at {}", *i))
            }
        };
        let string = |i: &mut usize| -> Result<String, String> {
            expect(i, '"')?;
            let mut o = String::new();
            while *i < b.len() && b[*i] != '"' {
                if b[*i] == '\\' {
                    *i += 1;
                    match b.get(*i) {
                        Some('n') => o.push('\n'),
                        Some('t') => o.push('\t'),
                        Some('r') => o.push('\r'),
                        Some('b') => o.push('\u{8}'),
                        Some('f') => o.push('\u{c}'),
                        Some('u') => {
                            let h: String = b[*i + 1..(*i + 5).min(b.len())].iter().collect();
                            let mut cp = u32::from_str_radix(&h, 16).unwrap_or(0xfffd);
                            *i += 4;
                            // surrogate pair (\ud83d\ude00)
                            if (0xd800..0xdc00).contains(&cp) && b.get(*i + 1) == Some(&'\\') && b.get(*i + 2) == Some(&'u') {
                                let h2: String = b[*i + 3..(*i + 7).min(b.len())].iter().collect();
                                if let Ok(lo) = u32::from_str_radix(&h2, 16) {
                                    if (0xdc00..0xe000).contains(&lo) {
                                        cp = 0x10000 + ((cp - 0xd800) << 10) + (lo - 0xdc00);
                                        *i += 6;
                                    }
                                }
                            }
                            o.push(char::from_u32(cp).unwrap_or('\u{fffd}'));
                        }
                        Some(c) => o.push(*c),
                        None => {}
                    }
                } else {
                    o.push(b[*i]);
                }
                *i += 1;
            }
            *i += 1;
            Ok(o)
        };
        expect(&mut i, '[')?;
        loop {
            while i < b.len() && (b[i].is_whitespace() || b[i] == ',') {
                i += 1;
            }
            if b.get(i) == Some(&']') {
                break;
            }
            expect(&mut i, '{')?;
            let (mut route, mut fields, mut err) = (String::new(), Vec::new(), None);
            loop {
                let k = string(&mut i)?;
                expect(&mut i, ':')?;
                let v = string(&mut i)?;
                match k.as_str() {
                    "route" => route = v,
                    "error" => err = Some(v),
                    _ => fields.push((k, v)),
                }
                while i < b.len() && b[i].is_whitespace() {
                    i += 1;
                }
                if b.get(i) == Some(&',') {
                    i += 1;
                    continue;
                }
                expect(&mut i, '}')?;
                break;
            }
            out.push((route, match err {
                Some(e) => Err(e),
                None => Ok(fields),
            }));
        }
        Ok(out)
    }
}

/// `<head>` SEO tags from `<meta seo …>` attributes (INTERNALS §8): title, description, canonical, Open Graph,
/// robots and JSON-LD. `site` makes canonical/og:url absolute.
pub fn seo_head(seo: &[(String, String)], site: Option<&str>, route: &str, generator: &str) -> (Option<String>, String) {
    let esc = |s: &str| s.replace('&', "&amp;").replace('"', "&quot;").replace('<', "&lt;").replace('>', "&gt;");
    let get = |k: &str| seo.iter().find(|(a, _)| a == k).map(|(_, v)| v.as_str()).filter(|v| !v.is_empty());
    // URLs from seo(props) may carry raw params (`/post/b c`): percent-encode what is not legal in a URL
    let url = |p: &str| {
        p.bytes()
            .map(|b| match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b':' | b'/' | b'?' | b'#' | b'[' | b']' | b'@' | b'!' | b'$' | b'&' | b'\'' | b'(' | b')' | b'*' | b'+' | b',' | b';' | b'=' | b'%' => (b as char).to_string(),
                _ => format!("%{b:02X}"),
            })
            .collect::<String>()
    };
    let abs = |p: &str| match site {
        Some(s) if p.starts_with('/') => url(&format!("{}{p}", s.trim_end_matches('/'))),
        _ => url(p),
    };
    let mut h = String::new();
    if !generator.is_empty() {
        h.push_str(&format!("<meta name=\"generator\" content=\"{}\">", esc(generator)));
    }
    // app.brain's `<meta seo titleTemplate="%s · Site">` wraps every view title
    let title = get("title").map(|t| match get("titleTemplate") {
        Some(tpl) if tpl.contains("%s") => tpl.replace("%s", t),
        _ => t.to_string(),
    });
    if let Some(d) = get("description") {
        h.push_str(&format!("<meta name=\"description\" content=\"{}\">", esc(d)));
    }
    // the not-found page (`*`) and server templates (`!error:…`) are not URLs: no canonical and no og:url (#177)
    let canonical = if !route.starts_with('/') { None } else { get("canonical").map(abs).or_else(|| site.map(|_| abs(route))) };
    if let Some(c) = &canonical {
        h.push_str(&format!("<link rel=\"canonical\" href=\"{}\">", esc(c)));
    }
    if let Some(r) = get("robots") {
        h.push_str(&format!("<meta name=\"robots\" content=\"{}\">", esc(r)));
    }
    if let Some(t) = &title {
        h.push_str(&format!("<meta property=\"og:title\" content=\"{}\">", esc(t)));
    }
    if let Some(d) = get("description") {
        h.push_str(&format!("<meta property=\"og:description\" content=\"{}\">", esc(d)));
    }
    if let Some(c) = &canonical {
        h.push_str(&format!("<meta property=\"og:url\" content=\"{}\">", esc(c)));
    }
    // `<meta seo twitter="@handle">` (SPEC «SEO»): app.brain's default, a view's value wins through the merge
    if let Some(x) = get("twitter") {
        h.push_str(&format!("<meta name=\"twitter:site\" content=\"{}\">", esc(x)));
    }
    if let Some(img) = get("og:image") {
        h.push_str(&format!("<meta property=\"og:image\" content=\"{}\">", esc(&abs(img))));
    }
    h.push_str("<meta property=\"og:type\" content=\"website\">");
    // `jsonld={…}` (INTERNALS §32): the object as given, with a default @context
    if let Some(ld) = get("jsonld") {
        let ld = ld.trim();
        let ld = if ld.starts_with('{') && !ld.contains("\"@context\"") { format!("{{\"@context\":\"https://schema.org\",{}", &ld[1..]) } else { ld.to_string() };
        let ld = if ld.ends_with(",}") { ld.replace(",}", "}") } else { ld };
        h.push_str(&format!("<script type=\"application/ld+json\">{}</script>", ld.replace("</", "<\\/")));
    } else if let Some(schema) = get("schema") {
        let mut ld = format!("{{\"@context\":\"https://schema.org\",\"@type\":{}", json_str(schema));
        if let Some(t) = &title {
            ld.push_str(&format!(",\"name\":{}", json_str(t)));
        }
        if let Some(d) = get("description") {
            ld.push_str(&format!(",\"description\":{}", json_str(d)));
        }
        if let Some(c) = &canonical {
            ld.push_str(&format!(",\"url\":{}", json_str(c)));
        }
        ld.push('}');
        h.push_str(&format!("<script type=\"application/ld+json\">{}</script>", ld.replace("</", "<\\/")));
    }
    (title, h)
}

/// Key of a head tag for deduplication: `name`/`property` of a `<meta>`, `rel`(+`hreflang`) of a `<link>` other than
/// stylesheets/preloads, or the tag itself for `<title>`. Tags without a key are always kept.
fn head_key(tag: &str) -> Option<String> {
    let attr = |n: &str| {
        let i = tag.find(&format!(" {n}=\""))? + n.len() + 3;
        Some(tag[i..i + tag[i..].find('"')?].to_string())
    };
    let lower = tag.to_ascii_lowercase();
    if lower.starts_with("<title") {
        return Some("title".into());
    }
    if lower.starts_with("<meta") {
        if lower.contains(" charset=") {
            return Some("charset".into());
        }
        return attr("name").map(|n| format!("name:{n}")).or_else(|| attr("property").map(|p| format!("property:{p}")));
    }
    if lower.starts_with("<link") {
        let rel = attr("rel")?;
        // the same module preloaded twice (a chunk the view reaches and the page script imports): keep one
        if matches!(rel.as_str(), "modulepreload" | "preload") {
            return attr("href").map(|h| format!("rel:{rel}|href:{h}"));
        }
        if matches!(rel.as_str(), "stylesheet" | "icon") {
            return None;
        }
        return Some(format!("rel:{rel}|{}", attr("hreflang").unwrap_or_default()));
    }
    None
}

/// Deduplicates the `<head>` of a full document by `head_key`: the LAST occurrence wins, so the order
/// app.brain → `<meta seo>` → the view's `<head>` block lets the most specific source override (INTERNALS §32).
pub fn dedupe_head(doc: &str) -> String {
    let (Some(a), Some(b)) = (doc.find("<head"), doc.find("</head>")) else { return doc.to_string() };
    let Some(a) = doc[a..].find('>').map(|i| a + i + 1) else { return doc.to_string() };
    if a > b {
        return doc.to_string();
    }
    let head = &doc[a..b];
    // split into top-level tags (meta/link/title; script and style blocks are copied whole)
    let mut parts: Vec<(Option<String>, &str)> = Vec::new();
    let mut i = 0;
    while i < head.len() {
        let Some(lt) = head[i..].find('<').map(|x| i + x) else {
            parts.push((None, &head[i..]));
            break;
        };
        if lt > i {
            parts.push((None, &head[i..lt]));
        }
        let rest = &head[lt..];
        let close = ["title", "script", "style", "noscript"].iter().find_map(|t| {
            rest.to_ascii_lowercase().starts_with(&format!("<{t}")).then(|| rest.find(&format!("</{t}>")).map(|e| e + t.len() + 3))
        });
        let end = match close {
            Some(Some(e)) => lt + e,
            _ => lt + rest.find('>').map_or(rest.len(), |e| e + 1),
        };
        let tag = &head[lt..end];
        parts.push((head_key(tag), tag));
        i = end;
    }
    let mut seen = std::collections::HashSet::new();
    let mut keep = vec![true; parts.len()];
    for (n, (k, _)) in parts.iter().enumerate().rev() {
        if let Some(k) = k {
            if !seen.insert(k.clone()) {
                keep[n] = false;
            }
        }
    }
    let head: String = parts.iter().zip(keep).filter(|(_, k)| *k).map(|((_, t), _)| *t).collect();
    format!("{}{head}{}", &doc[..a], &doc[b..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn twitter_site_from_meta_seo() {
        let kv = |v: &[(&str, &str)]| v.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect::<Vec<_>>();
        let (_, h) = seo_head(&kv(&[("title", "T"), ("twitter", "@h")]), None, "/", "");
        assert!(h.contains("<meta name=\"twitter:site\" content=\"@h\">"), "{h}");
        let (_, h) = seo_head(&kv(&[("title", "T")]), None, "/", "");
        assert!(!h.contains("twitter:site"), "{h}");
    }

    #[test]
    fn seo_tags() {
        let seo: Vec<(String, String)> = [("title", "Tasks"), ("description", "Manage \"tasks\""), ("canonical", "/tasks"), ("schema", "WebApplication")]
            .iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect();
        let (title, h) = seo_head(&seo, Some("https://example.dev/"), "/tasks", "BRAINLLM 0.1.0");
        assert_eq!(title.as_deref(), Some("Tasks"));
        assert!(h.contains("<link rel=\"canonical\" href=\"https://example.dev/tasks\">"));
        assert!(h.contains("content=\"Manage &quot;tasks&quot;\""));
        assert!(h.contains("\"@type\":\"WebApplication\""));
        assert!(h.contains("<meta name=\"generator\" content=\"BRAINLLM 0.1.0\">"));
    }

    #[test]
    fn dedupes_repeated_module_preloads() {
        let d = dedupe_head("<html><head><link rel=\"modulepreload\" href=\"/_b/a.js\"><link rel=\"modulepreload\" href=\"/_b/b.js\"><link rel=\"modulepreload\" href=\"/_b/a.js\"></head><body></body></html>");
        assert_eq!(d.matches("/_b/a.js").count(), 1, "{d}");
        assert!(d.contains("/_b/b.js"));
    }

    #[test]
    fn dedupes_head_last_wins() {
        let d = "<html><head><meta charset=\"utf-8\"><title>App</title><meta name=\"description\" content=\"app\"><link rel=\"stylesheet\" href=\"/a.css\"><title>View</title><meta name=\"description\" content=\"view\"><link rel=\"alternate\" hreflang=\"es\" href=\"/es\"><link rel=\"alternate\" hreflang=\"en\" href=\"/en\"><script type=\"application/ld+json\">{\"a\":\"<title>\"}</script></head><body></body></html>";
        let o = dedupe_head(d);
        assert_eq!(o.matches("<title>").count(), 2, "{o}"); // the view title + the one inside the JSON-LD string
        assert!(o.contains("<title>View</title>") && !o.contains("<title>App</title>"), "{o}");
        assert!(o.contains("content=\"view\"") && !o.contains("content=\"app\""), "{o}");
        assert!(o.contains("hreflang=\"es\"") && o.contains("hreflang=\"en\"") && o.contains("/a.css"), "{o}");
    }

    #[test]
    fn jsonld_and_title_template() {
        let seo: Vec<(String, String)> = [("titleTemplate", "%s · Site"), ("title", "Post"), ("jsonld", "{\"@type\":\"Article\",\"headline\":\"</script>\"}")].iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
        let (title, h) = seo_head(&seo, None, "/p", "");
        assert_eq!(title.as_deref(), Some("Post · Site"));
        assert!(h.contains("{\"@context\":\"https://schema.org\",\"@type\":\"Article\",\"headline\":\"<\\/script>\"}"), "{h}");
    }

    #[test]
    fn parses_results() {
        let r = brain_min_json::parse(r#"[{"route":"/","html":"<p>a\"b \ud83d\ude00</p>","seo":"t"},{"route":"/x","error":"Error: boom"}]"#).unwrap();
        assert_eq!(r[0].0, "/");
        assert_eq!(r[0].1.as_ref().unwrap()[0], ("html".to_string(), "<p>a\"b \u{1f600}</p>".to_string()));
        assert!(r[1].1.is_err());
        assert_eq!(form_decode("a%20b+c%2Fd"), "a b c/d");
    }
}
