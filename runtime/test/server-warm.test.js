// The first request after a (re)start must not pay for importing the app's modules: `node server.mjs` imports every
// module a request can need (route server modules, views, layouts, guards, i18n, RPC modules, the DOM shim) before it
// prints `brain:ready`, so the supervisor only sends traffic to a warm instance. Counted, never timed.
import { test, after } from 'node:test'
import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { copyFileSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync, existsSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = fileURLToPath(new URL('.', import.meta.url))
const dist = mkdtempSync(join(tmpdir(), 'brain-warm-'))
const marks = join(dist, 'imports.log')
after(() => rmSync(dist, { recursive: true, force: true }))
const w = (rel, s) => (mkdirSync(join(dist, rel, '..'), { recursive: true }), writeFileSync(join(dist, rel), s))
// each module appends its name when it is imported (top level), once per process
const mark = name => `import { appendFileSync } from 'node:fs'\nappendFileSync(${JSON.stringify(marks)}, ${JSON.stringify(name + '\n')})\n`
const imported = () => (existsSync(marks) ? readFileSync(marks, 'utf8').split('\n').filter(Boolean) : [])

mkdirSync(join(dist, '_b'), { recursive: true })
copyFileSync(join(here, '../brain.js'), join(dist, '_b/brain.js'))
w('_server/dom.js', readFileSync(join(here, '../shim/dom.js'), 'utf8'))
for (const f of ['server.mjs', 'llm.mjs', 'native-sockets.mjs', 'redis-cache.mjs']) copyFileSync(join(here, '../server', f), join(dist, f))
w('_b/a.js', `${mark('view')}import { tpl } from '/_b/brain.js'\nexport default function A(p) { return tpl('<p>n ' + p.n + '</p>')() }\n`)
w('_b/l.js', `${mark('layout')}import { tpl } from '/_b/brain.js'\nexport default function L(p) { return p.children() }\n`)
w('_server/a.server.mjs', `${mark('server')}export function load() { return { n: 1 } }\n`)
w('_server/g.mjs', `${mark('guard')}export default () => undefined\n`)
w('_server/rpc.mjs', `${mark('rpc')}export async function ping() { return 'pong' }\n`)
w('_server/broken.server.mjs', `${mark('broken')}throw new Error('broken at import')\n`)
w('_server/pages/p.html', '<!doctype html><html><head><title>t</title></head><body><div id="brain-view"><!--brain:ssr--></div><script type="application/json" id="brain-props"><!--brain:props--></script></body></html>')
w('_server/manifest.json', JSON.stringify({
  v: 1, build: 'b1', core: '/_b/brain.js', rpc: { r1: '_server/rpc.mjs' },
  routes: [
    { path: '/a', rx: '^/a/?$', view: '/_b/a.js', layouts: ['/_b/l.js'], guards: ['_server/g.mjs'], server: '_server/a.server.mjs', page: '_server/pages/p.html' },
    { path: '/broken', rx: '^/broken/?$', view: '/_b/a.js', layouts: [], server: '_server/broken.server.mjs', page: '_server/pages/p.html' },
  ],
}))

// starts `node dist/server.mjs` and resolves with what was imported at the moment it printed `brain:ready`
function start() {
  const child = spawn(process.execPath, [join(dist, 'server.mjs')], {
    env: { PATH: process.env.PATH, PORT: '0', HOST: '127.0.0.1', BRAIN_SECRET: 'test-only-session-key-of-32-bytes!!' },
    stdio: ['ignore', 'pipe', 'pipe'],
  })
  let out = '', err = ''
  child.stderr.on('data', d => (err += d))
  return new Promise((ok, fail) => {
    child.stdout.on('data', d => {
      out += d
      if (/brain:ready \d+/.test(out)) ok({ child, atReady: imported() })
    })
    child.once('exit', code => fail(new Error(`server exited ${code} before ready: ${err}`)))
  })
}

test('every module a request can import is imported before brain:ready; a module that throws does not stop the start', async () => {
  const { child, atReady } = await start()
  try {
    for (const name of ['server', 'view', 'layout', 'guard', 'rpc', 'broken']) assert.ok(atReady.includes(name), `${name} imported before ready (got ${atReady.join(', ')})`)
    // each module once: a request after ready reuses the import instead of loading it again
    assert.equal(atReady.length, new Set(atReady).size)
  } finally {
    child.kill('SIGTERM')
    await new Promise(r => child.once('exit', r))
  }
})

test('createServer stays lazy unless asked to warm, and warm() imports each module once', async () => {
  rmSync(marks, { force: true })
  const { createServer } = await import('../server/server.mjs')
  const lazy = createServer({ dist, secret: 'test-only-session-key', log() {} })
  assert.equal(lazy.$brainWarm, undefined)
  assert.deepEqual(imported(), [])
  const logs = []
  const warmed = createServer({ dist, secret: 'test-only-session-key', warm: true, log: (...e) => logs.push(e) })
  const stats = await warmed.$brainWarm
  assert.deepEqual([...imported()].sort(), ['broken', 'guard', 'layout', 'rpc', 'server', 'view'])
  assert.equal(stats.failed, 1) // the broken module: reported, its route still answers 500 when asked
  await new Promise(r => warmed.listen(0, '127.0.0.1', r))
  try {
    const base = `http://127.0.0.1:${warmed.address().port}`
    const a = await fetch(base + '/a')
    assert.equal(a.status, 200)
    assert.match(await a.text(), /n 1/)
    assert.equal((await fetch(base + '/broken')).status, 500)
    assert.deepEqual([...imported()].sort(), ['broken', 'guard', 'layout', 'rpc', 'server', 'view']) // nothing imported twice
  } finally {
    await new Promise(r => warmed.close(r))
  }
})
