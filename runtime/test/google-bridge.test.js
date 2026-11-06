import { test } from 'node:test'
import assert from 'node:assert/strict'
import { install, Event, MouseEvent } from '../shim/dom.js'
import { start } from '../brain-analytics.js'
import { startGoogle } from '../brain-google.js'
const doc = install()
globalThis.localStorage = { getItem: () => null }
const { consent, consented, consentBanner } = await import('../brain-consent.js')
let previous, banner
function setup(config = {}, initial = null) {
  previous?.stop(); banner?.stop(); doc.body.innerHTML = '<button data-track="save"><span>person@example.com</span></button><form data-track="submit"><input name="email" value="person@example.com"></form><div data-private><button data-track="save">Private</button></div>'
  for (const el of [...doc.head.querySelectorAll('script')]) el.parentNode.removeChild(el)
  const storage = new Map(initial === null ? [] : [['brain:consent', JSON.stringify({ analytics: initial })]])
  const env = { document: doc, window: doc.defaultView, Event, navigator: {}, location: { origin: 'https://example.com', href: 'https://example.com/person@example.com?token=private#secret' }, localStorage: { getItem: key => storage.get(key) ?? null, setItem: (key, value) => storage.set(key, value) } }
  previous = startGoogle({ mode: 'basic', analytics: 'G-ABC123', events: { save: 'save_item', submit: 'generate_lead' }, ...config }, env)
  return { env, api: previous, storage, messages: () => (env.dataLayer || []).filter(entry => entry[0] === 'event' || (entry.event && entry.event !== 'gtm.js')) }
}
const fire = (element, type, props = {}) => element.dispatchEvent(Object.assign(new Event(type, { bubbles: true, cancelable: true }), props))
test('basic mode: denied, unset and truthy malformed consent do not load tags or queue events; opt-in is explicit', () => {
  for (const state of [null, false, 'true', 1]) {
    const { env, api } = setup({}, state)
    fire(doc.querySelector('span'), 'click'); fire(doc.querySelector('form'), 'submit'); api.track('save')
    assert.equal(env.dataLayer, undefined); assert.equal(doc.head.querySelectorAll('script').length, 0)
  }
  assert.equal(startGoogle({}, { document: doc }), null)
})
test('GA4 declared click, form and custom mapping omits values, properties, paths, referrers and DOM text', () => {
  const { env, messages } = setup({ funnels: { acquisition: ['save_item', 'generate_lead'] } })
  consent('analytics', true, env)
  fire(doc.querySelector('span'), 'click'); fire(doc.querySelector('form'), 'submit')
  fire(env.window, 'brain:track', { detail: { name: 'save', props: { email: 'person@example.com', token: 'private' } } })
  const events = messages()
  assert.deepEqual(events.map(entry => entry[1]), ['save_item', 'generate_lead', 'save_item'])
  assert.equal(events[0][2].send_to, 'G-ABC123'); assert.equal(events[0][2].page_location, 'https://example.com/')
  assert.equal(events[1][2].brain_step, 2); assert.equal(events[1][2].brain_funnel, 'acquisition')
  assert(!JSON.stringify(env.dataLayer).includes('person@')); assert(!JSON.stringify(env.dataLayer).includes('secret')); assert(!JSON.stringify(env.dataLayer).includes('private'))
  const config = env.dataLayer.find(entry => entry[0] === 'config')[2]
  assert.equal(config.send_page_view, false); assert.equal(config.allow_google_signals, false)
  fire(doc.querySelector('div button'), 'click'); assert.equal(messages().length, 3)
})
test('GTM sends dataLayer event objects only for declared mappings; missing or invalid names are dropped', () => {
  const { env, messages, api } = setup({ analytics: undefined, tagManager: 'GTM-ABC123', events: { save: 'save_item', bad: 'ga_reserved', mail: 'person@example.com' } }, true)
  assert.equal(api.track('unlisted'), false); assert.equal(api.track('bad'), false); assert.equal(api.track('mail'), false)
  api.track('save')
  assert.equal(messages()[0].event, 'save_item'); assert(!('send_to' in messages()[0]))
  assert.match(doc.head.querySelector('script').src, /gtm\.js\?id=GTM-ABC123$/)
  assert.equal(env.dataLayer.filter(entry => entry.event === 'gtm.js').length, 1)
})
test('revocation disables GA4, drops our pending events, keeps unrelated queue entries and never replays denied actions', () => {
  const { env, api, messages } = setup({}, true)
  env.dataLayer.push({ event: 'unrelated' }); api.track('save')
  consent('analytics', false, env)
  assert.equal(env['ga-disable-G-ABC123'], true); assert.deepEqual(messages().map(e => e.event), ['unrelated'])
  for (let i = 0; i < 5; i++) api.track('save')
  consent('analytics', true, env)
  assert.equal(messages().length, 1); assert.equal(env['ga-disable-G-ABC123'], false)
  api.track('submit'); assert.equal(messages().length, 2)
  assert.equal(doc.head.querySelectorAll('script').length, 1)
})
test('blocked storage still supports explicit in-page consent; defaults stay denied and consent API rejects non-booleans', () => {
  const { env, api } = setup()
  env.localStorage = { getItem() { throw Error('blocked') }, setItem() { throw Error('blocked') } }
  assert.equal(api.track('save'), false)
  consent('analytics', true, env); assert.equal(consented('analytics', env), true); assert.equal(api.track('save'), true)
  assert.throws(() => consent('analytics', 'true', env), /true or false/)
  consent('analytics', false, env); assert.equal(api.track('save'), false)
})
test('banner supports deny, accept, manage and revoke using accessible native buttons', () => {
  const { env } = setup()
  banner = consentBanner({}, env)
  assert.equal(banner.element.getAttribute('role'), 'region')
  const buttons = [...banner.element.querySelectorAll('button')]
  fire(buttons[0], 'click'); assert.equal(consented('analytics', env), false)
  fire(buttons[2], 'click'); assert.equal(banner.element.querySelector('div').hidden, false)
  fire(buttons[1], 'click'); assert.equal(consented('analytics', env), true)
  fire(buttons[3], 'click'); assert.equal(consented('analytics', env), false)
  banner.stop(); assert.equal(doc.querySelector('[data-brain-consent]'), null)
})
test('privacy signals, disabled/mixed ids and malformed data layers remain off; owned listeners and loaders stop', () => {
  for (const config of [{ analytics: 'G-ABC123', tagManager: 'GTM-ABC123' }, { analytics: 'bad-id' }, { mode: 'advanced' }]) assert.equal(setup(config).api, null)
  const { env, api, messages } = setup({}, true)
  api.stop(); fire(doc.querySelector('span'), 'click'); assert.equal(messages().length, 0); assert.equal(doc.head.querySelectorAll('script').length, 0)
  for (const navigator of [{ globalPrivacyControl: true }, { doNotTrack: '1' }, { webdriver: true }]) assert.equal(startGoogle({ mode: 'basic', analytics: 'G-ABC123' }, { ...env, navigator }), null)
})
test('offline loader queue is bounded and consent-required first-party batches are dropped after revoke', () => {
  const { env, api, messages, storage } = setup({}, true)
  for (let i = 0; i < 200; i++) api.track('save')
  assert.equal(messages().length, 128)
  consent('analytics', false, env); assert.equal(messages().length, 0)
  const sent = []; env.navigator.sendBeacon = (url, body) => (sent.push(body), true)
  storage.set('brain:consent', JSON.stringify({ analytics: true }))
  const own = start({ consent: 'required' }, env)
  storage.set('brain:consent', JSON.stringify({ analytics: false })); own.send(); own.stop()
  assert.equal(sent.length, 0)
})

