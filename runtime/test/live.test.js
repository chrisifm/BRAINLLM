import test from 'node:test'
import assert from 'node:assert/strict'
import { install } from '../shim/dom.js'
const doc = install()
const { mount, sig, batch, sibling } = await import('./rt.js')
const { liveResource, _liveStats } = await sibling('brain-live.js')

let clock = 100000
class Socket {
  static instances = []
  constructor(url, protocol) {
    this.url = url; this.protocol = protocol; this.readyState = 0; this.bufferedAmount = 0; this.sent = []
    Socket.instances.push(this)
  }
  open() { this.readyState = 1; this.onopen?.({}) }
  message(value) { this.onmessage?.({ data: typeof value === 'string' ? value : JSON.stringify(value) }) }
  remoteClose(code, reason = '') { this.readyState = 3; this.onclose?.({ code, reason }) }
  send(text) { this.sent.push(JSON.parse(text)) }
  close() { this.readyState = 3 }
}
const micro = async () => { for (let n = 0; n < 5; n++) await Promise.resolve() }
async function advance(t, ms = 101) { t.mock.timers.tick(ms); await micro(); t.mock.timers.tick(1); await micro() }
async function fixture(t, options = {}) {
  t.mock.timers.enable({ apis: ['setTimeout', 'setInterval', 'Date'], now: clock += 100000 })
  const previous = { WebSocket: globalThis.WebSocket, location: globalThis.location, query: doc.querySelector }
  globalThis.WebSocket = Socket
  globalThis.location = { protocol: 'https:', origin: 'https://app.example.test', href: 'https://app.example.test/private?token=must-not-travel' }
  doc.querySelector = selector => selector === 'meta[name=brain-base]' && options.base ? { getAttribute: () => options.base } : null
  Socket.instances = []
  const stops = []
  t.after(async () => {
    for (const stop of stops.reverse()) stop()
    await micro()
    assert.deepEqual(_liveStats(), { owners: 0, rooms: 0, pendingJoins: 0, dedupe: 0, connection: 0, timers: 0, refreshes: 0, dirty: 0 })
    globalThis.WebSocket = previous.WebSocket; globalThis.location = previous.location; doc.querySelector = previous.query
  })
  return function own(node, settings) {
    let updates
    const stop = mount(() => {
      updates = liveResource(node, settings)
      return doc.createElement('div')
    }, doc.createElement('div'))
    stops.push(stop)
    return { updates, stop }
  }
}
const settings = scope => ({ scope, rooms: [{ resource: 'posts' }] })
const ready = socket => { socket.open(); socket.message({ type: 'ready', protocol: 1, durable: false, resyncRequired: true }) }
const ack = socket => socket.message({ type: 'resync', reason: 'joined' })
const event = (suffix = '1') => ({ type: 'invalidate', resource: 'posts', id: '00000000-0000-4000-8000-000000000001', operation: 'update', eventId: '00000000-0000-4000-8000-' + suffix.padStart(12, '0') })

test('null scope is idle; the fixed base endpoint contains no credentials, query or custom URL', async t => {
  const own = await fixture(t, { base: '/product/' }), user = sig(null)
  let calls = 0
  const { updates } = own({ refetch() { calls++ }, v: Promise.resolve([]) }, settings(() => user.v))
  await micro(); assert.equal(Socket.instances.length, 0); assert.equal(updates.state, 'idle')
  user.v = 'public-user:1'; await micro()
  const socket = Socket.instances[0]
  assert.equal(socket.url, 'wss://app.example.test/product/_brain/socket')
  assert.equal(socket.protocol, 'brain.v1')
  ready(socket); await advance(t)
  assert.equal(calls, 0, 'READY is not proof that a room was joined')
  assert.deepEqual(socket.sent, [{ type: 'join', resource: 'posts' }])
  ack(socket); await advance(t)
  assert.equal(calls, 1); assert.equal(updates.connected, true); assert.equal(updates.state, 'open')
})

