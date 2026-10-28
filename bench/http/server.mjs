// Fault-injection API server for the HTTP client bench (QA #172, INTERNALS §40–§41.4). Node ≥ 22, no dependencies.
//
// One origin serves both the app and the API, so `api()` calls stay same-origin:
//   /f/<path>, /api/f/<path>  the scripted API (any prefix before `/f/` is kept in the log as `raw`, stripped in `path`)
//   /__ctl/*                  control endpoint (plan, log, reset), used by the runner
//   anything else             the app: proxied to `upstream` (`brain run`) or served from `staticDir` (`dist/`)
//
// What a request gets (first match wins):
//   1. header `x-fault: status=503;retryAfter=1`  2. query `?_fault=status=503;delay=200`
//   3. the plan: POST /__ctl/plan {"GET /f/items": [answer, answer, …]} (consumed in order, the last one repeats)
//   4. default: 200 `{}` (a multipart upload answers `{ ok, bytes, fields, files }`)
// Answer fields: status (200) · json (any, sent as JSON) · body (raw string) · headers ({}) · retryAfter (seconds or a
// date string, sets Retry-After) · delay (ms before the answer) · reset (destroy the socket, no answer) · hang (never
// answer) · readRate (bytes/s while reading the request body: a slow upload).
// Every response carries `x-request-id` (the client's own if it sent one, else `rid-<seq>`).
//
// Log (GET /__ctl/log): one entry per API request with seq, method, path, raw, query, t0/t1 (ms since reset), status,
// aborted (the client closed before the answer), reset, rid, bytes received, content-type, selected headers, and
// `inflight` at arrival; plus `peak` (max concurrent API requests). POST /__ctl/reset clears log and plan.
//
// Standalone: node bench/http/server.mjs --port 5197 [--upstream http://127.0.0.1:5196 | --static path/to/dist]
import { createServer, request as httpRequest } from 'node:http'
import { connect } from 'node:net'
import { readFileSync, statSync } from 'node:fs'
import { extname, join, normalize } from 'node:path'

const MIME = {
  '.html': 'text/html; charset=utf-8', '.js': 'text/javascript; charset=utf-8', '.mjs': 'text/javascript; charset=utf-8',
  '.css': 'text/css; charset=utf-8', '.json': 'application/json', '.txt': 'text/plain; charset=utf-8',
  '.svg': 'image/svg+xml', '.png': 'image/png', '.webmanifest': 'application/manifest+json', '.md': 'text/markdown; charset=utf-8',
}

// `status=503;retryAfter=1;delay=200` → { status: 503, retryAfter: 1, delay: 200 }
export function parseFault(s) {
  const a = {}
  for (const part of String(s).split(';')) {
    const [k, v] = part.split('=')
    if (!k) continue
    a[k.trim()] = v === undefined ? true : /^-?\d+(\.\d+)?$/.test(v) ? Number(v) : v === 'true' ? true : v === 'false' ? false : v
  }
  return a
}

