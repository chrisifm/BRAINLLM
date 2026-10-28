//! `load(ctx)` in dev (INTERNALS §11.1): each view's server code (`Output.server_js`) is written to
//! `<app>/.brain/server/` with its imports resolved to absolute paths, and a persistent Node process
//! (sidecar) runs it. Line protocol: Rust → `{"id","file","fn","ctx","args"?}` · Node → `OK <id> <json>` |
//! `ERR <id> <json>` | `BAD <id> {"error","errors"}` (the function threw `{errors}`: a 422 for RPC, §11.1).

use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};

const RUNNER: &str = r#"
import { createInterface } from 'node:readline'
// stdout is the protocol channel: the user's console.log calls go to stderr (brain run terminal).
const out = process.stdout.write.bind(process.stdout)
console.log = console.info = (...a) => console.error(...a)
// response headers as "name\u0001value\u0002…" (a header may repeat: set-cookie)
const flat = h => Object.entries(h || {}).flatMap(([k, v]) => (Array.isArray(v) ? v : [v]).map(x => k + '\u0001' + x)).join('\u0002')
// the request's session from its signed cookie (server.mjs sessionFrom), so login/logout set cookies in dev too
const withSession = (S, ctx, secret) => {
  const ses = S.sessionFrom(ctx.cookies || {}, secret || '')
  return { ...ctx, url: new URL(ctx.url, 'http://localhost'), env: process.env, user: ses.user, session: ses.session }
}
for await (const line of createInterface({ input: process.stdin })) {
  const { id, file, fn, ctx, args, rpc, raw, key, form, rules, chain, guards, middleware, secret } = JSON.parse(line)
  if (chain) {
    // _guard/_middleware of the route through server.mjs's runChain, with ctx.user from the signed session cookie
    try {
      const S = await import(chain)
      const G = [], M = []
      for (const u of guards || []) G.push(await import(u))
      for (const u of middleware || []) M.push(await import(u))
      const c = { ...ctx, url: new URL(ctx.url, 'http://localhost'), env: process.env }
      const ses = S.sessionFrom(c.cookies || {}, secret || '')
      c.user = ses.user
      c.session = ses.session
      const r = await S.runChain({ guards: G, middleware: M, ctx: c })
      const kind = r.redirect ? 'redirect' : r.deny !== undefined ? 'deny' : 'ok'
      out(`CHN ${id} ${JSON.stringify({ kind, location: r.redirect || '', status: String(r.status || 303), reason: String(r.deny ?? ''), user: JSON.stringify(c.user ?? null) })}\n`)
    } catch (e) {
      console.error(String(e?.stack ?? e))
      out(`CHN ${id} ${JSON.stringify({ kind: 'error', reason: String(e?.message ?? e) })}\n`)
    }
    continue
  }
  if (form) {
    // a form posted without JS (§38) through server.mjs's callForm, with the same rules as form() in the browser
    try {
      const S = await import(form), R = await import(rules), m = await import(file)
      const c = withSession(S, ctx, secret)
      const r = await S.callForm({ module: m, raw, ctx: c, rules: R, key })
      if (r.cause) console.error(String(r.cause?.stack ?? r.cause))
      out(`FRM ${id} ${JSON.stringify({ status: r.status, location: r.location || '', state: JSON.stringify(r.state || null), body: JSON.stringify(r.body || null), headers: flat({ ...(r.headers || {}), ...c.session.headers() }) })}\n`)
    } catch (e) {
      console.error(String(e?.stack ?? e))
      out(`FRM ${id} ${JSON.stringify({ status: 500, location: '', state: 'null', body: '{"error":"Internal error"}' })}\n`)
    }
    continue
  }
  if (rpc) {
    // RPC through the production handler (runtime/server/server.mjs callRpc): same parsing, limits and errors
    try {
      const S = await import(rpc)
      const m = await import(file)
      const c = withSession(S, ctx, secret)
      const r = await S.callRpc({ fn: m[fn], args: raw, ctx: c, key })
      if (r.cause) console.error(String(r.cause?.stack ?? r.cause))
      out(`RES ${id} ${JSON.stringify({ status: r.status, body: JSON.stringify(r.body ?? null), headers: flat({ ...(r.headers || {}), ...c.session.headers() }) })}\n`)
    } catch (e) {
      console.error(String(e?.stack ?? e))
      out(`RES ${id} ${JSON.stringify({ status: 500, body: '{"error":"Internal error"}' })}\n`)
    }
    continue
  }
  try {
    const m = await import(file)
    if (typeof m[fn] !== 'function') throw new Error(`${fn} is not exported by the view's server code`)
    const v = await m[fn]({ ...ctx, env: process.env }, ...(args || []))
    out(`OK ${id} ${JSON.stringify(v ?? null)}\n`)
  } catch (e) {
    if (e && e.errors && typeof e.errors === 'object') out(`BAD ${id} ${JSON.stringify({ error: e.message || 'Invalid input', errors: e.errors })}\n`)
    else out(`ERR ${id} ${JSON.stringify(String(e?.stack ?? e))}\n`)
  }
}
"#;