test('cross-tab revocation and broken external queues fail closed without breaking consent controls', () => {
  const { env, api, storage } = setup({}, true)
  api.track('save')
  storage.set('brain:consent', JSON.stringify({ analytics: false }))
  fire(env.window, 'storage', { key: 'brain:consent' })
  assert.equal(api.track('save'), false); assert.equal(env['ga-disable-G-ABC123'], true)
  env.gtag = () => { throw Error('blocked external tag') }
  assert.doesNotThrow(() => consent('analytics', false, env))
  const broken = setup({}, true)
  broken.env.dataLayer = Object.freeze([])
  assert.equal(broken.api.track('save'), false)
  assert.equal(startGoogle({mode:'basic',analytics:'G-ABC123'}, {...env,__brainSSR:true}),null)
  assert.equal(consentBanner({}, {...env,__brainSSR:true}),null)
})

test('native required consent uses explicit memory over stale read-only storage and never replays revoked batches', () => {
  for (const beaconResult of [true, false]) {
    const { env } = setup({}, true)
    env.localStorage.setItem = () => { throw Error('read-only storage') }
    let beacons = 0, fetches = 0
    env.navigator.sendBeacon = () => { beacons++; return beaconResult }
    env.fetch = () => { fetches++; return Promise.resolve() }
    const collector = start({ consent: 'required' }, env)
    try {
      collector.track('before_revoke'); assert(collector.queue().length > 0)
      consent('analytics', false, env)
      assert.equal(consented('analytics', env), false)
      assert.equal(collector.queue().length, 0)
      collector.track('while_denied'); collector.send()
      assert.equal(beacons, 0); assert.equal(fetches, 0)
      consent('analytics', true, env)
      collector.track('after_accept')
      assert(!JSON.stringify(collector.queue()).includes('before_revoke'))
      assert(!JSON.stringify(collector.queue()).includes('while_denied'))
      consent('analytics', false, env); collector.send()
      assert.equal(beacons, 0); assert.equal(fetches, 0)
    } finally { collector.stop() }
  }
})
test('native-only collector shares blocked-storage decisions and cross-tab revocation clears pending data', () => {
  const { env, storage } = setup({ mode: 'advanced' }, true) // no Google bridge listener can repair native consent
  let beacons = 0, fetches = 0
  env.navigator.sendBeacon = () => { beacons++; return false }
  env.fetch = () => { fetches++; return Promise.resolve() }
  consent('analytics', true, env)
  const collector = start({ consent: 'required' }, env)
  try {
    collector.track('before_cross_tab')
    storage.set('brain:consent', JSON.stringify({ analytics: false }))
    fire(env.window, 'storage', { key: 'brain:consent' })
    assert.equal(consented('analytics', env), false); assert.equal(collector.queue().length, 0)
    collector.send(); assert.equal(beacons, 0); assert.equal(fetches, 0)
    env.localStorage.getItem = () => { throw Error('blocked storage') }
    env.localStorage.setItem = () => { throw Error('blocked storage') }
    consent('analytics', true, env); collector.track('in_page_accept')
    assert(collector.queue().some(event => event.p?.name === 'in_page_accept'))
    consent('analytics', false, env); collector.send()
    assert.equal(collector.queue().length, 0); assert.equal(beacons, 0); assert.equal(fetches, 0)
  } finally { collector.stop() }
})

