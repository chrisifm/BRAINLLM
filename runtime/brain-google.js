const TOKEN = /^[\w.:-]{1,64}$/

// Explicit basic-mode Google bridge. It runs without the first-party collector or a native backend.
// Only declared event names cross the boundary: props, DOM text, field values, URLs and visitor ids never do.
export function startGoogle(config = {}, env = globalThis) {
  const doc = env.document, win = env.window || env, nav = env.navigator || {}
  const id = config.analytics || config.tagManager
  if (env.__brainSSR || env.__brainRoute || !doc || config.mode !== 'basic' || !id || (config.analytics && config.tagManager)) return null
  if (config.analytics ? !/^G-[A-Z0-9]{1,32}$/.test(id) : !/^GTM-[A-Z0-9]{1,32}$/.test(id)) return null
  if (nav.globalPrivacyControl === true || nav.doNotTrack === '1' || win.doNotTrack === '1' || nav.webdriver) return null
  const eventName = name => typeof name === 'string' && /^[A-Za-z][A-Za-z0-9_]{0,39}$/.test(name) && !/^(google_|ga_|firebase_)/i.test(name)
  const events = Object.create(null)
  for (const [source, name] of Object.entries(config.events || {}).slice(0, 100)) if (TOKEN.test(source) && eventName(name)) events[source] = name
  const funnels = Object.entries(config.funnels || {}).slice(0, 20).filter(([name, steps]) => eventName(name) && Array.isArray(steps) && steps.length >= 2 && steps.length <= 20)
  let stopped = false, initialized = false, loaded = false, lastGranted = false, loader = null
  const pending = new Set(), off = []
  const allowed = () => {
    if (stopped || nav.globalPrivacyControl === true || nav.doNotTrack === '1' || win.doNotTrack === '1') return false
    try {
      const memory = env[Symbol.for('brain:consent-state')]
      if (memory && Object.hasOwn(memory, 'analytics')) return memory.analytics === true
      return JSON.parse(env.localStorage.getItem('brain:consent'))?.analytics === true
    } catch { return false }
  }
  const listen = (target, type, fn) => {
    if (!target?.addEventListener) return
    target.addEventListener(type, fn, true)
    off.push(() => target.removeEventListener(type, fn, true))
  }
  const clearPending = () => {
    if (Array.isArray(env.dataLayer)) for (let i = env.dataLayer.length - 1; i >= 0; i--) if (pending.has(env.dataLayer[i])) env.dataLayer.splice(i, 1)
    pending.clear()
  }
  const privacy = () => {
    let origin = 'https://example.invalid/'
    try { const url = new URL(env.location?.origin); if (/^https?:$/.test(url.protocol)) origin = url.origin + '/' } catch {}
    return { page_location: origin, page_referrer: '', page_title: '[redacted]', send_page_view: false, allow_google_signals: false, allow_ad_personalization_signals: false }
  }
  const sync = () => {
    try {
    const granted = allowed()
    if (config.analytics) env['ga-disable-' + id] = !granted
    if (!granted) {
      clearPending()
      if (initialized && lastGranted) env.gtag('consent', 'update', { analytics_storage: 'denied' })
      lastGranted = false
      return false
    }
    if (initialized) {
      if (!lastGranted) env.gtag('consent', 'update', { analytics_storage: 'granted' })
      lastGranted = true
      return true
    }
    // Do not adopt a malformed/preconfigured data layer. Never overwrite another integration's queue.
    if (env.dataLayer !== undefined && !Array.isArray(env.dataLayer)) return false
    env.dataLayer ||= []
    if (env.gtag !== undefined && typeof env.gtag !== 'function') return false
    env.gtag ||= function () { env.dataLayer.push(arguments) }
    env.gtag('consent', 'default', { analytics_storage: 'denied', ad_storage: 'denied', ad_user_data: 'denied', ad_personalization: 'denied' })
    env.gtag('consent', 'update', { analytics_storage: 'granted', ad_storage: 'denied', ad_user_data: 'denied', ad_personalization: 'denied' })
    if (config.analytics) { env.gtag('js', new Date()); env.gtag('config', id, privacy()) }
    else env.dataLayer.push({ 'gtm.start': Date.now(), event: 'gtm.js' })
    loader = doc.createElement('script')
    loader.async = true
    loader.onload = () => { loaded = true; pending.clear() }
    loader.src = 'https://www.googletagmanager.com/' + (config.analytics ? 'gtag/js' : 'gtm.js') + '?id=' + id
    const parent = doc.head || doc.documentElement
    if (!parent) return false
    parent.appendChild(loader)
    initialized = lastGranted = true
    return true
    } catch { return false }
  }
  const track = source => {
    if (typeof source !== 'string' || !Object.hasOwn(events, source) || !sync() || (!loaded && pending.size >= 128)) return false
    const name = events[source], params = privacy()
    delete params.send_page_view
    // A declared funnel is a list of named steps, not an inferred conversion or a separate extra event.
    for (const [funnel, steps] of funnels) {
      const step = Array.isArray(steps) ? steps.indexOf(name) : -1
      if (eventName(funnel) && step >= 0 && step < 20) { params.brain_funnel = funnel; params.brain_step = step + 1; break }
    }
    let message
    if (config.analytics) {
      params.send_to = id
      // Arguments objects are the official gtag queue shape. Keep references so revocation drops our pending events.
      message = (function () { return arguments })('event', name, params)
    } else message = { event: name, ...params }
    if (!loaded) pending.add(message)
    try { env.dataLayer.push(message); return true } catch { pending.delete(message); return false }
  }
  const tracked = target => {
    let found = null
    for (let el = target; el?.nodeType === 1; el = el.parentNode) {
      if (el.hasAttribute('data-private')) return null
      if (!found && el.hasAttribute('data-track')) found = el
    }
    return found
  }
  listen(doc, 'click', e => {
    const el = tracked(e.target)
    if (el && el.tagName !== 'FORM') track(el.getAttribute('data-track'))
  })
  listen(doc, 'submit', e => {
    const el = e.target
    if (el?.tagName === 'FORM' && tracked(el) === el) track(el.getAttribute('data-track'))
  })
  listen(win, 'brain:track', e => { if (e.detail && typeof e.detail.name === 'string') track(e.detail.name) })
  listen(win, 'brain:consent', sync)
  listen(win, 'storage', e => { if (e.key === 'brain:consent' || e.key === null) { delete env[Symbol.for('brain:consent-state')]; sync() } })
  sync()
  return { track, stop() { stopped = true; clearPending(); if (config.analytics) env['ga-disable-' + id] = true; for (const fn of off) fn(); if (loader?.parentNode) loader.parentNode.removeChild(loader) } }
}