pub struct Sidecar {
    child: Child,
    stdin: ChildStdin,
    /// Lines from Node's stdout, read on a thread so a hung `load()` can time out instead of blocking forever.
    lines: std::sync::mpsc::Receiver<String>,
    next: u64,
    timeout: std::time::Duration,
}

/// A `load()` that does not answer in this time is killed (INTERNALS §25: every command terminates).
const MAX_RESPONSE: u64 = 16 * 1024 * 1024;

const LOAD_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

impl Sidecar {
    /// `vars`: the resolved environment (`.brain_env` development section over the process), seen as `ctx.env`.
    pub fn spawn(root: &Path, vars: &[(String, String)]) -> io::Result<Sidecar> {
        let mut child = Command::new(std::env::var("BRAIN_NODE").unwrap_or_else(|_| "node".into()))
            .args(["--input-type=module", "-e", RUNNER])
            .envs(vars.iter().map(|(k, v)| (k.as_str(), v.as_str())))
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| io::Error::new(e.kind(), format!("could not launch node for load(): {e}; fix: install Node ≥ 22 or set BRAIN_NODE")))?;
        let stdin = child.stdin.take().unwrap();
        let mut stdout = BufReader::new(child.stdout.take().unwrap());
        let (tx, lines) = std::sync::mpsc::sync_channel(2);
        std::thread::spawn(move || {
            let mut line = String::new();
            // Cap a frame before allocating an arbitrary line from user code. A full queue backpressures Node.
            while (&mut stdout).take(MAX_RESPONSE + 1).read_line(&mut line).is_ok_and(|n| n > 0) {
                if line.len() as u64 > MAX_RESPONSE { break; }
                if tx.send(std::mem::take(&mut line)).is_err() {
                    break;
                }
            }
        });
        Ok(Sidecar { child, stdin, lines, next: 0, timeout: LOAD_TIMEOUT })
    }

    fn response(&mut self, deadline: std::time::Instant) -> Result<String, (u16, String)> {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        match self.lines.recv_timeout(remaining) {
            Ok(line) if !remaining.is_zero() => Ok(line),
            outcome => {
                let timed_out = remaining.is_zero() || matches!(outcome, Err(std::sync::mpsc::RecvTimeoutError::Timeout));
                let _ = self.child.kill();
                let _ = self.child.wait();
                Err((if timed_out { 504 } else { 500 }, if timed_out {
                    "the load() node process exited: request timed out; fix: use bounded awaits and no infinite loops".into()
                } else {
                    "the load() node process exited or exceeded the 16 MiB response limit; check the brain run terminal".into()
                }))
            }
        }
    }

    /// Calls `fn(ctx)` from module `file` (URL `file://…?v=…`). `ctx_json` is the already-built JSON object.
    /// Returns the result JSON or the error (stack) as text.
    pub fn call(&mut self, file: &str, func: &str, ctx_json: &str) -> Result<String, String> {
        self.invoke(file, func, ctx_json, None).map_err(|(_, e)| e)
    }

    /// `POST /_r/<hash>/<fn>` through server.mjs's `callRpc` (`server` = its file URL): returns (status, JSON body).
    pub fn rpc(&mut self, file: &str, func: &str, ctx_json: &str, raw: &str, server: &str, key: &str, secret: &str) -> Result<(u16, String, Vec<(String, String)>), String> {
        self.next += 1;
        let id = self.next;
        let req = format!(
            "{{\"id\":{id},\"file\":{},\"fn\":{},\"ctx\":{ctx_json},\"rpc\":{},\"raw\":{},\"key\":{},\"secret\":{}}}\n",
            crate::json_str(file),
            crate::json_str(func),
            crate::json_str(server),
            crate::json_str(raw),
            crate::json_str(key),
            crate::json_str(secret)
        );
        self.stdin.write_all(req.as_bytes()).and_then(|_| self.stdin.flush()).map_err(|e| format!("node sidecar: {e}"))?;
        let deadline = std::time::Instant::now() + self.timeout;
        loop {
            let line = self.response(deadline).map_err(|(_, message)| message)?;
            let rest = line.trim_end();
            let Some(rest) = rest.strip_prefix("RES ") else { continue };
            let (rid, payload) = rest.split_once(' ').unwrap_or((rest, "{}"));
            if rid != id.to_string() {
                continue;
            }
            let fields = crate::json_flat(payload).unwrap_or_default();
            let status = fields.iter().find(|(k, _)| k == "status").and_then(|(_, v)| v.parse().ok()).unwrap_or(500);
            let headers = unflat(fields.iter().find(|(k, _)| k == "headers").map(|(_, v)| v.as_str()).unwrap_or(""));
            let body = fields.into_iter().find(|(k, _)| k == "body").map(|(_, v)| v).unwrap_or_else(|| "null".into());
            return Ok((status, body, headers));
        }
    }

    /// Runs a route's guards and middleware (server.mjs `runChain`): fields kind (ok|redirect|deny|error), location,
    /// status, reason, user (JSON).
    pub fn chain(&mut self, server: &str, guards: &[String], middleware: &[String], ctx_json: &str, secret: &str) -> Result<Vec<(String, String)>, String> {
        self.next += 1;
        let id = self.next;
        let list = |v: &[String]| format!("[{}]", v.iter().map(|u| crate::json_str(u)).collect::<Vec<_>>().join(","));
        let req = format!(
            "{{\"id\":{id},\"ctx\":{ctx_json},\"chain\":{},\"guards\":{},\"middleware\":{},\"secret\":{}}}\n",
            crate::json_str(server),
            list(guards),
            list(middleware),
            crate::json_str(secret)
        );
        self.stdin.write_all(req.as_bytes()).and_then(|_| self.stdin.flush()).map_err(|e| format!("node sidecar: {e}"))?;
        let deadline = std::time::Instant::now() + self.timeout;
        loop {
            let line = self.response(deadline).map_err(|(_, message)| message)?;
            let Some(rest) = line.trim_end().strip_prefix("CHN ") else { continue };
            let (rid, payload) = rest.split_once(' ').unwrap_or((rest, "{}"));
            if rid != id.to_string() {
                continue;
            }
            return Ok(crate::json_flat(payload).unwrap_or_default());
        }
    }

    /// A form-encoded POST through server.mjs's `callForm`: (status, location, state JSON, body JSON).
    pub fn form(&mut self, file: &str, ctx_json: &str, raw: &str, server: &str, rules: &str, key: &str, secret: &str) -> Result<(u16, String, String, String, Vec<(String, String)>), String> {
        self.next += 1;
        let id = self.next;
        let req = format!(
            "{{\"id\":{id},\"file\":{},\"ctx\":{ctx_json},\"form\":{},\"rules\":{},\"raw\":{},\"key\":{},\"secret\":{}}}\n",
            crate::json_str(file),
            crate::json_str(server),
            crate::json_str(rules),
            crate::json_str(raw),
            crate::json_str(key),
            crate::json_str(secret)
        );
        self.stdin.write_all(req.as_bytes()).and_then(|_| self.stdin.flush()).map_err(|e| format!("node sidecar: {e}"))?;
        let deadline = std::time::Instant::now() + self.timeout;
        loop {
            let line = self.response(deadline).map_err(|(_, message)| message)?;
            let Some(rest) = line.trim_end().strip_prefix("FRM ") else { continue };
            let (rid, payload) = rest.split_once(' ').unwrap_or((rest, "{}"));
            if rid != id.to_string() {
                continue;
            }
            let f = crate::json_flat(payload).unwrap_or_default();
            let get = |k: &str| f.iter().find(|(a, _)| a == k).map(|(_, v)| v.clone()).unwrap_or_default();
            return Ok((get("status").parse().unwrap_or(500), get("location"), get("state"), get("body"), unflat(&get("headers"))));
        }
    }

    /// Calls `fn(ctx, ...args)`; the error carries 422 + `{error, errors}` JSON when the function threw `{errors}`,
    /// 504 on timeout, 500 + the stack otherwise.
    pub fn invoke(&mut self, file: &str, func: &str, ctx_json: &str, args_json: Option<&str>) -> Result<String, (u16, String)> {
        self.next += 1;
        let id = self.next;
        let args = args_json.map_or(String::new(), |a| format!(",\"args\":{a}"));
        let req = format!("{{\"id\":{id},\"file\":{},\"fn\":{},\"ctx\":{ctx_json}{args}}}\n", crate::json_str(file), crate::json_str(func));
        self.stdin.write_all(req.as_bytes()).and_then(|_| self.stdin.flush()).map_err(|e| (500, format!("node sidecar: {e}")))?;
        let deadline = std::time::Instant::now() + self.timeout;
        loop {
            let line = self.response(deadline)?;
            let rest = line.trim_end();
            let (tag, rest) = rest.split_once(' ').unwrap_or((rest, ""));
            let (rid, payload) = rest.split_once(' ').unwrap_or((rest, "null"));
            if rid != id.to_string() {
                continue;
            }
            return match tag {
                "OK" => Ok(payload.to_string()),
                "BAD" => Err((422, payload.to_string())),
                _ => Err((500, unquote(payload))),
            };
        }
    }
}

