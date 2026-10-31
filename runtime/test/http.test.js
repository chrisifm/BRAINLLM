// #168 HTTP client (INTERNALS §40, §41.4): ApiError kinds, safe retries + Retry-After, dedupe, per-origin queue with
// priorities, navigation aborts background requests, offline, .cancel(), refetch after a write, 401 → login, headers,
// upload progress, dev network events.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { install } from '../shim/dom.js'

const { Event: NE, CustomEvent: NCE } = globalThis
install()
Object.assign(globalThis, { Event: NE, CustomEvent: NCE }) // window events here are native (the DOM shim has its own)
const et = new EventTarget()
globalThis.addEventListener = et.addEventListener.bind(et)
globalThis.removeEventListener = et.removeEventListener.bind(et)
globalThis.dispatchEvent = et.dispatchEvent.bind(et)
globalThis.location = Object.assign(new URL('https://app.test/items?x=1'), { assign: u => assigned.push(u) })
const assigned = []
const rt = await import('./rt.js')
const { fx } = rt
const { api, resource, http, online, ApiError } = await rt.sibling('brain-data.js')
const tick = (ms = 0) => new Promise(r => setTimeout(r, ms))

// fake server: `plan` maps "METHOD /path" to a list of answers consumed in order (the last one repeats)
let plan = {}, calls = [], inflight = 0, peak = 0
const res = (status, body = '', headers = {}) => ({ ok: status < 400, status, headers: { get: k => headers[k] ?? null }, text: async () => body })
globalThis.fetch = async (u, init) => {
  const key = init.method + ' ' + new URL(u, 'https://app.test').pathname
  const c = { u, init, key }
  calls.push(c)
  const list = plan[key] || [{ status: 200, body: '{}' }]
  const a = list.length > 1 ? list.shift() : list[0]
  inflight++
  peak = Math.max(peak, inflight)
  try {
    await new Promise((ok, no) => {
      const t = setTimeout(ok, a.delay || 0)
      init.signal.addEventListener('abort', () => (clearTimeout(t), no(init.signal.reason)))
    })
  } finally {
    inflight--
  }
  if (a.throw) throw new TypeError('fetch failed')
  return res(a.status, a.body, a.headers)
}
const reset = p => ((plan = p), (calls = []), (peak = 0))
const pending = async p => {
  let s = 'pending'
  p.then(() => (s = 'resolved'), () => (s = 'rejected'))
  await tick(5)
  return s
}

test('cache isolates API bases and dynamic identities, and respects no-store', async () => {
  http({ base: '/first', headers: null, retry: 0 })
  reset({ 'GET /first/cache-check': [{ status: 200, body: '{"id":1}' }] })
  const endpoint = api('/cache-check', { get: {} })
  await endpoint.get()
  assert.deepEqual(endpoint.get().$stale, { id: 1 })
  http({ base: '/second' })
  const moved = endpoint.get()
  assert.equal(moved.$stale, undefined)
  await moved
  http({ base: '', headers: () => ({ authorization: 'Bearer test' }) })
  reset({ 'GET /cache-check': [{ status: 200, body: '{"id":2}', delay: 5 }] })
  await Promise.all([endpoint.get(), endpoint.get()])
  assert.equal(calls.length, 2)
  const privateCall = endpoint.get()
  assert.equal(privateCall.$stale, undefined)
  await privateCall
  http({ headers: null })
  reset({ 'GET /cache-check': [{ status: 200, body: '{"id":3}', headers: { 'cache-control': 'no-store' } }] })
  await endpoint.get()
  const noStore = endpoint.get()
  assert.equal(noStore.$stale, undefined)
  await noStore
})

test('cache evicts older entries and a late GET cannot restore data invalidated by a write', async () => {
  http({ base: '', headers: null, retry: 0 })
  reset({})
  for (let n = 0; n < 257; n++) await api(`/bounded/${n}`, { get: {} }).get()
  const oldest = api('/bounded/0', { get: {} }).get()
  assert.equal(oldest.$stale, undefined)
  await oldest
  reset({ 'GET /race-cache': [{ status: 200, body: '{"old":true}', delay: 30 }], 'POST /race-cache': [{ status: 200, body: '{}' }] })
  const endpoint = api('/race-cache', { get: {}, post: {} })
  const old = endpoint.get()
  await endpoint.post()
  await old
  const next = endpoint.get()
  assert.equal(next.$stale, undefined)
  await next
})

