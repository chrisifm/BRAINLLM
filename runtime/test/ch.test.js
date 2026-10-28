// INTERNALS §12: context, store (with tab/storage sync), bus and exposed API (brain-ch.js).
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { install, MouseEvent } from '../shim/dom.js'

const doc = install()
const rt = await import('./rt.js')
const { sig, fx, tpl, insert, on, when, mount, sibling } = rt
const C = await sibling('brain-ch.js')
const { key, provide, use, scope, withProvide, store, bus, expose, seen } = C
const tick = () => new Promise(r => setTimeout(r, 0))
const click = el => el.dispatchEvent(new MouseEvent('click', { bubbles: true }))

const Theme = key('theme')
const Lang = key('lang', 'es')

test('context: the descendant reads what the ancestor provided, with a reactive signal and key default', async () => {
  const theme = sig('light')
  const seenTheme = []
  function Child() {
    const t = use(Theme)
    const r = tpl('<i><!></i>')()
    insert(r, () => `${t.v}/${use(Lang)}`, r.firstChild)
    return r
  }
  const Parent = scope(() => (provide(Theme, theme), Child()))
  const host = doc.createElement('div')
  mount(Parent, host)
  assert.equal(host.textContent, 'light/es')
  theme.v = 'dark'
  await tick()
  assert.equal(host.textContent, 'dark/es')
})

test('context: scope isolates siblings and use without provide throws B0807', () => {
  const A = scope(() => (provide(Theme, 'only-A'), doc.createTextNode(use(Theme))))
  const B = () => doc.createTextNode(use(Theme))
  const host = doc.createElement('div')
  assert.throws(() => mount(() => [A(), B()], host), e => e.code === 'B0807')
})

test('withProvide replaces a service in tests', () => {
  const Api = key('api')
  const C = () => doc.createTextNode(use(Api).hello())
  const host = doc.createElement('div')
  mount(() => withProvide([[Api, { hello: () => 'fake' }]], () => C()), host)
  assert.equal(host.textContent, 'fake')
})

test('mount(C, el, props, { provide }) replaces services for the whole tree (array of pairs or Map)', () => {
  const Api = key('api2')
  const C = $p => doc.createTextNode($p.n + use(Api).hello() + use(Lang))
  const a = doc.createElement('div'), b = doc.createElement('div')
  const d = mount(C, a, { n: 1 }, { provide: [[Api, { hello: () => 'fake' }]] })
  mount(C, b, { n: 2 }, { provide: new Map([[Api, { hello: () => 'map' }], [Lang, 'en']]) })
  assert.equal(a.textContent, '1fakees')
  assert.equal(b.textContent, '2mapen')
  d()
  assert.equal(a.textContent, '')
  assert.throws(() => mount(C, doc.createElement('div'), { n: 3 }), e => e.code === 'B0807') // nothing leaked
})

test('store: reactive getter/setter per field, batched $set and $snap', async () => {
  const cart = store({ items: [], total: 0 })
  const log = []
  fx(() => log.push(`${cart.items.length}:${cart.total}`))
  cart.$set({ items: ['bread'], total: 12 })
  await tick()
  assert.deepEqual(log, ['0:0', '1:12'])
  assert.deepEqual(cart.$snap(), { items: ['bread'], total: 12 })
  assert.deepEqual(Object.keys(cart), ['items', 'total'])
})

test('store sync storage: loads, persists and applies the storage event from another tab', async () => {
  const mem = new Map([['brain:channels/prefs', '{"dark":true}']])
  globalThis.localStorage = { getItem: k => mem.get(k) ?? null, setItem: (k, v) => mem.set(k, v) }
  const ls = []
  globalThis.addEventListener = (t, f) => t === 'storage' && ls.push(f)
  const prefs = store({ dark: false, size: 1 }, { sync: 'storage' }, 'channels/prefs')
  assert.equal(prefs.dark, true)
  prefs.size = 3
  await tick()
  assert.deepEqual(JSON.parse(mem.get('brain:channels/prefs')), { dark: true, size: 3 })
  ls[0]({ key: 'brain:channels/prefs', newValue: '{"dark":false,"size":3}' })
  assert.equal(prefs.dark, false)
  delete globalThis.addEventListener
})

