// callRpc / parseArgs / callForm (server.mjs, TOOLCHAIN #188): the same code answers RPC and form posts in `brain run`
// (its Node sidecar imports these) and in `node dist/server.mjs`.
import { test } from 'node:test'
import assert from 'node:assert/strict'

const { callRpc, parseArgs, callForm } = await import('../server/server.mjs')
const rules = await import('../brain-schema.js')
const ctx = { url: new URL('http://x.test/signup?a=1'), user: null }

test('parseArgs: JSON array only, prototype keys dropped', () => {
  assert.deepEqual(parseArgs('[1,{"a":2}]'), [1, { a: 2 }])
  assert.deepEqual(parseArgs(''), [])
  const [o] = parseArgs('[{"__proto__":{"x":1},"constructor":{"p":1},"ok":1}]')
  assert.deepEqual(Object.keys(o), ['ok'])
  assert.throws(() => parseArgs('{"a":1}'), /JSON array/)
  assert.throws(() => parseArgs('{nope'), /not JSON/)
})

test('callRpc: 200, 422 {errors}, 4xx HttpError, 504 and 500 with the real error only in `cause`, 429 by key', async () => {
  assert.deepEqual(await callRpc({ fn: async (c, a, b) => a + b, args: '[2,3]', ctx }), { status: 200, body: 5 })
  assert.deepEqual(await callRpc({ fn: async () => undefined, args: [], ctx }), { status: 200, body: null })
  const v = await callRpc({ fn: async () => { throw Object.assign(new Error('Invalid input'), { errors: { email: 'Taken' } }) }, args: [], ctx })
  assert.deepEqual(v, { status: 422, body: { error: 'Invalid input', errors: { email: 'Taken' } } })
  const t = await callRpc({ fn: () => new Promise(() => {}), args: [], ctx, timeout: 30 })
  assert.equal(t.status, 504)
  const e = await callRpc({ fn: async () => { throw new Error('password=hunter2') }, args: [], ctx })
  assert.deepEqual(e.body, { error: 'Internal error' })
  assert.match(e.cause.message, /hunter2/)
  assert.equal((await callRpc({ fn: null, args: [], ctx })).status, 404)
  assert.equal((await callRpc({ fn: async () => 1, args: '{bad', ctx })).status, 400)
  const codes = []
  for (let i = 0; i < 4; i++) codes.push((await callRpc({ fn: async () => 1, args: [], ctx, key: 'ip9 h/f', limit: 2 })).status)
  assert.deepEqual(codes, [200, 200, 429, 429])
})

test('callForm: validated with the schema → 303 (PRG or redirect), 422 state with values and messages, 400 unknown', async () => {
  let got
  const module = {
    $forms: { save: { email: 'email!', age: 'num>=18' } },
    save: async (c, v) => ((got = v), v.email === 'go@x.dev' ? { redirect: '/done' } : 'ok'),
  }
  const ok = await callForm({ module, raw: '_brain=h%2Fsave&email=a%40x.dev&age=20&role=admin', ctx, rules })
  assert.deepEqual(ok, { status: 303, location: '/signup?a=1' })
  assert.deepEqual(got, { email: 'a@x.dev', age: 20 })
  assert.equal((await callForm({ module, raw: '_brain=h%2Fsave&email=go%40x.dev&age=20', ctx, rules })).location, '/done')
  const bad = await callForm({ module, raw: '_brain=h%2Fsave&email=a%40&age=16', ctx, rules })
  assert.equal(bad.status, 422)
  assert.deepEqual(bad.state['h/save'].errors, { email: 'Enter a valid email', age: 'Must be at least 18' })
  assert.equal(bad.state['h/save'].values.email, 'a@')
  assert.equal((await callForm({ module, raw: '_brain=h%2Fnope', ctx, rules })).status, 400)
})

