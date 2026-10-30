// Native, same-origin invalidation metadata. HTTP resource reads remain authoritative.
import { sig, fx, untrack, _rt } from './brain.js'

const G = globalThis, I = _rt()
const MAX_OWNERS = 128, MAX_ROOMS = 32, FRAME = 4096, BUFFER = 32768
const COALESCE = 100, READY_MS = 8000, REFRESH_MS = 10000, RETRY_MS = 60000
const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i
const own = (value, key) => Object.prototype.hasOwnProperty.call(value, key)
const RESERVED = new Set(['constructor', 'prototype', '__proto__', 'id', 'owner', 'created_at', 'updated_at', 'search'])
let broker
const fault = fix => Object.assign(Error('B0918: ' + fix), { code: 'B0918', fix })
const fail = fix => { throw fault(fix) }
const exact = (value, keys) => value && typeof value === 'object' && !Array.isArray(value) && Object.keys(value).every(key => keys.includes(key))
const resourceName = name => typeof name === 'string' && /^[a-z][a-z0-9_]{0,47}$/.test(name) && !RESERVED.has(name)

function roomsOf(value) {
  if (!Array.isArray(value) || value.length > MAX_ROOMS) fail('Use at most 32 native resource rooms')
  const rooms = new Map()
  for (const room of value) {
    let command, key
    const id = room && typeof room === 'object' && own(room, 'id') ? room.id : undefined
    if (exact(room, ['room']) && own(room, 'room') && room.room === 'user:self') {
      command = { room: 'user:self' }; key = 'user:self'
    } else if (exact(room, ['resource', 'id']) && own(room, 'resource') && resourceName(room.resource) && (id === undefined || (typeof id === 'string' && UUID.test(id)))) {
      command = id === undefined ? { resource: room.resource } : { resource: room.resource, id: id.toLowerCase() }
      key = command.resource + ':' + (command.id || '')
    } else fail('Use a declared resource, an optional UUID, or the user:self room')
    rooms.set(key, Object.freeze(command))
  }
  return rooms
}