test('HTTP admission bounds pending work and expires waiters separately from active requests', async () => {
  http({ timeout: 40, retry: 0, headers: null })
  reset({ 'GET /queue-bound': [{ status: 200, body: '{}', delay: 100 }] })
  const endpoint = api('/queue-bound', { get: {} })
  const started = Date.now()
  const results = await Promise.allSettled(Array.from({ length: 140 }, (_, id) => endpoint.get({ id })))
  assert.ok(results.some(r => r.status === 'rejected' && r.reason.status === 503))
  assert.ok(results.some(r => r.status === 'rejected' && r.reason.kind === 'timeout'))
  assert.ok(peak <= 6)
  assert.ok(Date.now() - started < 1000)
  http({ timeout: '10s', retry: 2 })
})

test('ApiError: one typed error per kind, with status, errors, retryAfter, request id and is()', async () => {
  http({ retry: 0 })
  reset({
    'GET /a': [{ status: 404, body: '{"error":"Not found"}', headers: { 'x-request-id': 'rid-1' } }],
    'POST /a': [{ status: 422, body: '{"error":"Invalid input","errors":{"name":"Taken"}}' }],
    'GET /b': [{ throw: true }],
    'GET /slow': [{ status: 200, body: '{}', delay: 80 }],
    'GET /busy': [{ status: 429, body: '', headers: { 'retry-after': '60' } }],
  })
  const a = api('/a', { get: {}, post: {} })
  const e1 = await a.get().catch(e => e)
  assert.ok(e1 instanceof ApiError)
  assert.deepEqual([e1.kind, e1.status, e1.id, e1.message, e1.is(404), e1.is(403)], ['http', 404, 'rid-1', 'Not found', true, false])
  const e2 = await a.post({ name: 'x' }).catch(e => e)
  assert.deepEqual([e2.kind, e2.status, e2.errors], ['http', 422, { name: 'Taken' }])
  assert.equal((await api('/b', { get: {} }).get().catch(e => e)).kind, 'network')
  http({ timeout: 20 })
  assert.equal((await api('/slow', { get: {} }).get().catch(e => e)).kind, 'timeout')
  http({ timeout: '10s' })
  const e3 = await api('/busy', { get: {} }).get().catch(e => e)
  assert.deepEqual([e3.kind, e3.status, e3.retryAfter], ['http', 429, 60])
  const e4 = await api('/a', { get: { out: { id: 'num!' } } }).get().catch(e => e)
  assert.equal(e4.kind, 'http') // the 404 comes first
  reset({ 'GET /s': [{ status: 200, body: '{"id":"x"}' }] })
  globalThis.reportError = () => {}
  assert.equal((await api('/s', { get: { out: { id: 'num!' } } }).get().catch(e => e)).kind, 'schema')
  delete globalThis.reportError
})

test('retries: safe methods on 503/network with backoff, Retry-After honoured; POST never retried', async () => {
  http({ retry: 2 })
  reset({ 'GET /r': [{ status: 503 }, { throw: true }, { status: 200, body: '{"ok":1}' }], 'POST /r': [{ status: 503 }], 'DELETE /r': [{ status: 429, headers: { 'retry-after': '0' } }, { status: 204 }] })
  const r = api('/r', { get: {}, post: {}, delete: {} })
  const t0 = Date.now()
  assert.deepEqual(await r.get(), { ok: 1 })
  assert.equal(calls.length, 3)
  assert.ok(Date.now() - t0 >= 600) // 300 ms then 600 ms (±25 % jitter)
  await assert.rejects(r.post({}), e => e.status === 503)
  assert.equal(calls.filter(c => c.key === 'POST /r').length, 1)
  assert.equal(await r.delete(), null)
  assert.equal(calls.filter(c => c.key === 'DELETE /r').length, 2)
})

test('dedupe: identical GETs in flight share one request; it is aborted only when every caller left', async () => {
  reset({ 'GET /d': [{ status: 200, body: '[1]', delay: 30 }] })
  const d = api('/d', { get: {} })
  const p1 = d.get(), p2 = d.get()
  const r1 = p1.catch(e => e)
  await tick(1)
  assert.equal(calls.length, 1)
  p1.cancel()
  assert.deepEqual(await p2, [1])
  assert.equal((await r1).kind, 'aborted') // a cancelled call rejects with kind 'aborted'
  const p3 = d.get(), p4 = d.get()
  p3.catch(() => {}), p4.catch(() => {})
  p3.cancel()
  p4.cancel()
  await tick(5)
  assert.equal(calls.length, 1) // both left before it was sent: it never went out
  const p5 = d.get(), p6 = d.get()
  p5.catch(() => {}), p6.catch(() => {})
  await tick(1)
  p5.cancel()
  p6.cancel()
  await tick(5)
  assert.equal(calls.at(-1).init.signal.aborted, true) // in flight: aborted once the last caller left
})

