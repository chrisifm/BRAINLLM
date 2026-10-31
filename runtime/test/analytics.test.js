// brain-analytics.js: every action type is recorded by delegated capture listeners, with a stable element descriptor
// and never a value. The collector is driven through an injected environment on top of the DOM shim.
import { test, mock } from 'node:test'
import assert from 'node:assert/strict'
import { randomUUID } from 'node:crypto'
import { install, Event, MouseEvent, MutationObserver } from '../shim/dom.js'

const doc = install()
const { start } = await import('../brain-analytics.js')

let previous = null
function setup({ nav = {}, consent, html = '', storage = {} } = {}) {
  if (previous) previous.stop()
  doc.body.innerHTML = html
  doc.visibilityState = 'visible'
  const sent = []
  const store = new Map(Object.entries(storage))
  const location = { pathname: '/', search: '?utm_source=news&utm_campaign=fall&email=a@b.co', href: 'http://shop.test/', origin: 'http://shop.test' }
  const go = path => Object.assign(location, { pathname: path, search: '', href: 'http://shop.test' + path })
  let clock = 1000
  const env = {
    document: doc,
    window: doc.defaultView,
    location,
    navigator: { language: 'en-US', sendBeacon: (url, body) => (sent.push({ url, body: JSON.parse(body) }), true), ...nav },
    localStorage: { getItem: k => (store.has(k) ? store.get(k) : null), setItem: (k, v) => store.set(k, String(v)) },
    history: { pushState: (s, t, u) => go(u), replaceState: (s, t, u) => go(u) },
    performance: { now: () => clock },
    crypto: { randomUUID },
    MutationObserver,
  }
  const api = (previous = start({ base: '/', consent }, env))
  const flush = () => (api && api.send(), sent.flatMap(s => s.body.ev))
  return { api, env, sent, flush, store, advance: ms => (clock += ms), $: s => doc.querySelector(s) }
}
const fire = (el, type, props = {}, Ctor = Event) => el.dispatchEvent(Object.assign(new Ctor(type, { bubbles: true, cancelable: true }), props))
const kinds = evs => evs.map(e => e.k)

test('page view: route, session context with UTM only, anonymous ids, beacon to /_brain/analytics', () => {
  const { sent, flush, store } = setup()
  const evs = flush()
  assert.deepEqual(kinds(evs), ['page'])
  assert.equal(sent[0].url, '/_brain/analytics')
  const b = sent[0].body
  assert.equal(b.v, 1)
  assert.match(b.vid, /^[0-9a-f-]{36}$/)
  assert.match(b.sid, /^[0-9a-f-]{36}$/)
  assert.deepEqual(b.ctx.utm, { source: 'news', campaign: 'fall' }) // the e-mail query parameter is never read
  assert.equal(b.ctx.lang, 'en-US')
  assert.equal(evs[0].r, '/')
  assert.equal(evs[0].pv, 1)
  assert.equal(JSON.parse(store.get('brain:vid')), b.vid)
})

test('click: component hash, tag, ordinal, accessible name, data-track, link route; data-track also records a track event', () => {
  const { flush, $ } = setup({
    html: '<main data-b="k3x9"><button>Save</button><a href="/pricing?ref=nav">Plans</a><button data-track="add_to_cart">Add</button></main>',
  })
  fire($('a'), 'click', {}, MouseEvent)
  fire(doc.querySelectorAll('button')[1], 'click', {}, MouseEvent)
  const evs = flush().filter(e => e.k !== 'page')
  assert.deepEqual(evs[0].el, { t: 'a', c: 'k3x9', i: 0, h: '/pricing', n: 'Plans' })
  assert.deepEqual(evs[1].el, { t: 'button', c: 'k3x9', i: 1, x: 'add_to_cart', n: 'Add' })
  assert.deepEqual(evs.find(e => e.k === 'track').p, { name: 'add_to_cart' })
})

test('ordinal skips nested components and counts across sibling roots of the same component', () => {
  const { flush } = setup({
    html: '<div data-b="aaaa"><button>One</button><section data-b="bbbb"><button>Inner</button></section></div><div data-b="aaaa"><button>Two</button></div>',
  })
  for (const b of doc.querySelectorAll('button')) fire(b, 'click', {}, MouseEvent)
  const els = flush()
    .filter(e => e.k === 'click')
    .map(e => [e.el.c, e.el.i])
  assert.deepEqual(els, [['aaaa', 0], ['bbbb', 0], ['aaaa', 1]])
})

test('keyboard activation: Enter/Space on controls only, never the key typed into a text field', () => {
  const { flush, $ } = setup({ html: '<button>Go</button><input name="q">' })
  fire($('button'), 'keydown', { key: 'Enter' })
  fire($('button'), 'keydown', { key: ' ' })
  fire($('button'), 'keydown', { key: 'a' })
  fire($('input'), 'keydown', { key: 'Enter' })
  const keys = flush().filter(e => e.k === 'key')
  assert.deepEqual(keys.map(e => e.p.key), ['enter', 'space'])
})