test('storage.clear revokes Google-only pending events, native batches and the optional banner immediately', () => {
  for (const withBanner of [false, true]) {
    const { env, api, storage, messages } = setup({}, true)
    if (withBanner) banner = consentBanner({}, env)
    consent('analytics', true, env); api.track('save'); assert.equal(messages().length, 1)
    storage.clear(); fire(env.window, 'storage', { key: null })
    assert.equal(consented('analytics', env), false); assert.equal(messages().length, 0)
    assert.equal(env['ga-disable-G-ABC123'], true)
    if (withBanner) assert.equal([...banner.element.querySelectorAll('button')][3].hidden, true)
    assert.equal(api.track('save'), false)
  }
})

test('native required consent works when access to localStorage itself is blocked', () => {
  const { env } = setup({ mode: 'advanced' })
  Object.defineProperty(env, 'localStorage', { get() { throw Error('storage object blocked') } })
  consent('analytics', true, env)
  let beacons = 0, fetches = 0
  env.navigator.sendBeacon = () => { beacons++; return false }
  env.fetch = () => { fetches++; return Promise.resolve() }
  const collector = start({ consent: 'required' }, env)
  try {
    collector.track('allowed_in_memory'); assert(collector.queue().length > 0)
    consent('analytics', false, env); collector.send()
    assert.equal(collector.queue().length, 0); assert.equal(beacons, 0); assert.equal(fetches, 0)
  } finally { collector.stop() }
})

test('denied native interactions and pre-revocation deferred events cannot reappear after accept', () => {
  const { env } = setup({ mode: 'advanced' }, true)
  const collector = start({ consent: 'required' }, env)
  try {
    const input = doc.querySelector('input')
    fire(input, 'focusin'); fire(input, 'input')
    consent('analytics', false, env)
    fire(input, 'focusin'); fire(input, 'input')
    consent('analytics', true, env)
    fire(input, 'focusout')
    assert(!collector.queue().some(event => event.k === 'input' || event.k === 'focus'))
  } finally { collector.stop() }
})