test('owners share one connection and metadata bursts coalesce behind one current resource Promise', async t => {
  const own = await fixture(t)
  let first = 0, second = 0, release
  const never = new Promise(resolve => { release = resolve })
  const a = own({ refetch() { first++ }, v: never }, settings(() => 'one'))
  const b = own({ refetch() { second++ }, v: Promise.resolve([]) }, settings(() => 'one'))
  await micro(); assert.equal(Socket.instances.length, 1)
  const socket = Socket.instances[0]; ready(socket); ack(socket); await advance(t)
  assert.equal(first, 1); assert.equal(second, 1)
  socket.message(event('1')); socket.message(event('2')); socket.message(event('2'))
  await advance(t); assert.equal(first, 1); assert.equal(second, 2)
  release([]); await micro(); await advance(t)
  assert.equal(first, 2, 'Exactly one dirty refresh follows completion')
  a.stop(); assert.equal(socket.readyState, 1)
  b.stop(); assert.equal(socket.readyState, 3)
})

test('conflicting auth scopes fail closed and automatically converge after the reactive batch', async t => {
  const own = await fixture(t), a = sig('old'), b = sig('old')
  const first = own({ refetch() {}, v: Promise.resolve([]) }, settings(() => a.v))
  const second = own({ refetch() {}, v: Promise.resolve([]) }, settings(() => b.v))
  await micro(); const old = Socket.instances[0]; ready(old); ack(old); await advance(t)
  a.v = 'new'; await micro()
  assert.equal(old.readyState, 3); assert.equal(first.updates.state, 'scope-conflict'); assert.equal(second.updates.connected, false)
  b.v = 'new'; await micro(); await advance(t, 250)
  assert.equal(Socket.instances.length, 2)
  const next = Socket.instances[1]; ready(next); ack(next); await advance(t)
  assert.equal(first.updates.connected, true); assert.equal(second.updates.connected, true)
  batch(() => { a.v = 'third'; b.v = 'third' }); await micro(); await advance(t, 250)
  assert.equal(Socket.instances.length, 3, 'A transient old/new scope disagreement is not sticky')
})

test('revoked authentication stops retries until a public revision changes or reconnect is explicit', async t => {
  const own = await fixture(t), user = sig('user:one')
  const { updates } = own({ refetch() {}, v: Promise.resolve([]) }, settings(() => user.v))
  await micro(); const first = Socket.instances[0]; ready(first); ack(first); await advance(t)
  first.remoteClose(1008, 'Authentication expired'); await advance(t, 60000)
  assert.equal(updates.state, 'auth-required'); assert.equal(Socket.instances.length, 1)
  updates.reconnect(); await micro(); await advance(t, 250)
  assert.equal(Socket.instances.length, 2)
  user.v = null; await micro(); assert.equal(Socket.instances[1].readyState, 3)
})

test('unmount with an externally retained pending Promise releases all owners, rooms, timers and handlers', async t => {
  const own = await fixture(t), retained = new Promise(() => {})
  let calls = 0
  const { updates, stop } = own({ refetch() { calls++ }, v: retained }, settings(() => 'user'))
  await micro(); const socket = Socket.instances[0]; ready(socket); ack(socket); await advance(t)
  assert.equal(calls, 1); assert.equal(_liveStats().refreshes, 1)
  socket.message(event()); await advance(t, 11000)
  assert.equal(calls, 1); assert.equal(updates.state, 'refresh-timeout')
  updates.reconnect(); await micro(); await advance(t, 250)
  ready(Socket.instances[1]); ack(Socket.instances[1]); await advance(t)
  assert.equal(calls, 1, 'Reconnect never overlaps a fetch that ignores cancellation')
  stop(); await micro()
  assert.equal(updates.state, 'closed')
  assert.equal(socket.onmessage, null); assert.equal(socket.onclose, null)
  assert.deepEqual(_liveStats(), { owners: 0, rooms: 0, pendingJoins: 0, dedupe: 0, connection: 0, timers: 0, refreshes: 0, dirty: 0 })
})

test('unknown, binary, oversized and prototype-bearing messages never reach callbacks', async t => {
  const own = await fixture(t)
  for (const bad of ['{"type":"invalidate","data":"private"}', '{"type":"pong","__proto__":{}}', ' '.repeat(4097), new Uint8Array([1])]) {
    let calls = 0
    const { updates, stop } = own({ refetch() { calls++ }, v: Promise.resolve([]) }, settings(() => 'user'))
    await micro(); await advance(t, 250)
    const socket = Socket.instances.at(-1); ready(socket); ack(socket); await advance(t)
    assert.equal(calls, 1)
    socket.onmessage({ data: bad }); await advance(t)
    assert.equal(updates.state, 'protocol-error'); assert.equal(calls, 1)
    stop(); await micro()
  }
  assert.equal({}.polluted, undefined)
})