test('focus and input: one coalesced input event per field visit with a count, focus time on blur, no value', t => {
  const { flush, $, advance } = setup({ html: '<label for="em">Email</label><input id="em" name="email" type="email" value="me@shop.test">' })
  const input = $('input')
  fire(input, 'focusin')
  for (let i = 0; i < 5; i++) fire(input, 'input')
  advance(1500)
  fire(input, 'focusout')
  const evs = flush().filter(e => e.k !== 'page')
  assert.deepEqual(kinds(evs), ['input', 'focus'])
  assert.equal(evs[0].v, 5)
  assert.equal(evs[1].v, 1500)
  assert.deepEqual(evs[0].el, { t: 'input', y: 'email', f: 'email', n: 'Email' })
  assert.ok(!JSON.stringify(evs).includes('me@shop.test'))
})

test('sensitive fields (password, card, one-time code) are only counted: no name, no label, no value', () => {
  const { flush, $ } = setup({
    html: '<input type="password" name="pw" aria-label="Password"><input name="cardnumber" autocomplete="cc-number"><input name="code" autocomplete="one-time-code">',
  })
  for (const i of doc.querySelectorAll('input')) fire(i, 'focusin'), fire(i, 'input'), fire(i, 'focusout')
  const evs = flush().filter(e => e.k === 'input')
  assert.equal(evs.length, 3)
  for (const e of evs) assert.equal(e.el.y, 'secret'), assert.equal(e.el.f, undefined), assert.equal(e.el.n, undefined)
})

test('change and toggles: checkbox state, select index (not its label), details open, text change without value', () => {
  const { flush, $ } = setup({
    html: '<input type="checkbox" name="news"><select name="size"><option>S</option><option>M</option></select><details><summary>More</summary></details><textarea name="note"></textarea>',
  })
  const box = $('input')
  box.checked = true
  fire(box, 'change')
  const sel = $('select')
  sel.selectedIndex = 1
  fire(sel, 'change')
  $('details').setAttribute('open', '')
  $('details').dispatchEvent(new Event('toggle'))
  fire($('textarea'), 'change')
  const evs = flush().filter(e => e.k !== 'page')
  assert.deepEqual(evs.map(e => [e.k, e.v]), [['toggle', 1], ['toggle', 1], ['toggle', 1], ['change', undefined]])
  assert.ok(!JSON.stringify(evs[1]).includes('"M"'))
})

test('submit: form name, action route and outcome (sent, or invalid once)', () => {
  const { flush, $ } = setup({ html: '<form name="checkout" action="/orders/42"><input name="qty" required></form>' })
  const form = $('form')
  $('input').form = form
  fire($('input'), 'invalid')
  fire($('input'), 'invalid')
  fire(form, 'submit')
  const evs = flush().filter(e => e.k === 'submit')
  assert.deepEqual(evs.map(e => e.p.o), ['invalid', 'sent'])
  assert.deepEqual(evs[1].el, { t: 'form', f: 'checkout', h: '/orders/42' })
})

test('copy, cut, paste, drag, drop and context menu are recorded as events only, never their content', () => {
  const { flush, $ } = setup({ html: '<p>Secret text 4111 1111</p><div draggable="true" role="listitem">Card</div>' })
  for (const k of ['copy', 'cut', 'paste', 'contextmenu']) fire($('p'), k, { clipboardData: { getData: () => 'leak' } })
  fire($('div'), 'dragstart')
  fire($('div'), 'drop')
  const evs = flush().filter(e => e.k !== 'page')
  assert.deepEqual(kinds(evs), ['copy', 'cut', 'paste', 'context', 'drag', 'drop'])
  assert.ok(!JSON.stringify(evs).includes('leak'))
})

test('media: play, pause, seek and end with the position in seconds', () => {
  const { flush, $ } = setup({ html: '<video aria-label="Intro"></video>' })
  const v = $('video')
  for (const [a, at] of [['play', 0], ['pause', 12.4], ['seeked', 30], ['ended', 61]]) (v.currentTime = at), v.dispatchEvent(new Event(a))
  const evs = flush().filter(e => e.k === 'media')
  assert.deepEqual(evs.map(e => [e.p.a, e.v]), [['play', 0], ['pause', 12], ['seeked', 30], ['ended', 61]])
})

test('hover intent: pointer resting over 500 ms on an interactive element, once per element and page', () => {
  mock.timers.enable({ apis: ['setTimeout'] })
  try {
    const { flush, $ } = setup({ html: '<a href="/pricing">Pricing</a><button>Buy</button>' })
    fire($('a'), 'pointerover')
    mock.timers.tick(300)
    fire($('a'), 'pointerout')
    mock.timers.tick(500)
    fire($('button'), 'pointerover')
    mock.timers.tick(600)
    fire($('button'), 'pointerout')
    fire($('button'), 'pointerover')
    mock.timers.tick(600)
    const evs = flush().filter(e => e.k === 'hover')
    assert.deepEqual(evs.map(e => e.el.n), ['Buy'])
  } finally {
    mock.timers.reset()
  }
})