/// `name\u{1}value\u{2}…` (the sidecar's response headers) → pairs.
fn unflat(s: &str) -> Vec<(String, String)> {
    s.split('\u{2}').filter_map(|h| h.split_once('\u{1}')).map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

impl Drop for Sidecar {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// JSON string → text (only the escapes JSON.stringify produces).
pub(crate) fn unquote(s: &str) -> String {
    let inner = s.strip_prefix('"').and_then(|s| s.strip_suffix('"')).unwrap_or(s);
    let mut o = String::new();
    let mut it = inner.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            o.push(c);
            continue;
        }
        match it.next() {
            Some('n') => o.push('\n'),
            Some('t') => o.push('\t'),
            Some('r') => o.push('\r'),
            Some('u') => {
                let h: String = it.by_ref().take(4).collect();
                if let Some(ch) = u32::from_str_radix(&h, 16).ok().and_then(char::from_u32) {
                    o.push(ch);
                }
            }
            Some(x) => o.push(x),
            None => {}
        }
    }
    o
}

/// `file:///abs/path?v=<mtime>`: Node caches by URL, so a change forces the module to reload.
pub fn file_url(p: &Path) -> String {
    let v = std::fs::metadata(p).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_nanos());
    let s = p.to_string_lossy().replace('\\', "/");
    let s = if s.starts_with('/') { s } else { format!("/{s}") };
    format!("file://{}?v={v}", s.replace(' ', "%20").replace('[', "%5B").replace(']', "%5D"))
}

