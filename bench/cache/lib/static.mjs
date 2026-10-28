// Static host emulator for a BRAINLLM dist/ (INTERNALS §10, §35). It serves exactly what the build asks a static host
// (Netlify, Cloudflare Pages) to do and nothing more:
// - headers come only from dist/_headers (every matching rule is merged, like Netlify; duplicates are recorded);
// - like those hosts, every 200 carries a strong ETag of the file bytes and If-None-Match answers 304 with no body;
// - clean URLs: /x and /x/ → x/index.html (also .brain for the §10 default extension);
// - precompressed .br / .gz siblings are served when the client accepts them;
// - dist/_brain/* is never served (the nginx snippet denies it);
// - optional /api/* probe endpoints (JSON, no-store, configurable delay) for resource() tests.
// Every request is logged with the bytes written to the socket (status line + headers + body).
import http from 'node:http'
import { readFileSync, existsSync, statSync } from 'node:fs'
import { join, extname } from 'node:path'
import { createHash } from 'node:crypto'

const MIME = {
  '.html': 'text/html; charset=utf-8', '.brain': 'text/html; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8', '.jsbrain': 'text/javascript; charset=utf-8', '.mjs': 'text/javascript; charset=utf-8',
  '.css': 'text/css; charset=utf-8', '.cssbrain': 'text/css; charset=utf-8',
  '.svg': 'image/svg+xml', '.png': 'image/png', '.jpg': 'image/jpeg', '.webp': 'image/webp', '.avif': 'image/avif',
  '.woff2': 'font/woff2', '.json': 'application/json', '.txt': 'text/plain; charset=utf-8', '.xml': 'application/xml',
  '.ico': 'image/x-icon', '.webmanifest': 'application/manifest+json',
}

export function parseHeaders(text) {
  const rules = []
  let cur = null
  for (const raw of text.split('\n')) {
    if (!raw.trim() || raw.trim().startsWith('#')) continue
    if (!/^\s/.test(raw)) { cur = { path: raw.trim(), headers: [] }; rules.push(cur); continue }
    const i = raw.indexOf(':')
    if (cur && i > 0) cur.headers.push([raw.slice(0, i).trim(), raw.slice(i + 1).trim()])
  }
  return rules
}

const match = (pattern, path) => {
  const re = new RegExp('^' + pattern.split('*').map((s) => s.replace(/[.+?^${}()|[\]\\]/g, '\\$&')).join('.*') + '$')
  return re.test(path)
}

export function startStatic({ port = 0, api = {} } = {}) {
  let root = null, label = null, rules = [], log = [], dupes = []
  const setRoot = (dir, name = null) => {
    root = dir
    label = name
    const h = join(dir, '_headers')
    rules = existsSync(h) ? parseHeaders(readFileSync(h, 'utf8')) : []
  }
  const resolve = (p) => {
    const cands = p.endsWith('/') ? [p + 'index.html', p + 'index.brain'] : [p, p + '/index.html', p + '/index.brain', p + '.html']
    for (const c of cands) {
      const f = join(root, decodeURIComponent(c))
      if (!f.startsWith(root)) return null
      if (existsSync(f) && statSync(f).isFile()) return f
    }
    return null
  }
  const server = http.createServer(async (req, res) => {
    const url = new URL(req.url, 'http://x')
    const p = url.pathname
    const entry = { t: Date.now(), root: label, method: req.method, path: p, status: 0, bytes: 0, body: 0, enc: '', preload: !!req.headers['service-worker-navigation-preload'], sw: req.headers['service-worker'] === 'script', inm: !!req.headers['if-none-match'] }
    log.push(entry)
    let wrote = 0
    const w = res.write.bind(res), e = res.end.bind(res)
    res.write = (c, ...a) => { if (c) wrote += Buffer.byteLength(c); return w(c, ...a) }
    res.end = (c, ...a) => { if (c && typeof c !== 'function') wrote += Buffer.byteLength(c); return e(c, ...a) }
    res.on('finish', () => {
      entry.status = res.statusCode
      entry.body = wrote
      // header block size as written by node (status line + headers + CRLF)
      entry.bytes = wrote + Buffer.byteLength(res._header || '')
    })
    if (api[p]) {
      const a = api[p]
      await new Promise((r) => setTimeout(r, a.delay ?? 0))
      res.writeHead(200, { 'content-type': 'application/json', 'cache-control': a.cc ?? 'no-store' })
      return res.end(JSON.stringify(typeof a.body === 'function' ? a.body() : a.body))
    }
    if (p.startsWith('/_brain/')) { res.writeHead(404); return res.end('not found') }
    const f = resolve(p)
    if (!f) { res.writeHead(404, { 'content-type': 'text/plain' }); return res.end('not found') }
    const headers = { 'content-type': MIME[extname(f)] || 'application/octet-stream' }
    const seen = new Map()
    for (const r of rules) if (match(r.path, p)) for (const [k, v] of r.headers) {
      const key = k.toLowerCase()
      if (seen.has(key) && seen.get(key) !== v) { dupes.push({ path: p, header: key, values: [seen.get(key), v] }); headers[key] += ', ' + v }
      else headers[key] = v
      seen.set(key, v)
    }
    let bytes = readFileSync(f)
    const ae = String(req.headers['accept-encoding'] || '')
    for (const [enc, ext] of [['br', '.br'], ['gzip', '.gz']]) {
      if (ae.includes(enc) && existsSync(f + ext)) { bytes = readFileSync(f + ext); headers['content-encoding'] = enc; entry.enc = enc; break }
    }
    headers.etag = '"' + createHash('sha1').update(bytes).digest('hex').slice(0, 16) + '"'
    const inm = req.headers['if-none-match']
    if (inm && inm.split(',').some((t) => t.trim().replace(/^W\//, '') === headers.etag)) {
      delete headers['content-type']; delete headers['content-encoding']
      res.writeHead(304, headers); return res.end()
    }
    headers['content-length'] = bytes.length
    res.writeHead(200, headers)
    res.end(req.method === 'HEAD' ? undefined : bytes)
  })
  const sockets = new Set()
  server.on('connection', (s) => { sockets.add(s); s.on('close', () => sockets.delete(s)) })
  return new Promise((ok) => server.listen(port, '127.0.0.1', () => ok({
    port: server.address().port,
    setRoot,
    rules: () => rules,
    log: () => log,
    dupes: () => dupes,
    mark: () => log.length,
    since: (m) => log.slice(m),
    close: () => new Promise((r) => { for (const s of sockets) s.destroy(); server.close(() => r()) }),
  })))
}