test('rage clicks (3 within a second) and dead clicks (no DOM change, navigation or focus move)', () => {
  mock.timers.enable({ apis: ['setTimeout'] })
  try {
    const { flush, $ } = setup({ html: '<button>Apply</button><button>Open</button>' })
    const [apply, open] = doc.querySelectorAll('button')
    for (let i = 0; i < 3; i++) fire(apply, 'click', {}, MouseEvent)
    fire(open, 'click', {}, MouseEvent)
    open.setAttribute('aria-expanded', 'true') // this one changed the page
    mock.timers.tick(1000)
    const evs = flush()
    assert.equal(evs.filter(e => e.k === 'rage').length, 1)
    assert.deepEqual(evs.filter(e => e.k === 'dead').map(e => e.el.n), [])
  } finally {
    mock.timers.reset()
  }
})

test('dead click: a button that changes nothing is reported once per click', () => {
  mock.timers.enable({ apis: ['setTimeout'] })
  try {
    const { flush, $ } = setup({ html: '<button>Broken</button>' })
    fire($('button'), 'click', {}, MouseEvent)
    mock.timers.tick(1000)
    assert.deepEqual(flush().filter(e => e.k === 'dead').map(e => e.el.n), ['Broken'])
  } finally {
    mock.timers.reset()
  }
})

test('SPA navigation: leave with dwell and scroll depth, then a new page view on the new route', () => {
  const { flush, env, advance } = setup({ html: '' })
  doc.documentElement.scrollHeight = 2000
  env.window.innerHeight = 1000
  env.window.scrollY = 500
  env.window.dispatchEvent(new Event('scroll'))
  advance(4000)
  env.history.pushState({}, '', '/checkout')
  const evs = flush()
  assert.deepEqual(kinds(evs), ['page', 'leave', 'page'])
  assert.deepEqual([evs[1].r, evs[1].v, evs[1].p.s], ['/', 4000, 50])
  assert.deepEqual([evs[2].r, evs[2].pv], ['/checkout', 2])
})

test('dwell counts visible time only; hiding the page flushes the queue', () => {
  const { sent, env, advance } = setup()
  advance(1000)
  doc.visibilityState = 'hidden'
  doc.dispatchEvent(new Event('visibilitychange'))
  assert.equal(sent.length, 1)
  advance(60000)
  doc.visibilityState = 'visible'
  doc.dispatchEvent(new Event('visibilitychange'))
  advance(500)
  env.window.dispatchEvent(new Event('pagehide'))
  const leave = sent.flatMap(s => s.body.ev).find(e => e.k === 'leave')
  assert.equal(leave.v, 1500)
})

test('custom events: brain:track with a name and scalar props only', () => {
  const { flush, env } = setup()
  const ev = new Event('brain:track')
  ev.detail = { name: 'add_to_cart', props: { sku: 'A1', qty: 2, nested: { no: 1 } } }
  env.window.dispatchEvent(ev)
  const bad = new Event('brain:track')
  bad.detail = { name: 'has spaces' }
  env.window.dispatchEvent(bad)
  assert.deepEqual(flush().filter(e => e.k === 'track').map(e => e.p), [{ name: 'add_to_cart', sku: 'A1', qty: 2 }])
})

test('privacy: Global Privacy Control, Do Not Track and automated browsers record nothing', () => {
  for (const nav of [{ globalPrivacyControl: true }, { doNotTrack: '1' }, { webdriver: true }]) {
    const { api, sent, store } = setup({ nav })
    assert.equal(api, null)
    assert.equal(sent.length, 0)
    assert.equal(store.size, 0) // no visitor id either
  }
})

test('consent required: nothing is stored or sent until consent("analytics"); then the page view opens first', () => {
  const { flush, store, $ } = setup({ consent: 'required', html: '<button>Buy</button>' })
  fire($('button'), 'click', {}, MouseEvent)
  assert.deepEqual(flush(), [])
  assert.equal(store.has('brain:vid'), false)
  store.set('brain:consent', JSON.stringify({ analytics: true }))
  fire($('button'), 'click', {}, MouseEvent)
  assert.deepEqual(kinds(flush()), ['page', 'click'])
})

test('a session ends after 30 idle minutes; the visitor id survives', () => {
  const old = { id: '00000000-0000-4000-8000-000000000001', last: Date.now() - 31 * 60 * 1000, pv: 4, ctx: 1 }
  const { sent, flush } = setup({ storage: { 'brain:sid': JSON.stringify(old), 'brain:vid': JSON.stringify('00000000-0000-4000-8000-0000000000aa') } })
  flush()
  assert.notEqual(sent[0].body.sid, old.id)
  assert.equal(sent[0].body.vid, '00000000-0000-4000-8000-0000000000aa')
  assert.ok(sent[0].body.ctx) // a new session sends its context again
})

test('batches stay small: at most 64 events per request', () => {
  const { sent, flush, $ } = setup({ html: '<p>x</p>' })
  for (let i = 0; i < 150; i++) fire($('p'), 'copy')
  flush()
  assert.ok(sent.every(s => s.body.ev.length <= 64))
  assert.equal(sent.reduce((n, s) => n + s.body.ev.length, 0), 151)
})