test('store sync tab: two instances (tabs) sync over BroadcastChannel', async () => {
  // In-process BroadcastChannel: Node's native one does not coexist with the shim's Event
  const chans = new Map()
  globalThis.BroadcastChannel = class {
    constructor(n) {
      this.n = n
      ;(chans.get(n) || chans.set(n, new Set()).get(n)).add(this)
    }
    postMessage(d) {
      for (const c of chans.get(this.n)) if (c !== this) queueMicrotask(() => c.onmessage({ data: structuredClone(d) }))
    }
  }
  const a = store({ n: 0 }, { sync: 'tab' }, 'channels/count')
  a.n = 5
  await tick()
  const b = store({ n: 0 }, { sync: 'tab' }, 'channels/count') // new tab: asks for the state
  await tick()
  assert.equal(b.n, 5)
  b.n = 9
  await tick()
  assert.equal(a.n, 9)
  delete globalThis.BroadcastChannel
})

test('bus: send to subscribers, off on unmount and a broken handler does not stop the others', async () => {
  const toast = bus('toast:show')
  const got = []
  function Listener() {
    toast.on(m => got.push('comp:' + m))
    return doc.createTextNode('')
  }
  const host = doc.createElement('div')
  const stop = mount(Listener, host)
  const off = toast.on(m => {
    throw Error('broken ' + m)
  })
  toast.on(m => got.push('global:' + m))
  const errs = []
  globalThis.reportError = e => errs.push(e.message) // as in the browser: reaches onerror without stopping the send
  toast.send('hello')
  stop()
  off()
  toast.send('bye')
  await tick()
  assert.deepEqual(got, ['comp:hello', 'global:hello', 'global:bye'])
  assert.deepEqual(errs, ['broken hello'])
  delete globalThis.reportError
})

test('expose: the parent receives the frozen API and null when the child unmounts', async () => {
  let m = 'no api'
  const show = sig(true)
  function Modal($p) {
    const open = sig(false)
    const r = tpl('<dialog><!></dialog>')()
    insert(r, () => (open.v ? 'open' : 'closed'), r.firstChild)
    expose($p, { open: () => (open.v = true), close: () => (open.v = false) })
    return r
  }
  const host = doc.createElement('div')
  const T = tpl('<div><!></div>')
  mount(() => {
    const r = T()
    when(r, () => show.v, () => Modal({ 'bind:api': a => (m = a) }), null, r.firstChild)
    return r
  }, host)
  assert.ok(Object.isFrozen(m))
  m.open()
  await tick()
  assert.equal(host.textContent, 'open')
  show.v = false
  await tick()
  assert.equal(m, null)
})

test('seen: every channel use notifies (kind, name, role) for the Graph view', () => {
  const rec = []
  const f = (k, n, r) => rec.push(`${k}:${n}:${r}`)
  seen.add(f)
  const b = bus('analytics')
  b.send(1)
  const s = store({ x: 1 }, {}, 'channels/x')
  s.x
  s.x = 2
  seen.delete(f)
  assert.deepEqual(rec, ['bus:analytics:send', 'store:channels/x:read', 'store:channels/x:write'])
})

test('val: reads a provided sig/drv reactively and returns plain values as is', async () => {
  const { val } = C
  const { drv } = rt
  assert.equal(val(sig(1)), 1)
  assert.equal(val(drv(() => 2)), 2)
  const o = { v: 3 }
  assert.equal(val(o), o)
  assert.equal(val(5), 5)
  const theme = sig('light'), seenV = []
  function Child() {
    const t = drv(((u) => () => val(u))(use(Theme))) // what codegen emits for `let t = use(Theme)`
    fx(() => seenV.push(t.v))
    return doc.createTextNode('')
  }
  mount(scope(() => (provide(Theme, theme), Child())), doc.createElement('div'))
  theme.v = 'dark'
  await tick()
  assert.deepEqual(seenV, ['light', 'dark'])
})

test("store sync 'ws': asks on connect, sends changes, applies remote state without echo, shares one socket", async () => {
  const socks = []
  globalThis.location = new URL('https://app.test/x')
  globalThis.WebSocket = class {
    constructor(u) {
      this.u = u
      this.readyState = 0
      this.out = []
      socks.push(this)
    }
    send(m) {
      this.out.push(JSON.parse(m))
    }
    close() {
      this.readyState = 3
      this.onclose && this.onclose()
    }
    up() {
      this.readyState = 1
      this.onopen()
    }
    in(m) {
      this.onmessage({ data: JSON.stringify(m) })
    }
  }
  const a = store({ n: 0 }, { sync: 'ws', url: '/live' }, 'ca')
  const b = store({ t: '' }, { sync: 'ws', url: '/live' }, 'cb')
  assert.equal(socks.length, 1)
  assert.equal(socks[0].u, 'wss://app.test/live')
  socks[0].up()
  assert.deepEqual(socks[0].out, [{ s: 'brain:ca', q: 1 }, { s: 'brain:cb', q: 1 }])
  socks[0].out = []
  a.n = 5
  await tick()
  assert.deepEqual(socks[0].out, [{ s: 'brain:ca', d: { n: 5 } }])
  socks[0].in({ s: 'brain:cb', d: { t: 'hi' } })
  socks[0].in({ s: 'brain:zz', d: { t: 'other store' } })
  socks[0].in('garbage')
  await tick()
  assert.equal(b.t, 'hi')
  assert.equal(socks[0].out.length, 1) // the remote state is not echoed back
  a.$close()
  b.$close()
  delete globalThis.WebSocket
})