/// Path where the server code of `rel` is written (e.g. `views/users/[id].brain` → `.brain/server/views/users/[id].brain.mjs`).
pub fn server_path(root: &Path, rel: &str) -> PathBuf {
    root.join(".brain/server").join(format!("{rel}.mjs"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrites_only_relative_specifiers() {
        let src = "import api from \"../lib/api.js\";\nimport fs from 'node:fs';\nconst m = await import('./x.jsbrain');\nexport async function load(ctx) { return { a: 'from ../no' } }";
        let mut seen = vec![];
        let out = brain_pack::rewrite_specifiers(src, Path::new("/app/views"), &mut |p| {
            seen.push(p.to_path_buf());
            Ok(format!("file://{}", p.display()))
        })
        .unwrap();
        assert!(out.contains("import api from \"file:///app/views/../lib/api.js\""));
        assert!(out.contains("'node:fs'") && out.contains("'from ../no'"));
        assert!(out.contains("import(\"file:///app/views/./x.jsbrain\")"));
        assert_eq!(seen.len(), 2);
    }

    #[test]
    fn sidecar_runs_load_with_ctx() {
        if Command::new("node").arg("--version").output().is_err() {
            return; // without node there is nothing to test
        }
        let d = std::env::temp_dir().join(format!("brain-load-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let f = d.join("v.mjs");
        std::fs::write(&f, "export async function load(ctx) { console.log('noise'); return { id: ctx.params.id, hasEnv: !!ctx.env.PATH } }\nexport async function boom() { throw new Error('bad') }").unwrap();
        let mut sc = Sidecar::spawn(&d, &[]).unwrap();
        let ok = sc.call(&file_url(&f), "load", "{\"params\":{\"id\":\"42\"}}").unwrap();
        assert_eq!(ok, "{\"id\":\"42\",\"hasEnv\":true}");
        let err = sc.call(&file_url(&f), "boom", "{}").unwrap_err();
        assert!(err.contains("Error: bad"), "{err}");
        std::fs::remove_dir_all(&d).unwrap();
    }
    #[test]
    fn hung_load_is_killed_and_reaped_at_its_deadline() {
        let d = std::env::temp_dir().join(format!("brain-hung-load-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let f = d.join("hung.mjs");
        std::fs::write(&f, "export function load() { while (true) {} }").unwrap();
        let mut sc = Sidecar::spawn(&d, &[]).unwrap();
        sc.timeout = std::time::Duration::from_millis(250);
        let start = std::time::Instant::now();
        let (status, _) = sc.invoke(&file_url(&f), "load", "{}", None).unwrap_err();
        assert_eq!(status, 504);
        assert!(start.elapsed() < std::time::Duration::from_secs(3));
        assert!(sc.child.try_wait().unwrap().is_some(), "timeout must reap Node");
        drop(sc);
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn response_noise_does_not_renew_the_deadline() {
        let d = std::env::temp_dir().join(format!("brain-noisy-load-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let f = d.join("noise.mjs");
        std::fs::write(&f, "export async function load() { setInterval(() => process.stdout.write('OK 0 null\\n'), 5); return new Promise(() => {}) }").unwrap();
        let mut sc = Sidecar::spawn(&d, &[]).unwrap();
        sc.timeout = std::time::Duration::from_millis(250);
        let start = std::time::Instant::now();
        assert_eq!(sc.invoke(&file_url(&f), "load", "{}", None).unwrap_err().0, 504);
        assert!(start.elapsed() < std::time::Duration::from_secs(3));
        drop(sc);
        std::fs::remove_dir_all(d).unwrap();
    }

}