test('callForm: a form posted without JS is validated in the page locale (§37)', async () => {
  const before = globalThis.__brainDict
  // Tagged English stand-ins for a second locale (the repository is English only).
  globalThis.__brainDict = { es: { 'form.email': 'Bad email [es]', 'form.error': 'Failed [es]' } }
  try {
    const module = { $forms: { save: { email: 'email!' } }, save: async () => { throw new Error('db down') } }
    const bad = await callForm({ module, raw: '_brain=h%2Fsave&email=a%40', ctx, rules, locale: 'es' })
    assert.equal(bad.state['h/save'].errors.email, 'Bad email [es]')
    const failed = await callForm({ module, raw: '_brain=h%2Fsave&email=a%40x.dev', ctx, rules, locale: 'es' })
    assert.equal(failed.state['h/save'].error, 'Failed [es]')
    assert.equal(globalThis.__brainLocale, undefined, 'the locale does not leak past the call')
    const en = await callForm({ module, raw: '_brain=h%2Fsave&email=a%40', ctx, rules })
    assert.equal(en.state['h/save'].errors.email, 'Enter a valid email')
  } finally {
    globalThis.__brainDict = before
  }
})

test('runChain: guards outer→inner (true/false/redirect/deny, returned or thrown), middleware onion with headers; open redirect refused', async () => {
  const { runChain, deny, redirect, sessionFrom } = await import('../server/server.mjs')
  const seen = []
  const g = (name, v) => ctx => (seen.push(name), typeof v === 'function' ? v(ctx) : v)
  assert.deepEqual(await runChain({ guards: [g('outer', true), g('inner', undefined)], ctx: {} }), { ok: true, headers: {} })
  assert.deepEqual(seen, ['outer', 'inner'])
  assert.deepEqual(await runChain({ guards: [g('a', false), g('b', true)], ctx: {} }), { deny: 'Forbidden' })
  assert.deepEqual(await runChain({ guards: [() => deny('Admins only')], ctx: {} }), { deny: 'Admins only' })
  assert.deepEqual(await runChain({ guards: [() => redirect('/login?next=%2Fadmin')], ctx: {} }), { redirect: '/login?next=%2Fadmin', status: 303 })
  assert.deepEqual(await runChain({ guards: [() => { throw deny('nope') }], ctx: {} }), { deny: 'nope' })
  assert.deepEqual(await runChain({ guards: [() => { throw { redirect: '/x', status: 302 } }], ctx: {} }), { redirect: '/x', status: 302 })
  await assert.rejects(runChain({ guards: [() => ({ redirect: 'https://evil.test' })], ctx: {} }), /another origin/)
  await assert.rejects(runChain({ guards: [() => { throw new Error('db down') }], ctx: {} }), /db down/)
  const order = []
  const r = await runChain({
    middleware: [
      async (ctx, next) => (order.push('a>'), (ctx.headers['x-a'] = '1'), await next(), order.push('<a')),
      async (ctx, next) => (order.push('b'), await next()),
    ],
    ctx: {},
  })
  assert.deepEqual(r, { ok: true, headers: { 'x-a': '1' } })
  assert.deepEqual(order, ['a>', 'b', '<a'])
  assert.deepEqual(await runChain({ middleware: [async () => deny('maintenance')], ctx: {} }), { deny: 'maintenance' })
  // sessionFrom: what set() writes, a later request reads; another secret or a forged cookie reads nothing
  const s1 = sessionFrom({}, 'k1')
  s1.session.set({ name: 'Ada' })
  const v = s1.session.headers()['set-cookie'][0].split(';')[0].split('=')[1]
  assert.deepEqual(sessionFrom({ '__Host-session': v }, 'k1').user, { name: 'Ada' })
  assert.equal(sessionFrom({ '__Host-session': v }, 'other').user, null)
  assert.equal(sessionFrom({ '__Host-session': v.slice(0, -2) + 'xx' }, 'k1').user, null)
})