test('queue: at most 6 requests per origin; a user action jumps ahead of background ones; navigation aborts background', async () => {
  reset({ 'GET /q': [{ status: 200, body: '1', delay: 30 }], 'POST /act': [{ status: 200, body: '2' }] })
  const order = []
  const all = []
  for (let i = 0; i < 8; i++) all.push(api('/q', { get: {} }).get({ i }, { priority: 'low' }).then(() => order.push('low' + i)))
  const act = api('/act', { post: {} }).post({}).then(() => order.push('act'))
  await tick(5)
  assert.equal(peak, 6)
  await Promise.all([...all, act])
  assert.ok(order.indexOf('act') < order.indexOf('low6'), order.join()) // queued ahead of the waiting background ones
  reset({ 'GET /bg': [{ status: 200, body: '1', delay: 50 }] })
  const bg = api('/bg', { get: {} }).get({}, { prefetch: true }).catch(e => e)
  await tick(5)
  dispatchEvent(new Event('brain:navigate'))
  await tick(5)
  assert.equal(calls[0].init.signal.aborted, true)
  assert.equal((await bg).kind, 'aborted')
})

test('offline: online() follows the events; GETs answer from the cache (stale), writes fail at once', async () => {
  reset({ 'GET /o': [{ status: 200, body: '{"v":1}' }] })
  const o = api('/o', { get: {}, post: {} })
  await o.get()
  dispatchEvent(new Event('offline'))
  assert.equal(online(), false)
  const p = o.get()
  assert.equal(p.stale, true)
  assert.deepEqual(await p, { v: 1 })
  assert.equal((await api('/none', { get: {} }).get().catch(e => e)).kind, 'offline')
  const w = await o.post({}).catch(e => e)
  assert.equal(w.kind, 'offline')
  assert.equal(calls.length, 1) // nothing was sent while offline
  dispatchEvent(new Event('online'))
  assert.equal(online(), true)
})

test('resource: a write to the same path re-runs it; refetch() forces it; unmount cancels', async () => {
  let n = 0
  reset({ 'GET /items': [{ status: 200, body: '[]' }], 'POST /items': [{ status: 200, body: '{}' }] })
  const items = api('/items', { get: {}, post: {} })
  let r
  const stop = fx(() => { r = resource(() => (n++, items.get())) })
  await tick(5)
  assert.equal(n, 1)
  await items.post({ name: 'x' })
  await tick(5)
  assert.equal(n, 2)
  r.refetch()
  await tick(5)
  assert.equal(n, 3)
  stop()
})

test('401 → login?next=<current path> (with the base); http({ headers }) adds state-dependent headers', async () => {
  http({ login: '/login', headers: () => ({ authorization: 'Bearer t1' }) })
  reset({ 'GET /me': [{ status: 401, body: '' }] })
  const e = await api('/me', { get: {} }).get().catch(e => e)
  assert.equal(e.status, 401)
  assert.deepEqual(assigned, ['/login?next=%2Fitems%3Fx%3D1'])
  assert.equal(calls[0].init.headers.authorization, 'Bearer t1')
  http({ login: null, headers: null })
})

test("upload: in: 'file' sends multipart with progress (0–1) and .cancel(); dev emits brain:net events", async () => {
  const sent = []
  globalThis.XMLHttpRequest = class {
    constructor() {
      this.upload = {}
      this.h = {}
    }
    open(m, u) {
      Object.assign(this, { m, u })
    }
    setRequestHeader(k, v) {
      this.h[k] = v
    }
    getResponseHeader() {
      return null
    }
    send(b) {
      sent.push(this)
      this.body = b
      setTimeout(() => this.upload.onprogress({ lengthComputable: true, loaded: 5, total: 10 }), 5)
      setTimeout(() => ((this.status = 200), (this.responseText = '{"ok":true}'), this.onload()), 20)
    }
    abort() {
      this.onabort()
    }
  }
  const net = []
  globalThis.__brainDev = true
  addEventListener('brain:net', e => net.push(e.detail))
  const files = api('/files', { post: { in: 'file' } })
  const up = files.post({ file: new Blob(['abc']), name: 'a.txt' })
  assert.equal(up.progress, 0)
  await tick(10)
  assert.equal(up.progress, 0.5)
  assert.deepEqual(await up, { ok: true })
  assert.equal(up.progress, 1)
  assert.ok(sent[0].body instanceof FormData)
  assert.equal(sent[0].body.get('name'), 'a.txt')
  const f = net.find(x => x.url === '/files')
  assert.deepEqual([f.method, f.status], ['POST', 200])
  const c = files.post({ file: new Blob(['x']) })
  c.cancel()
  assert.equal((await c.catch(e => e)).kind, 'aborted')
  delete globalThis.XMLHttpRequest
  delete globalThis.__brainDev
})

