//! `load(ctx)` in dev (INTERNALS §11.1): each view's server code (`Output.server_js`) is written to
//! `<app>/.brain/server/` with its imports resolved to absolute paths, and a persistent Node process
//! (sidecar) runs it. Line protocol: Rust → `{"id","file","fn","ctx","args"?}` · Node → `OK <id> <json>` |
//! `ERR <id> <json>` | `BAD <id> {"error","errors"}` (the function threw `{errors}`: a 422 for RPC, §11.1).

use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

const RUNNER: &str = r#"
import { createInterface } from 'node:readline'
import { EventEmitter } from 'node:events'
import { Worker } from 'node:worker_threads'
// A separate watchdog thread still runs if application code blocks Node's main event loop.
const owner = Number(process.env.BRAIN_SIDECAR_PARENT)
if (Number.isInteger(owner) && owner > 1) {
  const watcher = new Worker(`const {workerData}=require('node:worker_threads');setInterval(()=>{try{process.kill(workerData.owner,0)}catch(error){if(error.code==='ESRCH'){try{process.kill(workerData.group?-process.pid:process.pid,'SIGKILL')}catch{process.kill(process.pid,'SIGKILL')}}}},500)`, {eval:true,execArgv:[],resourceLimits:{maxOldGenerationSizeMb:8,maxYoungGenerationSizeMb:2,stackSizeMb:1},workerData:{owner,group:process.env.BRAIN_SIDECAR_GROUP==='1'}})
  watcher.unref()
}
// stdout is the protocol channel: the user's console.log calls go to stderr (brain run terminal).
const out = process.stdout.write.bind(process.stdout)
console.log = console.info = (...a) => console.error(...a)
// response headers as "name\u0001value\u0002…" (a header may repeat: set-cookie)
const flat = h => Object.entries(h || {}).flatMap(([k, v]) => (Array.isArray(v) ? v : [v]).map(x => k + '\u0001' + x)).join('\u0002')
// the request's session from its signed cookie (server.mjs sessionFrom), so login/logout set cookies in dev too
const nativeContexts = new WeakMap()
let nativeBridge = null, nativeModule = null, nativeURL = null
const withSession = async (S, ctx, secret, preauthenticated = false, nativeRequired = false) => {
  const c = { ...ctx, url: new URL(ctx.url, 'http://localhost'), env: process.env }
  const configured = process.env.BRAIN_BACKEND_URL
  if (nativeRequired && !configured) throw Object.assign(Error('Native authentication is temporarily unavailable'), { status: 503 })
  if (configured) {
    if (typeof S.nativeAuthBridge !== 'function') throw Object.assign(Error('Native authentication bridge is unavailable in this runtime version'), { status: 503 })
    if (!nativeBridge || nativeModule !== S || nativeURL !== configured) { nativeBridge?.close(); try { nativeBridge = S.nativeAuthBridge(configured) } catch { throw Object.assign(Error('Native authentication is temporarily unavailable'), { status: 503 }) }; nativeModule = S; nativeURL = configured }
    const req = new EventEmitter(), res = new EventEmitter(), headers = Object.create(null)
    for (const name of ['host', 'cookie', 'origin', 'accept', 'content-type']) { const value = ctx.req?.headers?.[name]; if (typeof value === 'string') headers[name] = value }
    req.headers = headers; req.method = ctx.req?.method || 'GET'; req.url = ctx.url
    res.headersSent = false; res.writableEnded = false
    const responseHeaders = new Map()
    res.getHeader = name => responseHeaders.get(String(name).toLowerCase())
    res.setHeader = (name, value) => responseHeaders.set(String(name).toLowerCase(), value)
    const rid = typeof ctx.rid === 'string' && /^[a-f0-9-]{36}$/i.test(ctx.rid) ? ctx.rid : '00000000-0000-4000-8000-000000000000'
    try {
      if (preauthenticated) {
        const user = ctx.user ?? null
        if (user !== null && (typeof user.id !== 'string' || !/^[a-f0-9-]{36}$/i.test(user.id) || typeof user.email !== 'string' || user.email.length > 254 || typeof user.role !== 'string' || !/^[a-z][a-z0-9_]{0,47}$/.test(user.role) || ['public','owner'].includes(user.role) || typeof user.verified !== 'boolean')) throw Object.assign(Error('Native authentication context is invalid'), {status:503})
        req.$nativeUser = user === null ? null : Object.freeze({ id:user.id, email:user.email, role:user.role, verified:user.verified })
        req.$nativeAuth = Promise.resolve()
      }
      else await nativeBridge.prepare(req, res, rid)
      const session = nativeBridge.session(req, res, rid)
      c.user = session.user; c.session = session
      c.backend = Object.freeze({resource: name => nativeBridge.resource(req, res, rid, name),login:credentials=>nativeBridge.login(req,res,rid,credentials)})
      nativeContexts.set(c, { req, res })
      return c
    } catch (error) { res.writableEnded = true; res.emit('finish'); res.emit('close'); throw error }
  }
  const ses = S.sessionFrom(ctx.cookies || {}, secret || '')
  c.user = ses.user; c.session = ses.session
  return c
}
const contextHeaders = async ctx => {
  const native = nativeContexts.get(ctx)
  if (!native) return ctx.session.headers()
  const { req } = native
  if (req.$nativeLogout) await req.$nativeLogout
  if (req.$nativeLogoutError) throw Object.assign(Error('Native authentication is temporarily unavailable'), { status: 503 })
  return req.$nativeLogoutHeaders || req.$nativeLoginHeaders || {}
}
const finishContext = ctx => { const native = nativeContexts.get(ctx); if (native) { native.res.writableEnded = true; native.res.emit('finish'); native.res.emit('close'); nativeContexts.delete(ctx) } }
process.once('exit', () => nativeBridge?.close())
for await (const line of createInterface({ input: process.stdin })) {
  const { id, file, fn, ctx, args, probe, rpc, raw, key, form, rules, chain, guards, middleware, secret, contextServer, preauthenticated, nativeRequired } = JSON.parse(line)
  if (chain) {
    // _guard/_middleware of the route through server.mjs's runChain, with ctx.user from the signed session cookie
    try {
      const S = await import(chain)
      const G = [], M = []
      for (const u of guards || []) G.push(await import(u))
      for (const u of middleware || []) M.push(await import(u))
      const c = await withSession(S, ctx, secret, false, nativeRequired)
      try {
        const r = await S.runChain({ guards: G, middleware: M, ctx: c })
        await contextHeaders(c)
        const kind = r.redirect ? 'redirect' : r.deny !== undefined ? 'deny' : 'ok'
        out(`CHN ${id} ${JSON.stringify({ kind, location: r.redirect || '', status: String(r.status || 303), reason: String(r.deny ?? ''), user: JSON.stringify(c.user ?? null) })}\n`)
      } finally { finishContext(c) }
    } catch (e) {
      console.error(String(e?.stack ?? e))
      out(`CHN ${id} ${JSON.stringify({ kind: 'error', status: String(e?.status || 500), reason: String(e?.message ?? e) })}\n`)
    }
    continue
  }
  if (form) {
    // a form posted without JS (§38) through server.mjs's callForm, with the same rules as form() in the browser
    try {
      const S = await import(form), R = await import(rules), m = await import(file)
      const c = await withSession(S, ctx, secret, false, nativeRequired)
      try {
        const r = await S.callForm({ module: m, raw, ctx: c, rules: R, key })
        if (r.cause) console.error(String(r.cause?.stack ?? r.cause))
        out(`FRM ${id} ${JSON.stringify({ status: r.status, location: r.location || '', state: JSON.stringify(r.state || null), body: JSON.stringify(r.body || null), headers: flat({ ...(r.headers || {}), ...await contextHeaders(c) }) })}\n`)
      } finally { finishContext(c) }
    } catch (e) {
      console.error(String(e?.stack ?? e))
      out(`FRM ${id} ${JSON.stringify({ status: Number.isInteger(e?.status) ? e.status : 500, location: '', state: 'null', body: '{"error":"Request could not complete"}' })}\n`)
    }
    continue
  }
  if (rpc) {
    // RPC through the production handler (runtime/server/server.mjs callRpc): same parsing, limits and errors
    try {
      const S = await import(rpc)
      const m = await import(file)
      const c = await withSession(S, ctx, secret, false, nativeRequired)
      try {
        const r = await S.callRpc({ fn: m[fn], args: raw, ctx: c, key })
        if (r.cause) console.error(String(r.cause?.stack ?? r.cause))
        out(`RES ${id} ${JSON.stringify({ status: r.status, body: JSON.stringify(r.body ?? null), headers: flat({ ...(r.headers || {}), ...await contextHeaders(c) }) })}\n`)
      } finally { finishContext(c) }
    } catch (e) {
      console.error(String(e?.stack ?? e))
      out(`RES ${id} ${JSON.stringify({ status: Number.isInteger(e?.status) ? e.status : 500, body: '{"error":"Request could not complete"}' })}\n`)
    }
    continue
  }
  try {
    const m = await import(file)
    if (typeof m[fn] !== 'function') throw new Error(`${fn} is not exported by the view's server code`)
    if (probe) { out(`OK ${id} true\n`); continue }
    const S = contextServer ? await import(contextServer) : null
    const c = S ? await withSession(S, ctx, secret, preauthenticated === true, nativeRequired === true) : { ...ctx, env: process.env }
    try {
      const v = await m[fn](c, ...(args || []))
      if (S) await contextHeaders(c)
      out(`OK ${id} ${JSON.stringify(v ?? null)}\n`)
    } finally { if (S) finishContext(c) }
  } catch (e) {
    if (e && e.notFound === true) out(`OK ${id} {"notFound":true,"status":404}\n`) // `throw notFound()`: answered as a 404 page
    else if (e && e.errors && typeof e.errors === 'object') out(`BAD ${id} ${JSON.stringify({ error: e.message || 'Invalid input', errors: e.errors })}\n`)
    else if (contextServer && e?.status === 503) out(`STATUS ${id} ${JSON.stringify({status:503,message:'Native authentication is temporarily unavailable'})}\n`)
    else out(`ERR ${id} ${JSON.stringify(String(e?.stack ?? e))}\n`)
  }
}
"#;