test('plain deliberate client errors preserve HTTP status without exposing their messages',async()=>{
  for(const status of [400,401,403,404,409,422,429]) {
    const response=await callRpc({fn:async()=>{throw {status,message:'private values and driver detail'}},args:[],ctx})
    assert.equal(response.status,status);assert.ok(!JSON.stringify(response.body).includes('private'))
  }
  for(const value of [{status:200},{status:599},Object.assign(new Error('private'),{status:403})]) {
    const response=await callRpc({fn:async()=>{throw value},args:[],ctx})
    assert.equal(response.status,500)
  }
})

test('session secrets: encrypted cookie, bound to the key, never a user, cleared with the session', async () => {
  const { sessionFrom, sessionSecrets } = await import('../server/server.mjs')
  const value = 'synthetic-provider-token-value'
  const first = sessionFrom({}, 'k1')
  first.session.setSecret({ token: value }, { maxAge: 60 })
  const header = first.session.headers()['set-cookie'].find(c => c.startsWith('__Host-session-secret='))
  assert.match(header, /HttpOnly; Secure; SameSite=Lax; Max-Age=60/)
  const cookie = header.split(';')[0].slice('__Host-session-secret='.length)
  assert.ok(!Buffer.from(cookie, 'base64url').toString('latin1').includes(value), 'the value is not readable in the cookie')
  const later = sessionFrom({ '__Host-session-secret': cookie }, 'k1')
  assert.deepEqual(later.session.secret(), { token: value })
  assert.equal(later.user, null, 'a secret never authenticates')
  assert.equal(sessionFrom({ '__Host-session-secret': cookie }, 'other').session.secret(), null)
  assert.equal(sessionFrom({ '__Host-session-secret': cookie.slice(0, -2) + 'xx' }, 'k1').session.secret(), null)
  // the signed session cookie cannot be replayed as a secret, or the reverse
  first.session.set({ name: 'Ada' })
  const signed = first.session.headers()['set-cookie'].find(c => c.startsWith('__Host-session=')).split(';')[0].slice('__Host-session='.length)
  assert.equal(sessionFrom({ '__Host-session-secret': signed }, 'k1').session.secret(), null)
  assert.equal(sessionFrom({ '__Host-session': cookie }, 'k1').user, null)
  // expiry travels inside the ciphertext
  const expired = sessionFrom({}, 'k1')
  expired.session.setSecret('x', { maxAge: -1 })
  const stale = expired.session.headers()['set-cookie'][0].split(';')[0].slice('__Host-session-secret='.length)
  assert.equal(sessionFrom({ '__Host-session-secret': stale }, 'k1').session.secret(), null)
  // clearing
  later.session.clearSecret()
  assert.match(later.session.headers()['set-cookie'][0], /^__Host-session-secret=; .*Max-Age=0/)
  assert.equal(later.session.secret(), null)
  const out = sessionFrom({ '__Host-session-secret': cookie }, 'k1')
  out.session.clear()
  assert.ok(out.session.headers()['set-cookie'].some(c => /^__Host-session-secret=; .*Max-Age=0/.test(c)), 'logout clears the secret')
  // limits and configuration fail loudly instead of truncating
  assert.throws(() => sessionFrom({}, 'k1').session.setSecret('x'.repeat(4000)), /too large/)
  assert.throws(() => sessionFrom({}, '').session.setSecret('x'), /BRAIN_SECRET/)
  assert.throws(() => sessionFrom({}, 'k1').session.setSecret(undefined), /JSON/)
  // a session from another handle (the native bridge) gains the same API and keeps its headers
  const native = sessionSecrets({ user: { id: 'u' }, set() { throw new Error('native') }, clear() { return 'done' }, headers: () => ({ 'set-cookie': ['a=1'], 'cache-control': 'no-store' }) }, {}, 'k1')
  native.setSecret('x')
  const h = native.headers()
  assert.equal(h['cache-control'], 'no-store')
  assert.equal(h['set-cookie'][0], 'a=1')
  assert.match(h['set-cookie'][1], /^__Host-session-secret=/)
  assert.deepEqual(native.user, { id: 'u' })
  assert.equal(native.clear(), 'done')
  assert.ok(native.headers()['set-cookie'].some(c => /^__Host-session-secret=; .*Max-Age=0/.test(c)))
})

