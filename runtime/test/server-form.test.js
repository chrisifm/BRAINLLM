// server.mjs: a form posted without JS (INTERNALS §38) against a fake dist with a real view, form() and the schema.
import { test, after } from 'node:test'
import assert from 'node:assert/strict'
import { copyFileSync, mkdirSync, mkdtempSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = fileURLToPath(new URL('.', import.meta.url))
const dist = mkdtempSync(join(tmpdir(), 'brain-form-dist-'))
const w = (rel, s) => (mkdirSync(join(dist, rel, '..'), { recursive: true }), writeFileSync(join(dist, rel), s))
mkdirSync(join(dist, '_b'), { recursive: true })
mkdirSync(join(dist, '_server/pages'), { recursive: true })
for (const [from, to] of [['../brain-i18n.js', '_b/i18n.js'], ['../brain-guard.js', '_b/guard.js'], ['../brain.js', '_b/brain.js'], ['../brain-form.js', '_b/form.js'], ['../brain.js', '_server/brain.js'], ['../brain-form.js', '_server/brain-form.js'], ['../brain-schema.js', '_server/brain-schema.js'], ['../brain-schema.js', '_b/brain-schema.js'], ['../shim/dom.js', '_server/dom.js']]) copyFileSync(join(here, from), join(dist, to))
// the client form chunk imports ./brain.js: same instance as the view in /_b/
w('_b/signup.js', `import { tpl, insert, attr } from '/_b/brain.js'
import { form } from '/_b/form.js'
const signup = Object.assign(() => {}, { id: 'abc/signup' })
const T = tpl('<form method="post"><input type="hidden" name="_brain" value="abc/signup"><input name="email"><p class="e"><!></p><p class="g"><!></p></form>')
export default function Signup() {
  const f = form({ email: 'email!', age: 'num>=18', tags: '[str]', terms: 'bool!', address: { city: 'str!' } }, { submit: signup })
  const r = T(), email = r.childNodes[1], e = r.childNodes[2], g = r.childNodes[3]
  attr(email, 'value', () => f.v.email)
  insert(null, () => f.err.email, e.firstChild)
  insert(null, () => f.error, g.firstChild)
  return r
}
`)
w('_server/signup.mjs', `export const $forms = { signup: { email: 'email!', age: 'num>=18', tags: '[str]', terms: 'bool!', address: { city: 'str!' } } }
export async function signup(ctx, v) {
  globalThis.__last = v
  if (v.email === 'taken@x.dev') throw Object.assign(new Error('x'), { errors: { email: 'Already taken' } })
  if (v.address.city === 'boom') throw new Error('db down: password=hunter2')
  if (v.email.startsWith('go')) return { redirect: '/welcome' }
  return 'ok'
}
`)
w('_b/hallo.js', `import { tpl, insert } from '/_b/brain.js'
import { t, add } from '/_b/i18n.js'
add('nl', { greeting: 'Hallo {name}' })
export default function Hallo() { const r = tpl('<p class="t"><!></p>')(); insert(null, () => t('greeting', { name: 'Ada' }), r.firstChild); return r }
`)
w('_server/guard.mjs', `export default ctx => ctx.query.key === 'ok' ? true : ctx.query.go ? { redirect: '/login' } : { deny: 'Admins only' }`)
w('_server/mw.mjs', `export default async (ctx, next) => { ctx.headers['x-mw'] = 'on'; await next() }`)
w('_server/boom.mjs', `export async function load() { throw new Error('load exploded') }`)
w('_b/plain.js', `import { tpl, insert } from '/_b/brain.js'
export default function Plain($p) { const r = tpl('<p class="p"><!></p>')(); insert(null, () => $p.reason || $p.message || 'admin area', r.firstChild); return r }
`)
w('_b/broken.js', `import { tpl } from '/_b/brain.js'
import { guard } from '/_b/guard.js'
const Ok = guard(() => tpl('<p class="ok">still here</p>')())
const Bad = guard(() => { throw new Error('widget exploded') })
Bad.f = 'comp/widget.brain'
export default function Broken() { return [Ok({}), Bad({})] }
`)
w('_server/pages/signup.html', '<!doctype html><html><head><title>Sign up</title></head><body><div id="brain-view"><!--brain:ssr--></div><script type="application/json" id="brain-props"><!--brain:props--></script></body></html>')
w('signup/index.html', '<!doctype html><html><head><title>Sign up</title></head><body><main>prerendered form</main></body></html>')
w('_server/manifest.json', JSON.stringify({ v: 1, build: 'f1', core: '/_b/brain.js', routes: [{ path: '/admin', view: '/_b/plain.js', layouts: [], server: null, page: '_server/pages/signup.html', guards: ['_server/guard.mjs'], middleware: ['_server/mw.mjs'] }, { path: '/boom', view: '/_b/plain.js', layouts: [], server: '_server/boom.mjs', page: '_server/pages/signup.html', error: { path: '/boom', view: '/_b/plain.js', layouts: [], server: null, page: '_server/pages/signup.html' } }, { path: '/nl/hallo', view: '/_b/hallo.js', layouts: [], server: null, page: '_server/pages/signup.html', locale: 'nl', alternates: [{ locale: 'en', href: '/hello' }, { locale: 'nl', href: '/nl/hallo' }] }, { path: '/broken', view: '/_b/broken.js', layouts: [], server: null, page: '_server/pages/signup.html' }, { path: '/signup', view: '/_b/signup.js', layouts: [], server: null, page: '_server/pages/signup.html', forms: true }], rpc: { abc: '_server/signup.mjs' }, forbidden: { path: '/403', view: '/_b/plain.js', layouts: [], server: null, page: '_server/pages/signup.html' } }))

const { createServer } = await import('../server/server.mjs')
const logs = []
const server = createServer({ dist, rateLimit: 50, log: (level, msg, x) => logs.push({ level, msg, ...x }) })
await new Promise(r => server.listen(0, '127.0.0.1', r))
const base = `http://127.0.0.1:${server.address().port}`
after(() => server.close())
const post = (fields, h = {}) =>
  fetch(base + '/signup', {
    method: 'POST',
    redirect: 'manual',
    headers: { 'content-type': 'application/x-www-form-urlencoded', origin: base, ...h },
    body: typeof fields === 'string' ? fields : new URLSearchParams(fields).toString(),
  })
const good = [['_brain', 'abc/signup'], ['email', 'ada@x.dev'], ['age', '20'], ['tags', 'a'], ['tags', 'b'], ['terms', 'on'], ['address.city', 'Puebla'], ['role', 'admin']]

test('valid post: converted with the schema (only its fields), the function runs, 303 back to the page (PRG)', async () => {
  const r = await post(good)
  assert.equal(r.status, 303)
  assert.equal(r.headers.get('location'), '/signup')
  assert.deepEqual(globalThis.__last, { email: 'ada@x.dev', age: 20, tags: ['a', 'b'], terms: true, address: { city: 'Puebla' } }) // no `role`
  const red = await post(good.map(([k, v]) => [k, k === 'email' ? 'go@x.dev' : v]))
  assert.equal(red.headers.get('location'), '/welcome')
})

test('invalid post: the function never runs; 422 page re-rendered with the values and the messages, state embedded', async () => {
  globalThis.__last = null
  const r = await post([['_brain', 'abc/signup'], ['email', 'ada@'], ['age', '16'], ['address.city', 'X']])
  assert.equal(r.status, 422)
  assert.equal(globalThis.__last, null)
  const h = await r.text()
  assert.match(h, /<input name="email" value="ada@">/) // what the user typed
  assert.match(h, /<p class="e">Enter a valid email/)
  const st = JSON.parse(h.match(/id="brain-forms">([^<]*)</)[1])['abc/signup']
  assert.deepEqual(st.errors, { email: 'Enter a valid email', age: 'Must be at least 18', terms: 'Required' })
  assert.equal(st.values.age, 16)
})

test('server-side errors: {errors} → 422 with the field message; a crash → 422 with a generic message (source frames only in the private log)', async () => {
  const taken = await post(good.map(([k, v]) => [k, k === 'email' ? 'taken@x.dev' : v]))
  assert.equal(taken.status, 422)
  assert.match(await taken.text(), /<p class="e">Already taken/)
  const crash = await post(good.map(([k, v]) => [k, k === 'address.city' ? 'boom' : v]))
  assert.equal(crash.status, 422)
  const t = await crash.text()
  assert.match(t, /Something went wrong/)
  assert.ok(!t.includes('hunter2'))
  assert.ok(logs.some(l => l.msg === 'form failed' && l.stack.includes('/_server/signup.mjs:') && l.valuesCaptured === false))
  assert.ok(!JSON.stringify(logs).includes('hunter2'))
})

test('defenses: cross-origin, no origin signal, wrong encoding, unknown form, POST elsewhere', async () => {
  assert.equal((await post(good, { origin: 'https://evil.test' })).status, 403)
  assert.equal((await post(good, { origin: '' })).status, 403) // neither Origin nor Sec-Fetch-Site
  assert.equal((await post(good, { origin: '', 'sec-fetch-site': 'same-origin' })).status, 303)
  assert.equal((await post('{"email":"x"}', { 'content-type': 'application/json' })).status, 415)
  assert.equal((await post([['_brain', 'zzz/signup']])).status, 400)
  assert.equal((await post([['_brain', 'abc/nope']])).status, 400)
  assert.equal((await fetch(base + '/elsewhere', { method: 'POST', body: 'a=1', headers: { 'content-type': 'application/x-www-form-urlencoded', origin: base } })).status, 405)
})

test('#173 SSR: a failing component renders its fallback inside a 200 page; the error is logged with the request id', async () => {
  const r = await fetch(base + '/broken', { headers: { 'x-request-id': 'req-42' } })
  assert.equal(r.status, 200)
  const h = await r.text()
  assert.match(h, /<p class="ok">still here<\/p>/)
  assert.match(h, /class="brain-fallback">This section could not load\./)
  const l = logs.find(l => l.msg === 'component failed during SSR')
  assert.equal(l.rid, 'req-42')
  assert.equal(l.comp, 'comp/widget.brain')
  assert.match(l.stack, /\/_b\/broken\.js:4:/)
  assert.ok(!l.stack.includes('widget exploded'))
})

test('BUILD: GET of a form page without load serves the prerendered file with the build ETag; POST still goes to the server', async () => {
  const r = await fetch(base + '/signup')
  assert.equal(r.status, 200)
  assert.match(await r.text(), /prerendered form/)
  assert.equal(r.headers.get('etag'), '"f1"')
  assert.equal((await post(good)).status, 303)
})

test('#150 SSR renders in the route locale (t with the page dictionary) and leaves no locale behind', async () => {
  const r = await fetch(base + '/nl/hallo')
  assert.match(await r.text(), /<p class="t">Hallo Ada/)
  assert.equal(globalThis.__brainLocale, null)
})

test('#65 prod: guard deny → 403 page with the reason; redirect → 303; allowed → page with the middleware headers; load error → _error page', async () => {
  const d = await fetch(base + '/admin')
  assert.equal(d.status, 403)
  assert.match(await d.text(), /<p class="p">Admins only/)
  const r = await fetch(base + '/admin?go=1', { redirect: 'manual' })
  assert.equal(r.status, 303)
  assert.equal(r.headers.get('location'), '/login')
  const ok = await fetch(base + '/admin?key=ok')
  assert.equal(ok.status, 200)
  assert.equal(ok.headers.get('x-mw'), 'on')
  assert.match(await ok.text(), /admin area/)
  const e = await fetch(base + '/boom')
  assert.equal(e.status, 500)
  assert.match(await e.text(), /<p class="p">Something went wrong\./)
  assert.ok(logs.some(l => l.msg === 'load failed' && l.stack.includes('/_server/boom.mjs:') && l.private === true))
  assert.ok(!JSON.stringify(logs).includes('load exploded'))
})
