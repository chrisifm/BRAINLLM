// A compiler budget failure (B0910/B0911, brain-budget.js) thrown by a view load() or an RPC keeps its own code, budget,
// limit, source position and fix in the production log instead of the generic NODE_INTERNAL_ERROR; the message stays out.
import { test, after } from 'node:test'
import assert from 'node:assert/strict'
import { copyFileSync, mkdirSync, mkdtempSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = fileURLToPath(new URL('.', import.meta.url))
const dist = mkdtempSync(join(tmpdir(), 'brain-budget-log-'))
const w = (rel, s) => (mkdirSync(join(dist, rel, '..'), { recursive: true }), writeFileSync(join(dist, rel), s))
mkdirSync(join(dist, '_b'), { recursive: true })
copyFileSync(join(here, '../brain.js'), join(dist, '_b/brain.js'))
mkdirSync(join(dist, '_server'), { recursive: true })
copyFileSync(join(here, '../shim/dom.js'), join(dist, '_server/dom.js'))
const budget = `Object.assign(new Error('B0910: ms budget exceeded'), { code: 'B0910', kind: 'budget', budget: 'ms', limit: 200, file: 'views/posts.brain', component: 'Posts', line: 3, column: 5, fix: 'Bound the work or move it to an owned worker.' })`
w('_b/v.js', `import { tpl } from '/_b/brain.js'\nexport default function V(p) { return tpl('<h1>' + p.msg + '</h1>')() }\n`)
w('_server/v.server.mjs', `export async function load() { throw ${budget} }\n`)
w('_server/items.mjs', `export async function heavy() { throw ${budget} }\nexport async function plain() { throw new Error('secret value 42') }\n`)
w('_b/hang.js', `export default function V() { return document.createTextNode('x') }\n`)
w('_server/hang.server.mjs', `export function load(ctx) { globalThis.__hang(ctx.signal); return new Promise(() => {}) }\n`)
w('_server/fine.server.mjs', `export function load(ctx) { globalThis.__fine = ctx.signal; return {} }\n`)
w('_server/pages/p.html', '<!doctype html><html><head><title>t</title></head><body><div id="brain-view"><!--brain:ssr--></div><script type="application/json" id="brain-props"><!--brain:props--></script></body></html>')
w('_server/manifest.json', JSON.stringify({
  v: 1, build: 'b1', core: '/_b/brain.js', rpc: { hb: '_server/items.mjs' },
  routes: [{ path: '/posts', rx: '^/posts/?$', view: '/_b/v.js', layouts: [], server: '_server/v.server.mjs', page: '_server/pages/p.html' },
    { path: '/hang', rx: '^/hang/?$', view: '/_b/hang.js', layouts: [], server: '_server/hang.server.mjs', page: '_server/pages/p.html' },
    { path: '/fine', rx: '^/fine/?$', view: '/_b/hang.js', layouts: [], server: '_server/fine.server.mjs', page: '_server/pages/p.html' }],
}))

const { createServer, callRpc } = await import('../server/server.mjs')
const logs = []
const server = createServer({ dist, secret: 'test-secret-0123456789', log: (level, msg, x) => logs.push({ level, msg, ...x }) }) // gitleaks:allow -- local test-only session secret
await new Promise(r => server.listen(0, '127.0.0.1', r))
const base = `http://127.0.0.1:${server.address().port}`
after(() => server.close())
const rpc = name => fetch(`${base}/_r/hb/${name}`, { method: 'POST', headers: { 'content-type': 'application/json', 'x-brain-rpc': '1' }, body: '[]' })

function assertBudget(entry) {
  assert.ok(entry, 'logged')
  assert.equal(entry.code, 'B0910')
  assert.equal(entry.budget, 'ms')
  assert.equal(entry.limit, 200)
  assert.equal(entry.file, 'views/posts.brain')
  assert.equal(entry.component, 'Posts')
  assert.equal(entry.line, 3)
  assert.equal(entry.column, 5)
  assert.equal(entry.fix, 'Bound the work or move it to an owned worker.')
  assert.doesNotMatch(JSON.stringify(entry), /budget exceeded/) // the message is never logged
}

test('a budget failure in a view load() is logged with its code, budget, position and fix', async () => {
  const r = await fetch(`${base}/posts`)
  assert.equal(r.status, 500)
  assertBudget(logs.find(l => l.msg === 'load failed'))
})

test('a budget failure in an RPC is logged the same way; other errors keep the generic code', async () => {
  assert.equal((await rpc('heavy')).status, 500)
  assertBudget(logs.find(l => l.msg === 'rpc failed' && l.code === 'B0910') || logs.filter(l => l.msg === 'rpc failed').at(-1))
  assert.equal((await rpc('plain')).status, 500)
  const plain = logs.filter(l => l.msg === 'rpc failed').at(-1)
  assert.equal(plain.code, 'NODE_INTERNAL_ERROR')
  assert.equal(plain.budget, undefined)
  assert.doesNotMatch(JSON.stringify(plain), /secret value/)
})

test('an unhandled request failure logs the path without its query string', async () => {
  // A request that reaches the outer catch: the manifest route module is replaced by one whose getter throws.
  const before = logs.length
  const original = server.listeners('request')[0]
  const secret = 'query-token-value'
  const req = { url: '/posts?token=' + secret, method: 'GET', headers: { get host() { throw new Error('boom') } }, socket: {}, on() {}, once() {}, removeListener() {} }
  const res = { req, setHeader() {}, getHeader() {}, on() {}, once() {}, removeListener() {}, end() {}, writeHead() {}, write() {}, headersSent: false }
  await original(req, res)
  const entry = logs.slice(before).find(l => l.msg === 'unhandled')
  assert.ok(entry, 'logged')
  assert.equal(entry.path, '/posts')
  assert.doesNotMatch(JSON.stringify(logs.slice(before)), new RegExp(secret))
})

test('ctx.signal aborts when the client disconnects, and not after a finished answer', async () => {
  const { request } = await import('node:http')
  const seen = new Promise(resolve => (globalThis.__hang = resolve))
  const client = request(base + '/hang')
  client.on('error', () => {})
  client.end()
  const signal = await seen
  assert.equal(signal.aborted, false)
  const aborted = new Promise(resolve => signal.addEventListener('abort', resolve, { once: true }))
  client.destroy()
  await aborted
  assert.equal(signal.aborted, true)
  const closed = fetch(base + '/fine', { headers: { connection: 'close' } })
  await (await closed).text()
  await new Promise(setImmediate)
  assert.equal(globalThis.__fine.aborted, false)
})

test('ctx.signal aborts at the deadline of a server function', async () => {
  let signal
  const r = await callRpc({ fn: ctx => { signal = ctx.signal; return new Promise(() => {}) }, args: [], ctx: {}, timeout: 1 })
  assert.equal(r.status, 504)
  assert.equal(signal.aborted, true)
})