test('session rotation, version, revocation and configured length', async () => {
  const { sessionFrom } = await import('../server/server.mjs')
  const saved = { p: process.env.BRAIN_SECRET_PREVIOUS, v: process.env.BRAIN_SESSION_VERSION, s: process.env.BRAIN_SESSION_SECONDS }
  try {
    const old = sessionFrom({}, 'old-key'); old.session.set({ name: 'Ada' }); old.session.setSecret('kept')
    const cookies = Object.fromEntries(old.session.headers()['set-cookie'].map(c => { const p = c.split(';')[0], i = p.indexOf('='); return [p.slice(0, i), p.slice(i + 1)] }))
    assert.equal(sessionFrom(cookies, 'new-key').user, null)
    process.env.BRAIN_SECRET_PREVIOUS = 'other, old-key'
    const rotated = sessionFrom(cookies, 'new-key')
    assert.deepEqual(rotated.user, { name: 'Ada' }, 'a previous key still verifies')
    assert.equal(rotated.session.secret(), 'kept', 'a previous key still decrypts')
    rotated.session.set({ name: 'Ada' })
    delete process.env.BRAIN_SECRET_PREVIOUS
    const fresh = rotated.session.headers()['set-cookie'][0].split(';')[0].slice('__Host-session='.length)
    assert.deepEqual(sessionFrom({ '__Host-session': fresh }, 'new-key').user, { name: 'Ada' }, 'new cookies use the current key')

    // bumping the version signs everybody out
    const v = sessionFrom({}, 'k'); v.session.set({ name: 'Ada' })
    const vc = v.session.headers()['set-cookie'][0].split(';')[0].slice('__Host-session='.length)
    assert.deepEqual(sessionFrom({ '__Host-session': vc }, 'k').user, { name: 'Ada' })
    process.env.BRAIN_SESSION_VERSION = '2'
    assert.equal(sessionFrom({ '__Host-session': vc }, 'k').user, null)
    const v2 = sessionFrom({}, 'k'); v2.session.set({ name: 'Ada' })
    const vc2 = v2.session.headers()['set-cookie'][0].split(';')[0].slice('__Host-session='.length)
    assert.deepEqual(sessionFrom({ '__Host-session': vc2 }, 'k').user, { name: 'Ada' })
    delete process.env.BRAIN_SESSION_VERSION

    // logout invalidates a copy of the cookie in this process
    const a = sessionFrom({}, 'k'); a.session.set({ name: 'Ada' })
    const copy = a.session.headers()['set-cookie'][0].split(';')[0].slice('__Host-session='.length)
    sessionFrom({ '__Host-session': copy }, 'k').session.clear()
    assert.equal(sessionFrom({ '__Host-session': copy }, 'k').user, null)

    // default length
    process.env.BRAIN_SESSION_SECONDS = '3600'
    const l = sessionFrom({}, 'k'); l.session.set({ name: 'Ada' })
    assert.match(l.session.headers()['set-cookie'][0], /Max-Age=3600/)
    process.env.BRAIN_SESSION_SECONDS = 'nonsense'
    const d = sessionFrom({}, 'k'); d.session.set({ name: 'Ada' })
    assert.match(d.session.headers()['set-cookie'][0], new RegExp(`Max-Age=${7 * 24 * 3600}`))
  } finally {
    for (const [k, n] of [['p', 'BRAIN_SECRET_PREVIOUS'], ['v', 'BRAIN_SESSION_VERSION'], ['s', 'BRAIN_SESSION_SECONDS']]) saved[k] === undefined ? delete process.env[n] : process.env[n] = saved[k]
  }
})
