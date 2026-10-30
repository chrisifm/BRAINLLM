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