test(".cancel() rejects with kind 'aborted', so the author's finally runs (busy is reset)", async () => {
  reset({ 'POST /save': [{ status: 200, body: '{}', delay: 50 }] })
  let busy = false, caught = null
  const save = api('/save', { post: {} })
  const p = save.post({})
  const run = (async () => {
    busy = true
    try {
      await p
    } catch (e) {
      caught = e
    } finally {
      busy = false
    }
  })()
  await tick(5)
  assert.equal(busy, true)
  p.cancel()
  await run
  assert.equal(busy, false)
  assert.ok(caught instanceof ApiError)
  assert.equal(caught.kind, 'aborted')
  assert.equal(calls[0].init.signal.aborted, true)
})

test("resource(): a cancelled run (re-run) never reaches {:catch}", async t => {
  reset({ 'GET /slow2': [{ status: 200, body: '"ok"', delay: 30 }] })
  const s = api('/slow2', { get: {} })
  const k = rt.sig(0)
  let r
  const stop = fx(() => { r = resource(() => (k.v, s.get({ k: k.v }))) })
  t.after(stop)
  const first = r.v
  let state = 'pending'
  first.then(() => (state = 'resolved'), () => (state = 'rejected'))
  await tick(5)
  k.v = 1 // re-run: the first request is cancelled
  await tick(60)
  assert.equal(state, 'pending')
  assert.equal(await r.v, 'ok')
})

test('stream(): SSE as a read-only node (.v, .connected), JSON or text, reduce, closed on unmount', async () => {
  const opened = []
  globalThis.EventSource = class {
    constructor(u) {
      this.u = u
      this.l = {}
      this.closed = false
      opened.push(this)
    }
    addEventListener(t, f) {
      this.l[t] = f
    }
    close() {
      this.closed = true
    }
  }
  const { stream } = await rt.sibling('brain-data.js')
  let s, all
  const stop = rt.fx(() => {
    s = stream('/events')
    all = stream('/events', { event: 'tick', initial: [], reduce: (a, m) => [...a, m] })
  })
  const [es, es2] = opened
  assert.equal(es.u, '/events')
  assert.equal(s.connected, false)
  es.onopen()
  assert.equal(s.connected, true)
  es.l.message({ data: '{"n":1}' })
  assert.deepEqual(s.v, { n: 1 })
  es.l.message({ data: 'plain' })
  assert.equal(s.v, 'plain')
  es2.l.tick({ data: '1' })
  es2.l.tick({ data: '2' })
  assert.deepEqual(all.v, [1, 2])
  es.onerror()
  assert.equal(s.connected, false) // EventSource reconnects by itself
  stop()
  assert.ok(es.closed && es2.closed)
  delete globalThis.EventSource
})

test('optimistic: applied at once, rolled back (in one batch) when the request fails; kept when it succeeds', async () => {
  http({ retry: 0 })
  reset({ 'POST /todos': [{ status: 500, body: '{"error":"boom"}' }, { status: 200, body: '{}' }] })
  const todos = api('/todos', { post: {} })
  const list = rt.sig(['a']), count = rt.sig(1)
  const p = todos.post({ t: 'b' }, { optimistic: () => ((list.v = [...list.v, 'b']), (count.v = 2)) })
  assert.deepEqual(list.v, ['a', 'b'])
  assert.equal((await p.catch(e => e)).status, 500)
  assert.deepEqual([list.v, count.v], [['a'], 1])
  await todos.post({ t: 'c' }, { optimistic: () => (list.v = [...list.v, 'c']) })
  assert.deepEqual(list.v, ['a', 'c'])
  http({ retry: 2 })
})

test('optimistic recording leaves the signal setter as it was, also when the update throws (#288)', async () => {
  const P = rt.sig(0).constructor.prototype, before = Object.getOwnPropertyDescriptor(P, 'v')
  reset({ 'POST /todos': [{ status: 200, body: '{}' }] })
  const todos = api('/todos', { post: {} })
  const s = rt.sig(1)
  await todos.post({}, { optimistic: () => (s.v = 2) })
  assert.throws(() => todos.post({}, { optimistic: () => ((s.v = 3), rt.drv(() => 0).v = 1) }), /read-only/)
  assert.deepEqual(Object.getOwnPropertyDescriptor(P, 'v'), before)
  s.v = 4
  assert.equal(s.v, 4)
})

