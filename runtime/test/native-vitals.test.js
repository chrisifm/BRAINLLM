import test from 'node:test'
import assert from 'node:assert/strict'
import { startNativeVitals } from '../brain-native-vitals.js'
import { consent } from '../brain-consent.js'
const config = { base: '/app', routes: ['/', '/items/:id', '/items/new'] }
function fixture(path = '/app/items/private-identifier', accepted = false) {
  const events = new EventTarget(), doc = new EventTarget(), observations = [], sent = [], storage = new Map()
  let navReads = 0
  class PO {
    static supportedEntryTypes = ['paint', 'largest-contentful-paint', 'layout-shift', 'event']
    constructor(read) { this.read = read; this.disconnected = false; observations.push(this) }
    observe(options) { this.type = options.type }
    disconnect() { this.disconnected = true }
  }
  const env = {
    window: events, document: doc, location: { pathname: path, href: 'https://example.test' + path + '?email=private#fragment' },
    navigator: {}, PerformanceObserver: PO, Event, AbortController, setTimeout, clearTimeout,
    localStorage: { getItem: k => storage.get(k), setItem: (k, v) => storage.set(k, v) },
    performance: { getEntriesByType() { navReads++; return [{ responseStart: 42 }] } },
    fetch(url, options) { sent.push({ url, options, body: JSON.parse(options.body) }); return Promise.resolve({ status: 204 }) },
  }
  doc.visibilityState = 'visible'
  if (accepted) consent('analytics', true, env)
  const emit = type => events.dispatchEvent(new Event(type))
  const flush = async () => { doc.visibilityState = 'hidden'; doc.dispatchEvent(new Event('visibilitychange')); await new Promise(setImmediate) }
  return { env, doc, observations, sent, emit, flush, reads: () => navReads }
}
test('collection creates no performance observers or reads before explicit analytics consent', async () => {
  const f = fixture(), state = startNativeVitals(config, f.env)
  assert.equal(f.observations.length, 0); assert.equal(f.reads(), 0)
  await f.flush(); assert.equal(f.sent.length, 0)
  consent('analytics', true, f.env); assert.equal(f.observations.length, 4)
  await f.flush(); assert.equal(f.sent.length, 1)
  assert.deepEqual(f.sent[0].body, { route: '/items/:id', consent: true, metrics: [{ name: 'TTFB', value: 42 }] })
  assert.equal(f.sent[0].url, 'https://example.test/app/_brain/vitals')
  for (const secret of ['private-identifier','email','fragment','rating','path','user','session']) assert.ok(!JSON.stringify(f.sent[0].body).includes(secret))
  assert.equal(f.sent[0].options.credentials, 'omit'); assert.equal(f.sent[0].options.referrerPolicy, 'no-referrer'); assert.equal(f.sent[0].options.redirect, 'error')
  await f.flush(); assert.equal(f.sent.length, 1, 'no retry or duplicate flush without fresh metrics')
  state.stop(); assert.ok(f.observations.every(o => o.disconnected))
})
test('DNT and GPC suppress collection even with remembered consent', async () => {
  for (const signal of ['dnt','gpc']) {
    const f = fixture(undefined, true)
    if (signal === 'dnt') f.env.navigator.doNotTrack = '1'
    else f.env.navigator.globalPrivacyControl = true
    const state = startNativeVitals(config, f.env)
    assert.equal(f.observations.length, 0); assert.equal(f.reads(), 0); await f.flush(); assert.equal(f.sent.length, 0); state.stop()
  }
})
test('revocation immediately disconnects observers, clears pending values and aborts in-flight requests', async () => {
  const f = fixture(undefined, true)
  f.env.fetch = (url, options) => { f.sent.push({ url, options }); return new Promise(() => {}) }
  const state = startNativeVitals(config, f.env)
  await f.flush(); assert.equal(f.sent.length, 1); assert.equal(f.sent[0].options.signal.aborted, false)
  consent('analytics', false, f.env)
  assert.equal(f.sent[0].options.signal.aborted, true); assert.ok(f.observations.every(o => o.disconnected))
  await f.flush(); assert.equal(f.sent.length, 1)
  state.stop()
})
test('navigation stops initial-document measurement and never assigns it to a later SPA route', async () => {
  const f = fixture(undefined, true), state = startNativeVitals(config, f.env)
  f.env.location.pathname = '/app/items/new'; f.emit('brain:navigate')
  await f.flush(); assert.equal(f.sent.length, 0); assert.ok(f.observations.every(o => o.disconnected))
  consent('analytics', true, f.env); assert.equal(f.observations.length, 4); state.stop()
  const delayed = fixture(), pending = startNativeVitals(config, delayed.env)
  delayed.env.location.pathname = '/app/items/new'; consent('analytics', true, delayed.env)
  assert.equal(delayed.reads(), 0); pending.stop()
})
test('literal patterns take priority, unknown routes and SSR add no capture, and 404 removes capture', async () => {
  const literal = fixture('/app/items/new', true), stop = startNativeVitals(config, literal.env)
  await literal.flush(); assert.equal(literal.sent[0].body.route, '/items/new'); stop.stop()
  for (const path of ['/other/items/1','/app/undeclared']) { const f = fixture(path, true); assert.equal(startNativeVitals(config, f.env), undefined); assert.equal(f.reads(), 0) }
  const ssr = fixture(undefined, true); ssr.env.__brainSSR = true; assert.equal(startNativeVitals(config, ssr.env), undefined); assert.equal(ssr.reads(), 0)
  const f = fixture(undefined, true); f.env.fetch = () => Promise.resolve({ status: 404 }); startNativeVitals(config, f.env)
  await f.flush(); assert.ok(f.observations.every(o => o.disconnected)); assert.equal(f.env[Symbol.for('brain:native-vitals')], undefined)
})
test('whitelisted finite bounded numeric estimates are coalesced without ratings or raw entries', async () => {
  const f = fixture(undefined, true), state = startNativeVitals(config, f.env)
  const lcp = f.observations.find(o => o.type === 'largest-contentful-paint')
  lcp.read({ getEntries: () => [{ startTime: 1900, url: 'private' }] }); lcp.read({ getEntries: () => [{ startTime: 2400 }] })
  await f.flush(); assert.deepEqual(f.sent[0].body.metrics, [{ name: 'TTFB', value: 42 }, { name: 'LCP', value: 2400 }])
  lcp.read({ getEntries: () => [{ startTime: Infinity }] }); await f.flush(); assert.equal(f.sent.length, 1)
  state.stop()
})
test('one canonical trailing slash matches the same declared pattern without collapsing repeated separators',async()=>{
  const f=fixture('/app/items/private-identifier/',true),state=startNativeVitals(config,f.env)
  await f.flush();assert.equal(f.sent[0].body.route,'/items/:id');state.stop()
  for(const path of ['/app/items/private-identifier//','/app/items//private-identifier/']){const f=fixture(path,true);assert.equal(startNativeVitals(config,f.env),undefined);assert.equal(f.reads(),0)}
})
test('navigation history remains known before consent and during revocation, including a return to the initial pathname',async()=>{
  for(const initiallyAccepted of [false,true]){
    const f=fixture(undefined,initiallyAccepted),state=startNativeVitals(config,f.env),initial=f.env.location.pathname
    if(initiallyAccepted)consent('analytics',false,f.env)
    const reads=f.reads(),observers=f.observations.length
    f.env.location.pathname='/app/items/new';f.emit('brain:navigate');f.env.location.pathname=initial;f.emit('popstate')
    consent('analytics',true,f.env);await f.flush()
    assert.equal(f.reads(),reads);assert.equal(f.observations.length,observers);assert.equal(f.sent.length,0)
    state.stop()
  }
})
