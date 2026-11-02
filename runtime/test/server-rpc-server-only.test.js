// RPC and form posts call only the functions a view declares with `server:` (the generated module lists them in
// `$server`). `load`, `paths`, `sitemap`, `seo`, `head` and any other export answer as an unknown function does, run
// nothing and never create a rate-limit key.
import { test, after } from 'node:test'
import assert from 'node:assert/strict'
import { copyFileSync, mkdirSync, mkdtempSync, realpathSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = fileURLToPath(new URL('.', import.meta.url))
const dist = realpathSync(mkdtempSync(join(tmpdir(), 'brain-dist-server-only-'))) // the server imports through realpath
const w = (rel, s) => {
  mkdirSync(join(dist, rel, '..'), { recursive: true })
  writeFileSync(join(dist, rel), s)
}
mkdirSync(join(dist, '_server'), { recursive: true })
for (const f of ['brain.js', 'brain-form.js', 'brain-schema.js']) copyFileSync(join(here, '..', f), join(dist, '_server', f))
w('_server/page.mjs', `export const ran = []
const helper = () => 'helper'
export async function load(ctx) { ran.push('load'); return { secret: 'page-data' } }
export async function paths() { ran.push('paths'); return [] }
export async function sitemap() { ran.push('sitemap'); return [] }
export async function save(ctx, v) { ran.push('save'); return { redirect: '/done' } }
export const $forms = { save: { email: 'str!' } }
export const seo = (props) => ({ title: 'Private' })
export const head = (props) => '<meta name="x">'
export const $server = Object.freeze(["save"])
`)
w('index.html', '<!doctype html><title>Home</title><main><h1>Home</h1></main>')
w('404.html', '<!doctype html><title>404</title><main><h1>Lost</h1></main>')
w('_server/manifest.json', JSON.stringify({
  v: 1, build: 'b1', core: '/_b/core.js',
  routes: [{ path: '/page', rx: '^/page/?$', view: '/_b/none.js', layouts: [], server: '_server/page.mjs', forms: true }],
  rpc: { pg: '_server/page.mjs' },
  notFound: '404.html',
}))

const { createServer, rateLimitKeys, callForm, serverFn } = await import('../server/server.mjs')
const server = createServer({ dist, secret: 'test-secret-0123456789', env: { BRAIN_SECRET: 's' }, log: () => {} }) // gitleaks:allow -- local test-only session secret
await new Promise(r => server.listen(0, '127.0.0.1', r))
const base = `http://127.0.0.1:${server.address().port}`
after(() => server.close())
const rpc = path => fetch(base + '/_r/' + path, { method: 'POST', headers: { origin: base, 'x-brain-rpc': '1', 'content-type': 'application/json' }, body: '[]' })
const form = id => fetch(base + '/page', { method: 'POST', redirect: 'manual', headers: { origin: base, 'content-type': 'application/x-www-form-urlencoded' }, body: '_brain=' + encodeURIComponent(id) + '&email=a' })
const page = await import(join(dist, '_server/page.mjs'))
const OTHERS = ['load', 'paths', 'sitemap', 'seo', 'head', '$forms', '$server', 'ran']

test('RPC answers 404 Unknown function for every export that is not a server: function', async () => {
  const before = rateLimitKeys()
  for (const fn of OTHERS) {
    const r = await rpc('pg/' + fn)
    assert.equal(r.status, 404, fn)
    assert.deepEqual(await r.json(), { error: 'Unknown function' }, fn)
  }
  assert.deepEqual(page.ran, [], 'nothing ran')
  assert.equal(rateLimitKeys(), before, 'no rate-limit key for a refused name')
  const ok = await rpc('pg/save')
  assert.equal(ok.status, 200)
  assert.deepEqual(page.ran, ['save'])
  assert.equal(rateLimitKeys(), before + 1)
})

test('a form post naming load or another export is an unknown form and runs nothing', async () => {
  page.ran.length = 0
  const before = rateLimitKeys()
  const unknown = await form('pg/nope'), shape = t => t.replace(/<code>[^<]*<\/code>/, '')
  assert.equal(unknown.status, 400)
  const expected = shape(await unknown.text())
  for (const fn of OTHERS) {
    const r = await form('pg/' + fn)
    assert.equal(r.status, 400, fn)
    assert.equal(shape(await r.text()), expected, fn + ' answers as an unknown name does')
  }
  assert.deepEqual(page.ran, [])
  assert.equal(rateLimitKeys(), before)
  assert.equal((await form('pg/save')).status, 303)
  assert.deepEqual(page.ran, ['save'])
})

test('serverFn and callForm read the module\'s $server list; a module without one exposes nothing', async () => {
  assert.equal(serverFn(page, 'save'), page.save)
  for (const fn of OTHERS) assert.equal(serverFn(page, fn), undefined, fn)
  const bare = { save: async () => 1, load: async () => 2 }
  assert.equal(serverFn(bare, 'save'), undefined)
  assert.equal(serverFn({ $server: ['gone'] }, 'gone'), undefined, 'a listed name that is not a function')
  assert.equal(serverFn(null, 'save'), undefined)
  const r = await callForm({ module: { ...bare, $forms: {} }, raw: '_brain=h%2Fsave', ctx: {} })
  assert.equal(r.status, 400)
})