test('the initial connect deadline and finite reconnect budget leave no retry storm', async t => {
  const own = await fixture(t)
  const { updates } = own({ refetch() {}, v: Promise.resolve([]) }, settings(() => 'user'))
  await micro()
  for (let n = 0; n < 70; n++) await advance(t, 1000)
  assert.equal(updates.state, 'exhausted')
  assert.ok(Socket.instances.length <= 6)
})

test('SSR and unsupported browsers create no WebSocket or active owner registration', async t => {
  const own = await fixture(t)
  globalThis.__brainSSR = true
  try {
    const { updates } = own({ refetch() { assert.fail('No SSR refresh') }, v: Promise.resolve([]) }, settings(() => 'user'))
    await micro(); assert.equal(updates.state, 'idle'); assert.equal(Socket.instances.length, 0)
  } finally { delete globalThis.__brainSSR }
  globalThis.WebSocket = undefined
  const { updates } = own({ refetch() {}, v: Promise.resolve([]) }, settings(() => 'user'))
  assert.equal(updates.state, 'unsupported'); assert.equal(_liveStats().owners, 0)
})

test('metadata observers receive frozen fields; the public handle cannot publish messages', async t => {
  const own = await fixture(t), observed = []
  const { updates } = own({ refetch() {}, v: Promise.resolve([]) }, { ...settings(() => 'user'), onResync: value => observed.push(value), onInvalidate: value => observed.push(value) })
  await micro(); const socket = Socket.instances[0]; ready(socket); ack(socket); await advance(t)
  socket.message(event()); await advance(t)
  assert.deepEqual(observed.map(value => value.type), ['resync', 'invalidate'])
  assert.ok(observed.every(Object.isFrozen))
  assert.deepEqual(Object.keys(observed[1]).sort(), ['eventId', 'id', 'operation', 'resource', 'type'])
  assert.equal(updates.send, undefined); assert.equal(updates.publish, undefined)
  assert.ok(socket.sent.every(command => ['join', 'leave', 'ping'].includes(command.type)))
})

test('a forbidden ordered join cannot masquerade as successful subscription resync', async t => {
  const own = await fixture(t)
  let calls = 0
  const { updates } = own({ refetch() { calls++ }, v: Promise.resolve([]) }, settings(() => 'user'))
  await micro(); const socket = Socket.instances[0]; ready(socket)
  socket.message({ type: 'error', code: 'ROOM_FORBIDDEN' }); await advance(t)
  assert.equal(updates.connected, false); assert.equal(updates.state, 'forbidden'); assert.equal(calls, 0)
})

test('invalid handles/options have a fixed actionable code and never echo supplied credentials', async t => {
  await fixture(t)
  const credential = 'synthetic-credential-must-not-appear'
  for (const [node, options] of [[Promise.resolve([]), settings(() => 'user')], [{ refetch() {}, v: null }, { ...settings(() => 'user'), token: credential }]]) {
    assert.throws(() => liveResource(node, options), error => error.code === 'B0918' && !error.message.includes(credential) && typeof error.fix === 'string')
  }
  assert.equal(Socket.instances.length, 0)
})

test('disposed pending resource handles are collectible even when the caller retains the never-settling Promise', { skip: typeof globalThis.gc !== 'function' }, async t => {
  await fixture(t)
  const external = new Promise(() => {})
  function create() {
    let node = { refetch() {}, v: external }, updates
    const reference = new WeakRef(node)
    const stop = mount(() => {
      updates = liveResource(node, settings(() => 'user'))
      return doc.createElement('div')
    }, doc.createElement('div'))
    node = null // The fixture itself must not retain the handle through its mount closure.
    return { reference, stop, updates }
  }
  const owned = create()
  await micro(); const socket = Socket.instances[0]; ready(socket); ack(socket); await advance(t)
  owned.stop(); owned.stop = null; await micro()
  for (let n = 0; n < 12; n++) {
    await new Promise(resolve => setImmediate(resolve))
    globalThis.gc()
    if (!owned.reference.deref()) break
  }
  assert.equal(owned.reference.deref(), undefined)
  assert.equal(owned.updates.state, 'closed')
})