test("store sync 'ws': reconnects with capped exponential backoff, flushes the offline change before asking", async () => {
  const socks = [], delays = []
  const st = globalThis.setTimeout, rnd = Math.random
  globalThis.setTimeout = (f, ms) => (ms && delays.push(Math.round(ms)), st(f, 0))
  Math.random = () => 0.5 // jitter factor 1
  globalThis.WebSocket = class {
    constructor() {
      this.readyState = 0
      this.out = []
      socks.push(this)
    }
    send(m) {
      this.out.push(JSON.parse(m))
    }
    close() {
      this.readyState = 3
      this.onclose && this.onclose()
    }
  }
  const s = store({ n: 0 }, { sync: 'ws', url: 'ws://h.test/r' }, 'rc')
  const fail = k => (socks[k].readyState = 3, socks[k].onclose())
  fail(0)
  await tick()
  fail(1)
  await tick()
  fail(2)
  await tick()
  assert.deepEqual(delays.slice(0, 3), [500, 1000, 2000])
  s.n = 7 // offline change
  await tick()
  const k = socks.length - 1
  socks[k].readyState = 1
  socks[k].onopen()
  assert.deepEqual(socks[k].out, [{ s: 'brain:rc', d: { n: 7 } }, { s: 'brain:rc', q: 1 }])
  fail(k)
  await tick()
  assert.equal(delays.at(-1), 500) // backoff reset after a successful open
  for (let i = 0; i < 12; i++) (socks.at(-1).readyState = 3), socks.at(-1).onclose(), await tick()
  assert.equal(Math.max(...delays), 30000)
  s.$close()
  const before = socks.length
  await tick()
  assert.equal(socks.length, before) // closed: no more reconnects
  globalThis.setTimeout = st
  Math.random = rnd
  delete globalThis.WebSocket
})

test('#169 store inside provide(): one instance per subtree; $reset() restores init; sync effect disposed with it', async () => {
  const Cart = key('cart')
  const seen = []
  function Reader($p) {
    const c = use(Cart)
    const r = tpl('<i><!></i>')()
    insert(r, () => c.items.length, r.firstChild)
    seen.push(c)
    return r
  }
  const Root = scope(() => {
    provide(Cart, store({ items: [] }))
    return [Reader({}), Reader({})]
  })
  const h1 = doc.body.appendChild(doc.createElement('div')), h2 = doc.body.appendChild(doc.createElement('div'))
  const s1 = mount(() => Root({}), h1), s2 = mount(() => Root({}), h2)
  assert.equal(seen[0], seen[1]) // same subtree: same instance
  assert.notEqual(seen[0], seen[2]) // another subtree: its own
  seen[0].items = ['x', 'y']
  await tick()
  assert.equal(h1.textContent, '22')
  assert.equal(h2.textContent, '00')
  seen[0].$reset()
  await tick()
  assert.equal(h1.textContent, '00')
  s1()
  s2()
})

test('#94 store sync: prototype keys from untrusted JSON are dropped; a full localStorage warns and keeps working', async () => {
  const ls = new Map(), warns = []
  const w0 = console.warn
  console.warn = m => warns.push(m)
  globalThis.localStorage = { getItem: k => ls.get(k) ?? null, setItem: (k, v) => { if (v.length > 50) throw Object.assign(new Error('QuotaExceededError'), { name: 'QuotaExceededError' }); ls.set(k, v) } }
  ls.set('brain:prefs94', '{"theme":"dark","__proto__":{"admin":true}}')
  const s = store({ theme: 'light', notes: '' }, { sync: 'storage' }, 'prefs94')
  assert.equal(s.theme, 'dark')
  assert.equal({}.admin, undefined)
  s.notes = 'x'.repeat(80) // too big for the fake quota
  await tick()
  assert.equal(s.notes.length, 80) // still in memory
  assert.match(warns.at(-1), /store prefs94: not saved to localStorage \(QuotaExceededError\)/)
  console.warn = w0
})