function endpoint() {
  const location = G.location
  if (!location || !/^https?:$/.test(location.protocol)) fail('Serve live resources from an HTTP(S) application origin')
  const meta = G.document && document.querySelector('meta[name=brain-base]')
  const base = meta && meta.getAttribute('content') || ''
  if (typeof base !== 'string' || base.length > 512 || (base && (!base.startsWith('/') || base.startsWith('//') || /[\\?#\x00-\x20\x7f]/.test(base)))) fail('Use a normalized same-origin application base path')
  const origin = new URL(location.origin)
  if (origin.origin !== location.origin || origin.username || origin.password) fail('Use a canonical application origin')
  const normalized = base.replace(/\/$/, '')
  const url = new URL(normalized + '/_brain/socket', origin)
  if (url.origin !== origin.origin || url.pathname !== normalized + '/_brain/socket') fail('Use a normalized same-origin application base path')
  url.protocol = origin.protocol === 'https:' ? 'wss:' : 'ws:'
  return url.href
}

function metadata(text) {
  if (typeof text !== 'string' || text.length > FRAME || /[^\x09\x0a\x0d\x20-\x7e]/.test(text)) return null
  let value
  try {
    value = JSON.parse(text, (key, item) => {
      if (key === '__proto__' || key === 'constructor' || key === 'prototype') throw Error('Unsafe metadata')
      return item
    })
  } catch { return null }
  if (exact(value, ['type', 'protocol', 'durable', 'resyncRequired']) && value.type === 'ready' && value.protocol === 1 && value.durable === false && value.resyncRequired === true) return Object.freeze(value)
  if (exact(value, ['type', 'reason']) && value.type === 'resync' && value.reason === 'joined') return Object.freeze(value)
  if (exact(value, ['type']) && value.type === 'pong') return Object.freeze(value)
  if (exact(value, ['type', 'code']) && value.type === 'error' && value.code === 'ROOM_FORBIDDEN') return Object.freeze(value)
  if (exact(value, ['type', 'resource', 'id', 'operation', 'eventId']) && value.type === 'invalidate' && resourceName(value.resource) && typeof value.id === 'string' && UUID.test(value.id) && typeof value.eventId === 'string' && UUID.test(value.eventId) && ['create', 'update', 'delete'].includes(value.operation)) return Object.freeze(value)
  return null
}

function createBroker() {
  const members = new Set(), joined = new Map(), waiting = [], events = new Map()
  let socket = null, epoch = 0, scope = null, terminal = '', ready = false
  let reconcileQueued = false, retryTimer, readyTimer, heartbeat, pongTimer, flushTimer
  let attempts = 0, retryStart = 0, lastConnect = 0

  const update = () => {
    for (const member of members) {
      const active = [...member.rooms.keys()].some(key => joined.get(key) === 'ready')
      const forbidden = member.rooms.size > 0 && [...member.rooms.keys()].every(key => joined.get(key) === 'forbidden')
      member.up.v = !!(socket && ready && member.scope && member.scope === scope && active)
      member.state.v = !member.scope || !member.rooms.size ? 'idle' : terminal || (member.refreshState || (forbidden ? 'forbidden' : member.up.v ? 'open' : retryTimer ? 'retrying' : 'connecting'))
    }
  }
  const clearMember = member => {
    member.generation++
    member.dirty = false
    member.pending = member.resync = null
    member.refreshState = member.flight ? 'refresh-pending' : ''
    if (member.flight) {
      const flight = member.flight
      clearTimeout(flight.deadline); clearTimeout(flight.tick)
      flight.deadline = flight.tick = null
      if (flight.resolveTick) { flight.resolveTick(); flight.resolveTick = null }
    }
  }
  const stop = () => {
    epoch++; ready = false
    clearTimeout(retryTimer); retryTimer = null
    clearTimeout(readyTimer); clearInterval(heartbeat); clearTimeout(pongTimer)
    readyTimer = heartbeat = pongTimer = null
    clearTimeout(flushTimer); flushTimer = null
    joined.clear(); waiting.length = 0; events.clear(); currentRooms.clear()
    const previous = socket; socket = null
    if (previous) {
      previous.onopen = previous.onmessage = previous.onclose = previous.onerror = null
      try { previous.close(1000, 'Live resource lifecycle ended') } catch {}
    }
    for (const member of members) { clearMember(member); member.up.v = false }
  }
  const reject = state => { stop(); terminal = state; update() }
  const send = value => {
    if (!socket || socket.readyState !== 1 || socket.bufferedAmount > BUFFER) return false
    const text = JSON.stringify(value)
    if (text.length > FRAME) return false
    try { socket.send(text); return true } catch { return false }
  }
  const refresh = member => {
    if (!members.has(member) || !member.up.v || !member.dirty || member.flight) return
    member.dirty = false; member.refreshState = ''; update()
    const pending = member.pending, resync = member.resync
    member.pending = member.resync = null
    const flight = { generation: member.generation, epoch, deadline: null, tick: null, resolveTick: null }
    member.flight = flight
    flight.deadline = setTimeout(timeoutFlight.bind(null, member.cell, flight), REFRESH_MS)
    try {
      if (resync && own(member.options, 'onResync') && member.options.onResync) observeMetadata(member.options.onResync, resync)
      if (pending && own(member.options, 'onInvalidate') && member.options.onInvalidate) observeMetadata(member.options.onInvalidate, pending)
      untrack(() => member.node.refetch())
      // Equivalent post-effect tick, with an owned cancellation handle. The
      // standard tick() intentionally exposes no handle for owner cancellation.
      const next = new Promise(resolve => {
        flight.resolveTick = resolve
        flight.tick = setTimeout(resolve, 0)
      })
      next.then(readResource.bind(null, member.cell, flight)).then(
        settleFlight.bind(null, member.cell, flight, false),
        settleFlight.bind(null, member.cell, flight, true),
      )
    } catch {
      I.oops(fault('Keep live metadata observers synchronous and handle resource failures in the view'), member.owner, 'B0918')
      dispose(member.cell)
    }
  }
  const valid = (member, flight) => members.has(member) && member.generation === flight.generation && epoch === flight.epoch
  function complete(member, flight, failed) {
    if (member.flight !== flight) return
    clearTimeout(flight.deadline); clearTimeout(flight.tick)
    flight.deadline = flight.tick = flight.resolveTick = null
    member.flight = null; member.refreshState = ''
    if (valid(member, flight) && failed) member.refreshState = 'refresh-error'
    if (members.has(member) && member.dirty) schedule()
    if (members.has(member)) update()
  }
  function schedule() {
    if (!flushTimer) flushTimer = setTimeout(() => {
      flushTimer = null
      for (const member of members) refresh(member)
    }, COALESCE)
  }
  const invalidate = (value, roomKey) => {
    for (const member of members) {
      if (!member.scope || member.scope !== scope) continue
      const interested = roomKey ? member.rooms.has(roomKey) : [...member.rooms].some(([key, room]) => joined.get(key) === 'ready' && (room.room === 'user:self' || (room.resource === value.resource && (!room.id || room.id === value.id))))
      if (interested) { member.dirty = true; if (value.type === 'resync') member.resync = value; else member.pending = value; schedule() }
    }
  }
  const retry = () => {
    stop()
    if (++attempts > 5 || (retryStart && Date.now() - retryStart >= RETRY_MS)) { terminal = 'exhausted'; update(); return }
    if (!retryStart) retryStart = Date.now()
    const delay = Math.floor(Math.min(8000, 250 * 2 ** (attempts - 1)) * (0.75 + Math.random() * 0.25))
    retryTimer = setTimeout(() => { retryTimer = null; reconcile() }, delay)
    update()
  }
  const desired = () => {
    const rooms = new Map(), scopes = new Set()
    for (const member of members) if (member.scope && member.rooms.size) {
      scopes.add(member.scope)
      for (const [key, value] of member.rooms) rooms.set(key, value)
    }
    return { rooms, scopes }
  }
  const syncRooms = rooms => {
    const changed = [...rooms.keys()].some(key => !joined.has(key)) || [...joined.keys()].some(key => !rooms.has(key))
    if (waiting.length && changed) { retry(); return }
    for (const [key] of joined) if (!rooms.has(key)) {
      const room = currentRooms.get(key)
      if (!room || !send({ type: 'leave', ...room })) { retry(); return }
      joined.delete(key)
    }
    currentRooms = rooms
    for (const [key, room] of rooms) if (!joined.has(key)) {
      if (waiting.length >= MAX_ROOMS || !send({ type: 'join', ...room })) { retry(); return }
      joined.set(key, 'pending'); waiting.push(key)
    }
    update()
  }
  let currentRooms = new Map()
  const connect = () => {
    const captured = ++epoch
    lastConnect = Date.now()
    let address
    try { address = endpoint() } catch { reject('invalid-config'); return }
    try { socket = new G.WebSocket(address, 'brain.v1') } catch { retry(); return }
    readyTimer = setTimeout(() => { if (epoch === captured) retry() }, READY_MS)
    socket.onopen = () => {
      if (epoch !== captured) return
      if (socket.protocol !== 'brain.v1') reject('protocol-error')
    }
    socket.onerror = () => { /* close or the ready deadline owns retries; arbitrary browser errors are not retained. */ }
    socket.onclose = event => {
      if (epoch !== captured) return
      if (event.code === 1008) reject(event.reason === 'Authentication expired' ? 'auth-required' : 'policy-closed')
      else retry()
    }
    socket.onmessage = event => {
      if (epoch !== captured) return
      const value = metadata(event.data)
      if (!value) { reject('protocol-error'); return }
      if (value.type === 'ready') {
        if (ready) { reject('protocol-error'); return }
        clearTimeout(readyTimer); readyTimer = null; ready = true
        syncRooms(desired().rooms)
        heartbeat = setInterval(() => {
          if (!send({ type: 'ping' })) { retry(); return }
          clearTimeout(pongTimer)
          pongTimer = setTimeout(() => { if (epoch === captured) retry() }, 12000)
        }, 25000)
      } else if (!ready) reject('protocol-error')
      else if (value.type === 'pong') { clearTimeout(pongTimer); pongTimer = null }
      else if (value.type === 'resync' || value.type === 'error') {
        const key = waiting.shift()
        if (!key) { reject('protocol-error'); return }
        joined.set(key, value.type === 'resync' ? 'ready' : 'forbidden')
        if (value.type === 'resync') { attempts = 0; retryStart = 0 }
        update()
        if (value.type === 'resync') invalidate(value, key)
      } else {
        const now = Date.now()
        for (const [id, time] of events) if (now - time > 60000) events.delete(id)
        if (events.has(value.eventId)) return
        events.set(value.eventId, now)
        while (events.size > 128) events.delete(events.keys().next().value)
        invalidate(value)
      }
    }
    update()
  }
  function reconcile() {
    reconcileQueued = false
    const { rooms, scopes } = desired()
    if (!rooms.size) { stop(); scope = null; terminal = ''; attempts = 0; retryStart = 0; update(); return }
    if (scopes.size !== 1) { reject('scope-conflict'); return }
    const next = scopes.values().next().value
    if (next !== scope || terminal === 'scope-conflict') { stop(); scope = next; terminal = ''; attempts = 0; retryStart = 0 }
    if (rooms.size > MAX_ROOMS) { reject('room-limit'); return }
    if (terminal || retryTimer) { update(); return }
    if (socket && ready) syncRooms(rooms)
    else if (!socket) {
      const elapsed = Date.now() - lastConnect
      const delay = elapsed < 0 ? 0 : Math.max(0, 250 - elapsed)
      if (delay) retryTimer = setTimeout(() => { retryTimer = null; reconcile() }, delay)
      else connect()
    }
    update()
  }
  const queue = () => { if (!reconcileQueued) { reconcileQueued = true; queueMicrotask(reconcile) } }
  return {
    valid, complete,
    stats() { return { owners: members.size, rooms: joined.size, pendingJoins: waiting.length, dedupe: events.size, connection: socket ? 1 : 0, timers: [retryTimer, readyTimer, heartbeat, pongTimer, flushTimer].filter(Boolean).length + [...members].filter(member => member.flight && member.flight.deadline).length, refreshes: [...members].filter(member => member.flight).length, dirty: [...members].filter(member => member.dirty).length } },
    add(member) {
      if (!members.has(member) && members.size >= MAX_OWNERS) fail('Close unused live resource owners before adding another')
      members.add(member)
      if (scope && member.scope && member.scope !== scope) stop()
      queue()
    },
    remove(member) {
      members.delete(member); clearMember(member); member.up.v = false
      if (!members.size) { stop(); scope = null; terminal = ''; attempts = 0; retryStart = 0 }
      else queue()
    },
    reconnect(member) {
      if (!members.has(member)) return
      stop(); terminal = ''; attempts = 0; retryStart = 0; queue()
    },
  }
}

function observeMetadata(callback, value) {
  const result = untrack(() => callback(value))
  if (result && typeof result.then === 'function') {
    // Observe rejection without retaining an owner or the user-provided error.
    Promise.resolve(result).catch(() => {})
    fail('Use synchronous metadata observers; resource owns asynchronous fetching')
  }
}

// Promise reactions retain only a nullable cell and scalar flight metadata. An
// externally retained, never-settling Promise cannot keep a disposed resource alive.
function readResource(cell, flight) {
  flight.tick = flight.resolveTick = null
  const member = cell.record
  if (member && broker.valid(member, flight)) return member.node.v
}
function settleFlight(cell, flight, failed) {
  const member = cell.record
  if (member) broker.complete(member, flight, failed)
}
function timeoutFlight(cell, flight) {
  const member = cell.record
  if (member && broker.valid(member, flight)) {
    // Never release a pending fetch slot merely because its deadline elapsed.
    member.refreshState = 'refresh-timeout'
    member.state.v = 'refresh-timeout'
  }
}
function detach(cell) {
  const member = cell.record
  if (member && broker) broker.remove(member)
}
function dispose(cell) {
  const member = cell.record
  if (!member) return
  cell.record = null
  if (member.stopWatch) member.stopWatch()
  if (broker) broker.remove(member)
  member.node = member.options = member.owner = member.stopWatch = null
  member.scope = null; member.rooms.clear(); member.flight = null
  member.state.v = 'closed'
}
function observe(cell) {
  const member = cell.record
  if (!member) return
  const scope = member.options.scope()
  if (scope !== null && (typeof scope !== 'string' || !scope.length || scope.length > 160 || /[\x00-\x20\x7f]/.test(scope))) fail('Use null or a bounded public user/revision string as scope; never credentials')
  member.scope = scope
  member.rooms = roomsOf(typeof member.options.rooms === 'function' ? member.options.rooms() : member.options.rooms)
  broker = broker || createBroker()
  broker.add(member)
  return detach.bind(null, cell)
}
function ownerCleanup(cell) { return dispose.bind(null, cell) }
function reconnect(cell) { if (cell.record && broker) broker.reconnect(cell.record) }

// Internal, metadata-only capacity evidence; never scope identifiers, rooms or values.
export function _liveStats() {
  return broker ? broker.stats() : { owners: 0, rooms: 0, pendingJoins: 0, dedupe: 0, connection: 0, timers: 0, refreshes: 0, dirty: 0 }
}

export function liveResource(node, options = {}) {
  if (!node || typeof node !== 'object' || typeof node.refetch !== 'function' || !('v' in node) || !exact(options, ['rooms', 'scope', 'onInvalidate', 'onResync']) || !own(options, 'rooms') || !own(options, 'scope') || typeof options.scope !== 'function') fail('Use liveResource(rows, {rooms, scope: () => publicUserRevision}) with a resource handle')
  if (own(options, 'onInvalidate') && options.onInvalidate !== undefined && typeof options.onInvalidate !== 'function') fail('Use a metadata callback for onInvalidate')
  if (own(options, 'onResync') && options.onResync !== undefined && typeof options.onResync !== 'function') fail('Use a metadata callback for onResync')
  const state = sig('idle'), up = sig(false), cell = { record: null }
  const out = {
    get connected() { return up.v },
    get state() { return state.v },
    close() { dispose(cell); up.v = false; state.v = 'closed' },
    reconnect: reconnect.bind(null, cell),
  }
  if (G.__brainSSR || G.__brainRoute) return out
  if (typeof G.WebSocket !== 'function') { state.v = 'unsupported'; return out }
  const owner = I.owner()
  if (!owner) fail('Create liveResource inside a component so unmount closes its subscription')
  cell.record = { cell, node, options, state, up, owner, scope: null, rooms: new Map(), generation: 0, flight: null, dirty: false, pending: null, resync: null, refreshState: '', stopWatch: null }
  fx(ownerCleanup.bind(null, cell))
  cell.record.stopWatch = fx(observe.bind(null, cell))
  return out
}