pub struct Sidecar {
    child: Child,
    terminated: bool,
    input:
        Option<std::sync::mpsc::SyncSender<(String, std::sync::mpsc::SyncSender<io::Result<()>>)>>,
    write_thread: Option<std::thread::JoinHandle<()>>,
    read_thread: Option<std::thread::JoinHandle<()>>,
    frame_limit: usize,
    stop_reader: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// Lines from Node's stdout, read on a thread so a hung `load()` can time out instead of blocking forever.
    lines: std::sync::mpsc::Receiver<String>,
    next: u64,
    timeout: std::time::Duration,
    native_required: bool,
}

/// A `load()` that does not answer in this time is killed (INTERNALS §25: every command terminates).
const MAX_RESPONSE: u64 = 16 * 1024 * 1024;

const LOAD_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

impl Sidecar {
    /// `vars`: the resolved environment (`.brain_env` development section over the process), seen as `ctx.env`.
    pub fn spawn(root: &Path, vars: &[(String, String)]) -> io::Result<Sidecar> {
        Self::spawn_with(
            root,
            vars,
            LOAD_TIMEOUT,
            MAX_RESPONSE as usize,
            false,
            false,
        )
    }
    /// Private hooks: strict end-to-end I/O deadline, production mode and silent untrusted stderr.
    pub fn spawn_bounded(
        root: &Path,
        vars: &[(String, String)],
        timeout: std::time::Duration,
        frame_limit: usize,
        production: bool,
    ) -> io::Result<Sidecar> {
        if !cfg!(unix) {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "Native before-hooks require Unix process-group supervision in this release; ordinary frontend/dev sidecars remain available",
            ));
        }
        if timeout < std::time::Duration::from_millis(50)
            || timeout > std::time::Duration::from_secs(2)
            || !(1024..=65536).contains(&frame_limit)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Invalid bounded sidecar limits",
            ));
        }
        Self::spawn_with(root, vars, timeout, frame_limit, production, true)
    }
    /// Like [`Sidecar::spawn_bounded`] for application routes: up to 30 s per call (outbound
    /// requests to providers) and 1 MiB frames, with the same private process-group supervision.
    pub fn spawn_route(
        root: &Path,
        vars: &[(String, String)],
        timeout: std::time::Duration,
        frame_limit: usize,
        production: bool,
    ) -> io::Result<Sidecar> {
        if !cfg!(unix) {
            return Err(io::Error::new(io::ErrorKind::Unsupported, "Native routes require Unix process-group supervision in this release"));
        }
        if timeout < std::time::Duration::from_millis(50)
            || timeout > std::time::Duration::from_secs(30)
            || !(1024..=1024 * 1024).contains(&frame_limit)
        {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "Invalid bounded sidecar limits"));
        }
        Self::spawn_with(root, vars, timeout, frame_limit, production, true)
    }
    fn spawn_with(
        root: &Path,
        vars: &[(String, String)],
        timeout: std::time::Duration,
        frame_limit: usize,
        production: bool,
        private: bool,
    ) -> io::Result<Sidecar> {
        let mut command =
            Command::new(std::env::var("BRAIN_NODE").unwrap_or_else(|_| "node".into()));
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        let mut child=command.args(if private {&["--max-old-space-size=128"][..]}else{&[]})
            .args(["--enable-source-maps", "--input-type=module", "-e", RUNNER])
            .envs(vars.iter().map(|(k, v)| (k.as_str(), v.as_str())))
            .env("NODE_ENV", if production {"production"}else{"development"})
            .env("BRAIN_SIDECAR_PARENT",std::process::id().to_string())
            .env("BRAIN_SIDECAR_GROUP",if cfg!(unix){"1"}else{"0"})
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(if private {Stdio::null()}else{Stdio::inherit()})
            .spawn()
            .map_err(|e| io::Error::new(e.kind(), format!("could not launch node for load(): {e}; fix: install Node ≥ 22 or set BRAIN_NODE")))?;
        let mut stdin = child.stdin.take().unwrap();
        let (input, requests) = std::sync::mpsc::sync_channel::<(
            String,
            std::sync::mpsc::SyncSender<io::Result<()>>,
        )>(1);
        let write_thread = std::thread::spawn(move || {
            while let Ok((request, ack)) = requests.recv() {
                let result = stdin
                    .write_all(request.as_bytes())
                    .and_then(|_| stdin.flush());
                let failed = result.is_err();
                let _ = ack.send(result);
                if failed {
                    break;
                }
            }
        });
        let mut stdout = BufReader::new(child.stdout.take().unwrap());
        let (tx, lines) = std::sync::mpsc::sync_channel(2);
        let stop_reader = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stopped = stop_reader.clone();
        let read_thread = std::thread::spawn(move || {
            let mut line = String::new();
            // Cap a frame before allocating an arbitrary line from user code. A full queue backpressures Node.
            while (&mut stdout)
                .take(frame_limit as u64 + 1)
                .read_line(&mut line)
                .is_ok_and(|n| n > 0)
            {
                if line.len() > frame_limit {
                    break;
                }
                let mut pending = std::mem::take(&mut line);
                loop {
                    match tx.try_send(pending) {
                        Ok(()) => break,
                        Err(std::sync::mpsc::TrySendError::Disconnected(_)) => return,
                        Err(std::sync::mpsc::TrySendError::Full(value)) => {
                            if stopped.load(std::sync::atomic::Ordering::Relaxed) {
                                return;
                            }
                            pending = value;
                            std::thread::sleep(std::time::Duration::from_millis(1));
                        }
                    }
                }
            }
        });
        Ok(Sidecar {
            child,
            terminated: false,
            input: Some(input),
            write_thread: Some(write_thread),
            read_thread: Some(read_thread),
            frame_limit,
            stop_reader,
            lines,
            next: 0,
            timeout,
            native_required: vars
                .iter()
                .find(|(key, _)| key == "BRAIN_BACKEND_URL")
                .map(|(_, value)| !value.is_empty())
                .unwrap_or_else(|| {
                    std::env::var("BRAIN_BACKEND_URL").is_ok_and(|value| !value.is_empty())
                }),
        })
    }

    fn terminate(&mut self) {
        if !self.terminated {
            self.terminated = true;
            // The unreaped child pins this process-group identifier: never target a reused PID.
            #[cfg(unix)]
            unsafe {
                libc::kill(-(self.child.id() as i32), libc::SIGKILL);
            }
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
    fn send(&mut self, request: String, deadline: std::time::Instant) -> Result<(), (u16, String)> {
        let (ack, done) = std::sync::mpsc::sync_channel(1);
        let outcome = if request.len() > self.frame_limit {
            false
        } else {
            self.input
                .as_ref()
                .is_some_and(|input| input.try_send((request, ack)).is_ok())
                && done
                    .recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
                    .is_ok_and(|result| result.is_ok())
        };
        if !outcome {
            self.terminate();
            return Err((
                503,
                "Node sidecar input exceeded its bounded deadline or frame budget".into(),
            ));
        }
        Ok(())
    }
    fn response(&mut self, deadline: std::time::Instant) -> Result<String, (u16, String)> {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        match self.lines.recv_timeout(remaining) {
            Ok(line) if !remaining.is_zero() => Ok(line),
            outcome => {
                let timed_out = remaining.is_zero()
                    || matches!(outcome, Err(std::sync::mpsc::RecvTimeoutError::Timeout));
                self.terminate();
                Err((
                    if timed_out { 504 } else { 500 },
                    if timed_out {
                        "the load() node process exited: request timed out; fix: use bounded awaits and no infinite loops".into()
                    } else {
                        "the load() node process exited or exceeded its configured response limit; inspect private sidecar diagnostics".into()
                    },
                ))
            }
        }
    }

    /// Calls `fn(ctx)` from module `file` (URL `file://…?v=…`). `ctx_json` is the already-built JSON object.
    /// Returns the result JSON or the error (stack) as text.
    pub fn call(&mut self, file: &str, func: &str, ctx_json: &str) -> Result<String, String> {
        self.invoke(file, func, ctx_json, None).map_err(|(_, e)| e)
    }

    /// `POST /_r/<hash>/<fn>` through server.mjs's `callRpc` (`server` = its file URL): returns (status, JSON body).
    pub fn rpc(
        &mut self,
        file: &str,
        func: &str,
        ctx_json: &str,
        raw: &str,
        server: &str,
        key: &str,
        secret: &str,
    ) -> Result<(u16, String, Vec<(String, String)>), String> {
        self.next += 1;
        let id = self.next;
        let req = format!(
            "{{\"id\":{id},\"file\":{},\"fn\":{},\"ctx\":{ctx_json},\"rpc\":{},\"raw\":{},\"key\":{},\"secret\":{},\"nativeRequired\":{}}}\n",
            crate::json_str(file),
            crate::json_str(func),
            crate::json_str(server),
            crate::json_str(raw),
            crate::json_str(key),
            crate::json_str(secret),
            self.native_required
        );
        let deadline = std::time::Instant::now() + self.timeout;
        self.send(req, deadline).map_err(|(_, message)| message)?;
        loop {
            let line = self.response(deadline).map_err(|(_, message)| message)?;
            let rest = line.trim_end();
            let Some(rest) = rest.strip_prefix("RES ") else {
                continue;
            };
            let (rid, payload) = rest.split_once(' ').unwrap_or((rest, "{}"));
            if rid != id.to_string() {
                continue;
            }
            let fields = crate::json_flat(payload).unwrap_or_default();
            let status = fields
                .iter()
                .find(|(k, _)| k == "status")
                .and_then(|(_, v)| v.parse().ok())
                .unwrap_or(500);
            let headers = unflat(
                fields
                    .iter()
                    .find(|(k, _)| k == "headers")
                    .map(|(_, v)| v.as_str())
                    .unwrap_or(""),
            );
            let body = fields
                .into_iter()
                .find(|(k, _)| k == "body")
                .map(|(_, v)| v)
                .unwrap_or_else(|| "null".into());
            return Ok((status, body, headers));
        }
    }

    /// Runs a route's guards and middleware (server.mjs `runChain`): fields kind (ok|redirect|deny|error), location,
    /// status, reason, user (JSON).
    pub fn chain(
        &mut self,
        server: &str,
        guards: &[String],
        middleware: &[String],
        ctx_json: &str,
        secret: &str,
    ) -> Result<Vec<(String, String)>, String> {
        self.next += 1;
        let id = self.next;
        let list = |v: &[String]| {
            format!(
                "[{}]",
                v.iter()
                    .map(|u| crate::json_str(u))
                    .collect::<Vec<_>>()
                    .join(",")
            )
        };
        let req = format!(
            "{{\"id\":{id},\"ctx\":{ctx_json},\"chain\":{},\"guards\":{},\"middleware\":{},\"secret\":{},\"nativeRequired\":{}}}\n",
            crate::json_str(server),
            list(guards),
            list(middleware),
            crate::json_str(secret),
            self.native_required
        );
        let deadline = std::time::Instant::now() + self.timeout;
        self.send(req, deadline).map_err(|(_, message)| message)?;
        loop {
            let line = self.response(deadline).map_err(|(_, message)| message)?;
            let Some(rest) = line.trim_end().strip_prefix("CHN ") else {
                continue;
            };
            let (rid, payload) = rest.split_once(' ').unwrap_or((rest, "{}"));
            if rid != id.to_string() {
                continue;
            }
            return Ok(crate::json_flat(payload).unwrap_or_default());
        }
    }

    /// A form-encoded POST through server.mjs's `callForm`: (status, location, state JSON, body JSON).
    pub fn form(
        &mut self,
        file: &str,
        ctx_json: &str,
        raw: &str,
        server: &str,
        rules: &str,
        key: &str,
        secret: &str,
    ) -> Result<(u16, String, String, String, Vec<(String, String)>), String> {
        self.next += 1;
        let id = self.next;
        let req = format!(
            "{{\"id\":{id},\"file\":{},\"ctx\":{ctx_json},\"form\":{},\"rules\":{},\"raw\":{},\"key\":{},\"secret\":{},\"nativeRequired\":{}}}\n",
            crate::json_str(file),
            crate::json_str(server),
            crate::json_str(rules),
            crate::json_str(raw),
            crate::json_str(key),
            crate::json_str(secret),
            self.native_required
        );
        let deadline = std::time::Instant::now() + self.timeout;
        self.send(req, deadline).map_err(|(_, message)| message)?;
        loop {
            let line = self.response(deadline).map_err(|(_, message)| message)?;
            let Some(rest) = line.trim_end().strip_prefix("FRM ") else {
                continue;
            };
            let (rid, payload) = rest.split_once(' ').unwrap_or((rest, "{}"));
            if rid != id.to_string() {
                continue;
            }
            let f = crate::json_flat(payload).unwrap_or_default();
            let get = |k: &str| {
                f.iter()
                    .find(|(a, _)| a == k)
                    .map(|(_, v)| v.clone())
                    .unwrap_or_default()
            };
            return Ok((
                get("status").parse().unwrap_or(500),
                get("location"),
                get("state"),
                get("body"),
                unflat(&get("headers")),
            ));
        }
    }

    /// Calls `fn(ctx, ...args)`; the error carries 422 + `{error, errors}` JSON when the function threw `{errors}`,
    /// 504 on timeout, 500 + the stack otherwise.
    pub fn call_context(
        &mut self,
        file: &str,
        func: &str,
        ctx_json: &str,
        server: &str,
        secret: &str,
        preauthenticated: bool,
    ) -> Result<String, String> {
        self.invoke_context(
            file,
            func,
            ctx_json,
            None,
            Some((server, secret, preauthenticated)),
            false,
        )
        .map_err(|(status, message)| {
            if status == 503 {
                format!("BNATIVE_AUTH_UNAVAILABLE: {message}")
            } else {
                message
            }
        })
    }
    pub fn invoke(
        &mut self,
        file: &str,
        func: &str,
        ctx_json: &str,
        args_json: Option<&str>,
    ) -> Result<String, (u16, String)> {
        self.invoke_context(file, func, ctx_json, args_json, None, false)
    }
    /// Imports a private module and verifies its callable export without running the hook body.
    pub fn probe(&mut self, file: &str, func: &str) -> Result<(), (u16, String)> {
        self.invoke_context(file, func, "{}", None, None, true)
            .map(|_| ())
    }
    fn invoke_context(
        &mut self,
        file: &str,
        func: &str,
        ctx_json: &str,
        args_json: Option<&str>,
        context: Option<(&str, &str, bool)>,
        probe: bool,
    ) -> Result<String, (u16, String)> {
        self.next += 1;
        let id = self.next;
        let args = args_json.map_or(String::new(), |a| format!(",\"args\":{a}"))
            + if probe { ",\"probe\":true" } else { "" };
        let context=context.map(|(server,secret,verified)|format!(",\"contextServer\":{},\"secret\":{},\"preauthenticated\":{verified},\"nativeRequired\":{}",crate::json_str(server),crate::json_str(secret),self.native_required)).unwrap_or_default();
        let req = format!(
            "{{\"id\":{id},\"file\":{},\"fn\":{},\"ctx\":{ctx_json}{args}{context}}}\n",
            crate::json_str(file),
            crate::json_str(func)
        );
        let deadline = std::time::Instant::now() + self.timeout;
        self.send(req, deadline)?;
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
                "STATUS" => {
                    let fields = crate::json_flat(payload).unwrap_or_default();
                    let status = fields
                        .iter()
                        .find(|(key, _)| key == "status")
                        .and_then(|(_, value)| value.parse::<u16>().ok())
                        .filter(|status| (400..=599).contains(status))
                        .unwrap_or(500);
                    let message = fields
                        .into_iter()
                        .find(|(key, _)| key == "message")
                        .map(|(_, value)| value)
                        .unwrap_or_else(|| "Native request failed".into());
                    Err((status, message))
                }
                _ => Err((500, unquote(payload))),
            };
        }
    }
}