test('resource page: true → { page: n }, more() appends, done on a short page; every polls while visible and online', async () => {
  const pages = { 1: '[1,2]', 2: '[3,4]', 3: '[5]' }
  reset({})
  plan = {}
  const seen = []
  const posts = { get: a => (seen.push(a.page), Promise.resolve(JSON.parse(pages[a.page]))) }
  const r = resource(posts.get, { page: true })
  assert.deepEqual(await r.v, [1, 2])
  assert.equal(r.done, false)
  await r.more()
  assert.deepEqual(await r.v, [1, 2, 3, 4])
  await r.more()
  assert.deepEqual(await r.v, [1, 2, 3, 4, 5])
  assert.equal(r.done, true) // a short page
  await r.more() // nothing more to ask
  assert.deepEqual(seen, [1, 2, 3])
  let n = 0
  const stop = rt.fx(() => resource(() => (n++, Promise.resolve(n)), { every: '20ms' }))
  await tick(70)
  assert.ok(n >= 3, String(n))
  dispatchEvent(new Event('offline'))
  const paused = n
  await tick(50)
  assert.equal(n, paused) // offline: no polling
  dispatchEvent(new Event('online'))
  stop()
  await tick(50)
  assert.equal(n, paused) // gone with its owner
})

test('a call made from an event handler is cancelled when its component unmounts (brain-guard $c)', async () => {
  const { guard } = await rt.sibling('brain-guard.js')
  reset({ 'POST /slow3': [{ status: 200, body: '{}', delay: 50 }] })
  const slow = api('/slow3', { post: {} })
  let p
  const C = guard(() => {
    const r = rt.tpl('<button class="go">go</button>')()
    rt.on(r, 'click', () => (p = slow.post({})))
    return r
  })
  const h = document.body.appendChild(document.createElement('div'))
  const stop = rt.mount(() => C({}), h)
  const btn = h.querySelector('.go')
  globalThis.event = { target: btn } // what the browser exposes while a handler runs
  p = slow.post({})
  delete globalThis.event
  const r = p.catch(e => e)
  await tick(5)
  stop()
  assert.equal((await r).kind, 'aborted')
  assert.equal(calls[0].init.signal.aborted, true)
})

test('requests from an event handler leave the component registry once they settle', async () => {
  const { guard } = await rt.sibling('brain-guard.js')
  reset({ 'POST /done4': [{ status: 200, body: '{}' }], 'POST /fail4': [{ status: 500, body: '{}' }] })
  const done = api('/done4', { post: {} }), bad = api('/fail4', { post: {} })
  const C = guard(() => rt.tpl('<button class="go4">go</button>')())
  const h = document.body.appendChild(document.createElement('div'))
  const stop = rt.mount(() => C({}), h)
  const btn = h.querySelector('.go4')
  const all = []
  for (let i = 0; i < 20; i++) {
    globalThis.event = { target: btn }
    all.push(done.post({}), bad.post({}).catch(() => {}))
    delete globalThis.event
  }
  await Promise.all(all)
  await tick()
  assert.equal(btn.$c.size, 0, 'A long-lived component must not keep finished requests')
  stop()
})

test('native backend errors retain actionable codes and fields without rendering object strings or server traces',async()=>{
  http({retry:0})
  reset({
    'POST /native':[{status:422,body:JSON.stringify({schemaVersion:1,error:{code:'INVALID_INPUT',fix:'Review the declared fields',fields:{email:'Use a valid email'}}}),headers:{'x-request-id':'native-reference'}}],
    'GET /private':[{status:500,body:JSON.stringify({error:'Error: private driver stack with account values'})}],
    'GET /private-client':[{status:400,body:JSON.stringify({error:'TypeError: private module path'})}],
  })
  const error=await api('/native',{post:{}}).post({}).catch(error=>error)
  assert.equal(error.message,'Check the highlighted fields.');assert.equal(error.backendCode,'INVALID_INPUT')
  assert.deepEqual(error.errors,{email:'Use a valid email'});assert.equal(error.id,'native-reference');assert.equal(error.fix,'Review the declared fields')
  for(const path of ['/private','/private-client']){
    const failure=await api(path,{get:{}}).get().catch(error=>error)
    assert.ok(!failure.message.includes('private')&&!failure.message.includes('TypeError'))
  }
})