export function createFaultServer({ upstream = null, staticDir = null } = {}) {
  let plan = {}, log = [], seq = 0, inflight = 0, peak = 0, epoch = Date.now()
  const sockets = new Set()
  const now = () => Date.now() - epoch

  function answerFor(req, url, key) {
    if (req.headers['x-fault']) return parseFault(req.headers['x-fault'])
    if (url.searchParams.has('_fault')) return parseFault(url.searchParams.get('_fault'))
    const list = plan[key]
    if (list && list.length) return list.length > 1 ? list.shift() : list[0]
    return null
  }

  function readBody(req, rate) {
    return new Promise((ok) => {
      const chunks = []
      let n = 0
      req.on('data', (c) => {
        chunks.push(c)
        n += c.length
        if (rate) { req.pause(); setTimeout(() => req.resume(), (c.length / rate) * 1000) }
      })
      req.on('end', () => ok(Buffer.concat(chunks, n)))
      req.on('error', () => ok(Buffer.concat(chunks, n)))
      req.on('aborted', () => ok(Buffer.concat(chunks, n)))
    })
  }

  // minimal multipart summary: field names, file names and sizes
  function multipart(buf, ct) {
    const m = /boundary=(?:"([^"]+)"|([^;]+))/.exec(ct || '')
    if (!m) return null
    const b = Buffer.from('--' + (m[1] || m[2]))
    const fields = {}, files = []
    let i = buf.indexOf(b)
    while (i >= 0) {
      const next = buf.indexOf(b, i + b.length)
      if (next < 0) break
      const part = buf.subarray(i + b.length + 2, next - 2)
      const sep = part.indexOf('\r\n\r\n')
      const head = part.subarray(0, sep).toString(), data = part.subarray(sep + 4)
      const name = /name="([^"]*)"/.exec(head)?.[1], file = /filename="([^"]*)"/.exec(head)?.[1]
      if (file !== undefined) files.push({ field: name, filename: file, size: data.length })
      else if (name) fields[name] = data.toString()
      i = next
    }
    return { fields, files }
  }

  async function api(req, res, url) {
    const raw = url.pathname
    const path = raw.slice(raw.indexOf('/f/'))
    const key = req.method + ' ' + path
    const a = answerFor(req, url, key) || {}
    const rid = req.headers['x-request-id'] || 'rid-' + (seq + 1)
    const e = {
      seq: ++seq, method: req.method, path, raw, query: Object.fromEntries([...url.searchParams].filter(([k]) => k !== '_fault')),
      t0: now(), t1: null, status: null, aborted: false, reset: false, rid, bytes: 0, ct: req.headers['content-type'] || null,
      headers: { accept: req.headers.accept, 'x-bench': req.headers['x-bench'], authorization: req.headers.authorization },
      inflight: ++inflight,
    }
    peak = Math.max(peak, inflight)
    log.push(e)
    let done = false
    const finish = () => { if (!done) { done = true; inflight--; e.t1 = now() } }
    res.on('close', () => { if (!res.writableEnded) e.aborted = true; finish() })
    const body = await readBody(req, a.readRate)
    e.bytes = body.length
    if (req.aborted || req.socket.destroyed) { e.aborted = true; return finish() } // the client left mid-body
    if (a.hang) return
    if (a.delay) await new Promise((r) => setTimeout(r, a.delay))
    if (res.destroyed) return
    if (a.reset) { e.reset = true; e.status = 0; req.socket.destroy(); return }
    let payload = a.body !== undefined ? String(a.body) : a.json !== undefined ? JSON.stringify(a.json) : null
    if (payload === null) {
      const mp = /multipart\/form-data/.test(e.ct || '') ? multipart(body, e.ct) : null
      if (mp) { e.multipart = mp; payload = JSON.stringify({ ok: true, bytes: body.length, ...mp }) }
      else payload = (a.status || 200) === 204 ? '' : '{}'
    }
    const headers = { 'content-type': 'application/json', 'cache-control': 'no-store', 'x-request-id': rid, ...(a.headers || {}) }
    if (a.retryAfter !== undefined) headers['retry-after'] = String(a.retryAfter)
    e.status = a.status || 200
    res.writeHead(e.status, headers)
    res.end(payload)
  }

  async function ctl(req, res, url) {
    const json = (v) => res.writeHead(200, { 'content-type': 'application/json' }).end(JSON.stringify(v))
    if (url.pathname === '/__ctl/reset') { plan = {}; log = []; seq = 0; peak = 0; inflight = 0; epoch = Date.now(); return json({ ok: true }) }
    if (url.pathname === '/__ctl/plan') {
      const b = JSON.parse((await readBody(req)).toString() || '{}')
      for (const k of Object.keys(b)) plan[k] = [...b[k]]
      return json({ ok: true })
    }
    if (url.pathname === '/__ctl/log') return json({ peak, inflight, log })
    res.writeHead(404).end()
  }

  function proxy(req, res) {
    const u = new URL(upstream)
    const p = httpRequest({ host: u.hostname, port: u.port, method: req.method, path: req.url, headers: { ...req.headers, host: u.host } }, (r) => {
      const { connection, 'keep-alive': _ka, ...h } = r.headers
      res.writeHead(r.statusCode, h)
      r.pipe(res)
    })
    p.on('error', () => { if (!res.headersSent) res.writeHead(502).end('upstream down') })
    req.pipe(p)
  }

  const isFile = (p) => { try { return statSync(p).isFile() ? p : null } catch { return null } }
  function serveStatic(req, res, url) {
    const p = normalize(join(staticDir, decodeURIComponent(url.pathname)))
    if (!p.startsWith(staticDir) || url.pathname.startsWith('/_brain/')) return res.writeHead(404).end()
    const hit = isFile(p) || isFile(join(p, 'index.html')) || isFile(p + '.html')
    if (!hit) {
      const nf = isFile(join(staticDir, '404.html'))
      return res.writeHead(404, { 'content-type': 'text/html; charset=utf-8' }).end(nf ? readFileSync(nf) : 'not found')
    }
    res.writeHead(200, { 'content-type': MIME[extname(hit)] || 'application/octet-stream', 'cache-control': 'no-cache' })
    res.end(readFileSync(hit))
  }

  const srv = createServer((req, res) => {
    // one request per connection: Chrome silently re-sends a request whose reused keep-alive socket is reset, which
    // would hide the client's own retry policy (a reset POST would reach the server several times)
    res.shouldKeepAlive = false
    const url = new URL(req.url, 'http://x')
    if (url.pathname.startsWith('/__ctl/')) return ctl(req, res, url)
    if (/^(\/[^/]+)*\/f\//.test(url.pathname) && !url.pathname.startsWith('/_brain') && !url.pathname.startsWith('/@')) return api(req, res, url)
    if (upstream) return proxy(req, res)
    if (staticDir) return serveStatic(req, res, url)
    res.writeHead(404).end()
  })
  srv.on('connection', (s) => { sockets.add(s); s.on('close', () => sockets.delete(s)) })
  // dev server live-reload sockets pass through untouched
  srv.on('upgrade', (req, sock, head) => {
    if (!upstream) return sock.destroy()
    const u = new URL(upstream)
    const up = connect(Number(u.port), u.hostname, () => {
      up.write(`${req.method} ${req.url} HTTP/1.1\r\n` + Object.entries({ ...req.headers, host: u.host }).map(([k, v]) => `${k}: ${v}`).join('\r\n') + '\r\n\r\n')
      if (head && head.length) up.write(head)
      up.pipe(sock); sock.pipe(up)
    })
    up.on('error', () => sock.destroy()); sock.on('error', () => up.destroy())
  })

  return {
    listen: (port = 0) => new Promise((r) => srv.listen(port, '127.0.0.1', () => r(`http://127.0.0.1:${srv.address().port}`))),
    close: () => new Promise((r) => { for (const s of sockets) s.destroy(); srv.close(() => r()) }),
  }
}

if (import.meta.url === `file://${process.argv[1]}`) {
  const argv = process.argv.slice(2)
  const arg = (k, d) => { const i = argv.indexOf(`--${k}`); return i >= 0 ? argv[i + 1] : d }
  const s = createFaultServer({ upstream: arg('upstream', null), staticDir: arg('static', null) })
  const url = await s.listen(Number(arg('port', 5197)))
  console.log(`fault server on ${url} (pid ${process.pid})`)
}