/// `name\u{1}value\u{2}…` (the sidecar's response headers) → pairs.
fn unflat(s: &str) -> Vec<(String, String)> {
    s.split('\u{2}')
        .filter_map(|h| h.split_once('\u{1}'))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

impl Drop for Sidecar {
    fn drop(&mut self) {
        self.stop_reader
            .store(true, std::sync::atomic::Ordering::Relaxed);
        self.terminate();
        self.input.take();
        #[cfg(unix)]
        {
            if let Some(thread) = self.write_thread.take() {
                let _ = thread.join();
            }
            if let Some(thread) = self.read_thread.take() {
                let _ = thread.join();
            }
        }
        // Windows retains the ordinary dev helper's detached reader behavior. Native bounded hooks
        // fail during provisioning there until safe process-tree supervision is available.
        #[cfg(not(unix))]
        {
            self.write_thread.take();
            self.read_thread.take();
        }
    }
}

/// JSON string → text (only the escapes JSON.stringify produces).
pub(crate) fn unquote(s: &str) -> String {
    let inner = s
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(s);
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
    let v = std::fs::metadata(p)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos());
    // Keep native path semantics: in particular a Unix backslash is a filename byte, while
    // Windows drive/verbatim/UNC prefixes are interpreted by the platform-aware URL library.
    let native = url::Url::from_file_path(p).or_else(|_| {
        std::path::absolute(p)
            .ok()
            .and_then(|absolute| url::Url::from_file_path(absolute).ok())
            .ok_or(())
    });
    match native {
        Ok(url) => cache_file_url(url, v),
        // Preserve the String API but fail closed for unsupported native prefixes / unavailable cwd.
        // NUL cannot name a filesystem entry, so this never falls back to an unrelated file or host.
        Err(()) => format!("file:///%00?v={v}"),
    }
}

fn cache_file_url(mut native: url::Url, revision: u128) -> String {
    native.set_query(Some(&format!("v={revision}")));
    // Node pathToFileURL escapes brackets in paths. Do not touch brackets in an IPv6 UNC authority.
    let prefix = &native[..url::Position::BeforePath];
    let path = &native[url::Position::BeforePath..url::Position::AfterPath];
    let suffix = &native[url::Position::AfterPath..];
    format!(
        "{prefix}{}{suffix}",
        path.replace('[', "%5B")
            .replace(']', "%5D")
            .replace('^', "%5E")
            .replace('|', "%7C")
    )
}

/// Path where the server code of `rel` is written (e.g. `views/users/[id].brain` → `.brain/server/views/users/[id].brain.mjs`).
pub fn server_path(root: &Path, rel: &str) -> PathBuf {
    root.join(".brain/server").join(format!("{rel}.mjs"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_url_windows_serializations_match_node_and_native_windows_paths() {
        let cases = [
            (
                concat!(r"C:\apps\name #?%[", "\u{e9}", "].mjs"),
                "file:///C:/apps/name%20%23%3F%25%5B%C3%A9%5D.mjs",
            ),
            (
                concat!(r"\\?\C:\apps\name #?%[", "\u{e9}", "].mjs"),
                "file:///C:/apps/name%20%23%3F%25%5B%C3%A9%5D.mjs",
            ),
            (
                r"\\server\share\name #%.mjs",
                "file://server/share/name%20%23%25.mjs",
            ),
            (
                r"\\?\UNC\server\share\name [x].mjs",
                "file://server/share/name%20%5Bx%5D.mjs",
            ),
            ("C:/apps/file.mjs", "file:///C:/apps/file.mjs"),
        ];
        for (path, expected) in cases {
            assert_eq!(
                cache_file_url(url::Url::parse(expected).unwrap(), 17),
                format!("{expected}?v=17")
            );
            #[cfg(windows)]
            {
                // Pure native conversion: synthetic UNC hosts must never trigger filesystem/network I/O.
                let native = cache_file_url(url::Url::from_file_path(Path::new(path)).unwrap(), 17);
                assert_eq!(native, format!("{expected}?v=17"), "{path}");
            }
            #[cfg(not(windows))]
            let _ = path;
        }
        let rows = brain_lex::Json::Arr(
            cases
                .iter()
                .map(|(path, expected)| {
                    brain_lex::Json::Arr(vec![
                        brain_lex::Json::str(*path),
                        brain_lex::Json::str(*expected),
                    ])
                })
                .collect(),
        )
        .stringify();
        let code = "import assert from 'node:assert/strict';import{pathToFileURL}from'node:url';for(const[path,expected]of JSON.parse(process.argv[1])){const u=pathToFileURL(path,{windows:true});assert.equal(u.href,expected);assert.equal(u.hash,'');assert.equal(u.search,'')}console.log('WINDOWS_FILE_URL_ORACLE_OK')";
        let output = Command::new("node")
            .args(["--input-type=module", "-e", code, &rows])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("WINDOWS_FILE_URL_ORACLE_OK"));
    }

    #[test]
    fn file_url_imports_reserved_unicode_paths_and_busts_node_module_cache() {
        struct Root(PathBuf);
        impl Drop for Root {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = Root(
            std::env::temp_dir().join(format!("brain-file-url-{}-{stamp}", std::process::id())),
        );
        std::fs::create_dir_all(&root.0).unwrap();
        let name = if cfg!(unix) {
            "module ?#%[\u{e9}] ^|`{}.mjs"
        } else {
            "module #%[\u{e9}].mjs"
        };
        let file = root.0.join(name);
        std::fs::write(&file, "export const value=1;\n").unwrap();
        let set_time = |seconds| {
            std::fs::File::options()
                .write(true)
                .open(&file)
                .unwrap()
                .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(seconds))
                .unwrap()
        };
        set_time(1000000000);
        let canonical = file.canonicalize().unwrap();
        let first = file_url(&canonical);
        assert!(first.ends_with("?v=1000000000000000000"));
        assert!(first.contains("%23%25%5B%C3%A9%5D"));
        let parsed = url::Url::parse(&first).unwrap();
        assert_eq!(
            parsed.to_file_path().unwrap().canonicalize().unwrap(),
            canonical
        );
        assert!(parsed.fragment().is_none());
        set_time(1000000002);
        let second = file_url(&canonical);
        assert!(second.ends_with("?v=1000000002000000000"));
        assert_ne!(first, second);
        set_time(1000000000);
        let first = brain_lex::Json::str(first).stringify();
        let second = brain_lex::Json::str(second).stringify();
        let code = format!(
            "import assert from 'node:assert/strict';import{{writeFile}}from'node:fs/promises';const a=await import({first});assert.equal(a.value,1);await writeFile(new URL({first}),'export const value=2;');const b=await import({second});assert.equal(b.value,2);assert.equal((await import({first})).value,1);console.log('FILE_URL_IMPORT_OK')"
        );
        let output = Command::new("node")
            .args(["--input-type=module", "-e", &code])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("FILE_URL_IMPORT_OK"));
        #[cfg(unix)]
        {
            // Node ESM rejects encoded backslashes even on Unix. Preserve the literal path and fail
            // instead of importing a different nested file after a Windows-style separator rewrite.
            let literal = root.0.join("back\\slash.mjs");
            let nested = root.0.join("back/slash.mjs");
            std::fs::create_dir_all(nested.parent().unwrap()).unwrap();
            std::fs::write(&literal, "export default 'literal';").unwrap();
            std::fs::write(&nested, "export default 'nested';").unwrap();
            let literal = brain_lex::Json::str(file_url(&literal)).stringify();
            let nested = brain_lex::Json::str(file_url(&nested)).stringify();
            let code = format!(
                "import assert from 'node:assert/strict';await assert.rejects(import({literal}),e=>e.code==='ERR_INVALID_MODULE_SPECIFIER');assert.equal((await import({nested})).default,'nested');console.log('LITERAL_BACKSLASH_NOT_REDIRECTED')"
            );
            let output = Command::new("node")
                .args(["--input-type=module", "-e", &code])
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(
                String::from_utf8_lossy(&output.stdout)
                    .contains("LITERAL_BACKSLASH_NOT_REDIRECTED")
            );
        }
    }

    #[test]
    #[cfg(unix)]
    fn file_url_preserves_unix_backslashes_and_non_utf8_filename_bytes() {
        use std::os::unix::ffi::OsStringExt;
        let path = PathBuf::from(std::ffi::OsString::from_vec(
            b"/tmp/brain-file-url-back\\slash-\xff.mjs".to_vec(),
        ));
        let actual = file_url(&path);
        assert!(actual.contains("back%5Cslash-%FF.mjs"), "{actual}");
        assert_eq!(
            url::Url::parse(&actual).unwrap().to_file_path().unwrap(),
            path
        );
    }

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
        let ok = sc
            .call(&file_url(&f), "load", "{\"params\":{\"id\":\"42\"}}")
            .unwrap();
        assert_eq!(ok, "{\"id\":\"42\",\"hasEnv\":true}");
        let err = sc.call(&file_url(&f), "boom", "{}").unwrap_err();
        assert!(err.contains("Error: bad"), "{err}");
        std::fs::remove_dir_all(&d).unwrap();
    }
    #[test]
    fn sidecar_reports_original_source_lines_from_inline_maps() {
        if Command::new("node").arg("--version").output().is_err() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("brain-mapped-load-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let module = dir.join("mapped.mjs");
        std::fs::write(&module, "export function boom(){throw Error('mapped')}\n//# sourceMappingURL=data:application/json;base64,eyJ2ZXJzaW9uIjozLCJzb3VyY2VzIjpbIm9yaWdpbmFsLmJyYWluIl0sIm5hbWVzIjpbXSwibWFwcGluZ3MiOiJBQVNBIn0=\n").unwrap();
        let mut sidecar = Sidecar::spawn(&dir, &[]).unwrap();
        let error = sidecar.call(&file_url(&module), "boom", "{}").unwrap_err();
        assert!(error.contains("original.brain:10:"), "{error}");
        drop(sidecar);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn sidecar_answers_not_found_from_load() {
        if Command::new("node").arg("--version").output().is_err() {
            return;
        }
        // F11: `throw notFound()` (compiled to this marker) and `return notFound()` both reach app_page as the 404 marker
        let d = std::env::temp_dir().join(format!("brain-nf-load-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let f = d.join("nf.mjs");
        std::fs::write(&f, "export async function load(ctx) { if (ctx.params.id === 'gone') throw ({ notFound: true, status: 404 }); if (ctx.params.id === 'ret') return ({ notFound: true, status: 404 }); return { id: ctx.params.id } }").unwrap();
        let mut sc = Sidecar::spawn(&d, &[]).unwrap();
        for id in ["gone", "ret"] {
            let v = sc
                .call(
                    &file_url(&f),
                    "load",
                    &format!("{{\"params\":{{\"id\":\"{id}\"}}}}"),
                )
                .unwrap();
            assert!(v.starts_with("{\"notFound\":true"), "{id}: {v}");
        }
        assert_eq!(
            sc.call(&file_url(&f), "load", "{\"params\":{\"id\":\"7\"}}")
                .unwrap(),
            "{\"id\":\"7\"}"
        );
        drop(sc);
        std::fs::remove_dir_all(d).unwrap();
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
        assert!(
            sc.child.try_wait().unwrap().is_some(),
            "timeout must reap Node"
        );
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
        assert_eq!(
            sc.invoke(&file_url(&f), "load", "{}", None).unwrap_err().0,
            504
        );
        assert!(start.elapsed() < std::time::Duration::from_secs(3));
        drop(sc);
        std::fs::remove_dir_all(d).unwrap();
    }
    #[test]
    #[cfg(not(unix))]
    fn native_hooks_reject_platforms_without_owned_process_group_supervision() {
        let error = Sidecar::spawn_bounded(
            &std::env::temp_dir(),
            &[],
            std::time::Duration::from_secs(2),
            65536,
            true,
        )
        .err()
        .expect("Unsupported native hooks must fail during provisioning");
        assert_eq!(error.kind(), std::io::ErrorKind::Unsupported);
    }
    #[test]
    #[cfg(unix)]
    fn bounded_hooks_enforce_production_frames_deadlines_and_reap_owned_node() {
        let root = std::env::temp_dir().join(format!("brain-bounded-hooks-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let file = root.join("hook.mjs");
        std::fs::write(&file,"export default function(ctx){if(ctx.large)return 'x'.repeat(70000);if(ctx.loop)while(true){};return {mode:process.env.NODE_ENV,ownedGroup:process.env.BRAIN_SIDECAR_GROUP==='1',payload:ctx.payload}}
").unwrap();
        let timeout = std::time::Duration::from_millis(500);
        let mut hook = Sidecar::spawn_bounded(
            &root,
            &[("NODE_ENV".into(), "development".into())],
            timeout,
            65536,
            true,
        )
        .unwrap();
        let result = hook
            .invoke(
                &file_url(&file),
                "default",
                r#"{"payload":{"ok":true}}"#,
                None,
            )
            .unwrap();
        assert!(result.contains("production"));
        assert!(
            result.contains("\"ownedGroup\":true"),
            "The Rust spawn path must enable owned-group supervision in the actual Node process"
        );
        assert!(!result.contains("development"));
        assert!(
            hook.invoke(&file_url(&file), "default", r#"{"large":true}"#, None)
                .is_err()
        );
        assert!(hook.child.try_wait().unwrap().is_some());
        drop(hook);
        let mut hook = Sidecar::spawn_bounded(&root, &[], timeout, 65536, true).unwrap();
        let started = std::time::Instant::now();
        assert_eq!(
            hook.invoke(&file_url(&file), "default", r#"{"loop":true}"#, None)
                .unwrap_err()
                .0,
            504
        );
        assert!(started.elapsed() < std::time::Duration::from_secs(3));
        assert!(hook.child.try_wait().unwrap().is_some());
        drop(hook);
        let mut hook = Sidecar::spawn_bounded(&root, &[], timeout, 1024, true).unwrap();
        let started = std::time::Instant::now();
        assert!(
            hook.invoke(
                &file_url(&file),
                "default",
                &format!(r#"{{"v":"{}"}}"#, "x".repeat(2048)),
                None
            )
            .is_err()
        );
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
        drop(hook);
        #[cfg(unix)]
        {
            std::fs::write(&file,"export default async function(){const {spawn}=await import('node:child_process');spawn(process.execPath,['-e','setTimeout(()=>{},3000)'],{stdio:['ignore','inherit','inherit']});return {ok:true}}\n").unwrap();
            let mut hook = Sidecar::spawn_bounded(&root, &[], timeout, 65536, true).unwrap();
            hook.invoke(&file_url(&file), "default", "{}", None)
                .unwrap();
            let started = std::time::Instant::now();
            drop(hook);
            assert!(
                started.elapsed() < std::time::Duration::from_secs(1),
                "Disposal must kill owned descendants before joining inherited stdout"
            );
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}
