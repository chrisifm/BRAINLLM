// BRAINLLM behavior analytics collector (docs/backend/analytics.md). First-party and off the critical path: the native
// backend serves this module at `/_brain/analytics.js` and the server adds it to HTML pages as an async module, so it
// never counts toward a route's budget. With no backend nothing loads and nothing is recorded.
//
// One delegated, capture-phase listener per action type on the document records what every element receives
// (click, keyboard activation, focus, input, change, toggles, submit, copy/cut/paste, media, drag/drop, context menu,
// hover intent), plus page views, scroll depth, dwell and SPA navigations. Each event carries a stable descriptor of the
// element: `c` the component hash (`data-b`, the same id the compiler gives every component root), `t` tag, `i` ordinal
// among same-tag elements of that component, `n` accessible name, `x` data-track, `o` role, `y` type, `f` field or form
// name, `h` link route. `brain analytics report` maps `c` + `t` + `i`/`n` back to `file:line` of the .brain source.
//
// Never recorded: field values, typed keys, copied text, IP addresses. Password, card and one-time-code fields are only
// counted. Global Privacy Control, Do Not Track and automated browsers turn the collector off; `consent: "required"`
// waits for consent('analytics') from brain-consent.js.
//
// Custom events from app code, with no import:
//   dispatchEvent(new CustomEvent('brain:track', { detail: { name: 'add_to_cart', props: { sku: 'A1' } } }))
// or declaratively: <button data-track="add_to_cart">.
const VISITOR = 'brain:vid'
const SESSION = 'brain:sid'
const IDLE_MS = 30 * 60 * 1000
const FLUSH_MS = 5000
const BATCH = 64
const HOVER_MS = 500
const RAGE = 3
const RAGE_MS = 1000
const DEAD_MS = 1000
const TAGS = /^(A|BUTTON|INPUT|SELECT|TEXTAREA|SUMMARY|DETAILS|LABEL|OPTION|VIDEO|AUDIO|FORM|IFRAME)$/
const ROLES =
  /^(button|link|tab|menuitem|menuitemcheckbox|menuitemradio|checkbox|radio|switch|option|combobox|slider|textbox|searchbox|treeitem|gridcell)$/
const FIELDS = /^(INPUT|SELECT|TEXTAREA|LABEL|OPTION|VIDEO|AUDIO|IFRAME|FORM|DETAILS)$/
const SECRET = /pass|card|cvc|cvv|ccv|iban|ssn|secret|token|otp|pin$/i
const TOKEN = /^[\w.:-]{1,64}$/

export function start(config = {}, env = globalThis) {
  const doc = env.document
  const win = env.window || env
  const nav = env.navigator || {}
  const loc = () => env.location || { pathname: '/', search: '', href: '/', origin: '' }
  const store = env.localStorage
  const now = () => (env.performance ? env.performance.now() : Date.now())
  if (!doc || config.enabled === false) return null
  if (nav.globalPrivacyControl === true || nav.doNotTrack === '1' || win.doNotTrack === '1' || nav.webdriver) return null
  const endpoint = config.endpoint || (config.base || '/') + '_brain/analytics'
  const consented = () => {
    if (config.consent !== 'required') return true
    try {
      return !!(JSON.parse(store.getItem('brain:consent')) || {}).analytics
    } catch {
      return false
    }
  }
  const get = k => {
    try {
      return JSON.parse(store.getItem(k))
    } catch {
      return null
    }
  }
  const put = (k, v) => {
    try {
      store.setItem(k, JSON.stringify(v))
    } catch {}
  }
  const uuid = () =>
    env.crypto && env.crypto.randomUUID
      ? env.crypto.randomUUID()
      : '10000000-1000-4000-8000-100000000000'.replace(/[018]/g, c =>
          (c ^ ((Math.random() * 16) >> (c / 4))).toString(16),
        )

  // ── identity: anonymous first-party visitor (localStorage) and a session that ends after 30 idle minutes
  let session = null
  const ids = () => {
    const t = Date.now()
    let vid = get(VISITOR)
    if (typeof vid !== 'string') put(VISITOR, (vid = uuid()))
    if (!session) session = get(SESSION)
    if (!session || typeof session.id !== 'string' || t - session.last > IDLE_MS) session = { id: uuid(), last: t, pv: 0, ctx: 0 }
    session.last = t
    put(SESSION, session)
    return vid
  }

  // ── element descriptor
  const attr = (n, k) => (n && n.getAttribute && n.getAttribute(k)) || ''
  const actionable = el => {
    for (let n = el; n && n.nodeType === 1; n = n.parentNode)
      if (TAGS.test(n.tagName) || ROLES.test(attr(n, 'role')) || n.hasAttribute('data-track') || n.hasAttribute('tabindex')) return n
    return null
  }
  const secret = el =>
    el.tagName !== 'A' &&
    (attr(el, 'type') === 'password' ||
      /^(cc-|one-time-code)/.test(attr(el, 'autocomplete')) ||
      SECRET.test(attr(el, 'name') + ' ' + attr(el, 'id')) ||
      el.hasAttribute('data-private'))
  const text = s => String(s || '').replace(/\s+/g, ' ').trim().slice(0, 64)
  const accessibleName = el => {
    const label = attr(el, 'aria-label')
    if (label) return text(label)
    const by = attr(el, 'aria-labelledby')
    if (by) {
      const n = doc.getElementById(by.split(/\s+/)[0])
      if (n) return text(n.textContent)
    }
    if (/^(INPUT|SELECT|TEXTAREA)$/.test(el.tagName)) {
      const id = attr(el, 'id')
      const l = id && doc.querySelector && doc.querySelector(`label[for=${id}]`)
      return text((l && l.textContent) || attr(el, 'placeholder') || attr(el, 'title'))
    }
    return text(attr(el, 'title') || attr(el, 'alt') || el.textContent)
  }
  const component = el => {
    for (let n = el; n && n.nodeType === 1; n = n.parentNode) if (n.hasAttribute('data-b')) return n
    return null
  }
  // Ordinal among same-tag elements of the component, in document order, skipping nested components.
  const ordinal = (root, el) => {
    const hash = attr(root, 'data-b')
    let i = 0
    let found = -1
    const walk = n => {
      for (const c of n.childNodes || []) {
        if (found >= 0 || c.nodeType !== 1) continue
        if (c.hasAttribute('data-b') && attr(c, 'data-b') !== hash) continue
        if (c === el) return void (found = i)
        if (c.tagName === el.tagName) i++
        walk(c)
      }
    }
    const roots = root.parentNode ? [...root.parentNode.childNodes].filter(c => c.nodeType === 1 && attr(c, 'data-b') === hash) : [root]
    for (const r of roots) {
      if (found >= 0) break
      if (r === el) found = i
      else {
        if (r.tagName === el.tagName) i++
        walk(r)
      }
    }
    return found
  }
  const route = href => {
    try {
      const u = new URL(href, loc().href)
      return u.origin === loc().origin ? u.pathname : null
    } catch {
      return null
    }
  }
  const describe = el => {
    if (!el || el.nodeType !== 1) return null
    const d = { t: el.tagName.toLowerCase() }
    const root = component(el)
    if (root) {
      d.c = attr(root, 'data-b')
      const i = ordinal(root, el)
      if (i >= 0 && i < 10000) d.i = i
    }
    const track = attr(el, 'data-track')
    if (TOKEN.test(track)) d.x = track
    const role = attr(el, 'role')
    if (TOKEN.test(role)) d.o = role
    if (/^(INPUT|BUTTON)$/.test(el.tagName) && TOKEN.test(attr(el, 'type'))) d.y = attr(el, 'type')
    if (secret(el)) return (d.y = 'secret'), d // counted, never named
    const field = attr(el, 'name') || (/^(INPUT|SELECT|TEXTAREA|FORM)$/.test(el.tagName) ? attr(el, 'id') : '')
    if (TOKEN.test(field)) d.f = field
    if (el.tagName === 'A') {
      const h = route(attr(el, 'href'))
      if (h) d.h = h
    }
    if (el.tagName === 'FORM') {
      const h = route(attr(el, 'action') || loc().href)
      if (h) d.h = h
    } else if (actionable(el) === el) {
      const n = accessibleName(el) // only controls are named: the text of arbitrary content is never read
      if (n) d.n = n
    }
    return d
  }

  // ── queue and transport
  let queue = []
  let timer = null
  let pv = 0
  let pageAt = now()
  let visibleMs = 0
  let visibleAt = now()
  let maxScroll = 0
  let path = loc().pathname
  let stopped = false
  const send = () => {
    clearTimeout(timer)
    timer = null
    if (!queue.length) return
    const vid = ids()
    while (queue.length) {
      const body = { v: 1, vid, sid: session.id, ev: queue.splice(0, BATCH) }
      if (!session.ctx) {
        const q = new URLSearchParams(loc().search || '')
        const utm = {}
        for (const k of ['source', 'medium', 'campaign', 'term', 'content']) if (q.get('utm_' + k)) utm[k] = q.get('utm_' + k).slice(0, 128)
        body.ctx = { ref: doc.referrer || undefined, utm, lang: nav.language, w: win.innerWidth | 0 }
        session.ctx = 1
        put(SESSION, session)
      }
      const json = JSON.stringify(body)
      let ok = false
      try {
        ok = !!(nav.sendBeacon && nav.sendBeacon(endpoint, json))
      } catch {}
      if (!ok && env.fetch)
        env
          .fetch(endpoint, { method: 'POST', body: json, keepalive: true, credentials: 'same-origin', headers: { 'content-type': 'text/plain' } })
          .catch(() => {})
    }
  }
  const record = (k, el, v, p) => {
    if (stopped || !consented()) return
    if (!pv && k !== 'page') page() // consent arrived after load: open the page view first
    const e = { k, r: path, pv, t: Math.max(0, Math.round(now() - pageAt)), at: Date.now() }
    const d = el ? describe(el) : null
    if (d) e.el = d
    if (v != null && isFinite(v)) e.v = Math.round(v)
    if (p) e.p = p
    queue.push(e)
    if (queue.length >= BATCH / 2) send()
    else if (!timer) timer = setTimeout(send, FLUSH_MS)
  }

  // ── page views, dwell, scroll
  const visible = () => doc.visibilityState !== 'hidden'
  const dwell = () => visibleMs + (visible() ? now() - visibleAt : 0)
  const page = () => {
    if (!consented()) return
    ids()
    pv = ++session.pv
    put(SESSION, session)
    pageAt = visibleAt = now()
    visibleMs = maxScroll = 0
    record('page', null)
  }
  const leave = () => {
    flushInputs()
    if (pv) record('leave', null, dwell(), { scroll: Math.round(maxScroll) })
  }
  const navigated = () => {
    const p = loc().pathname
    if (p === path) return
    leave()
    path = p
    page()
  }
  const scrolled = () => {
    const el = doc.documentElement || {}
    const h = (el.scrollHeight || 0) - (win.innerHeight || 0)
    const pct = h > 0 ? (100 * (win.scrollY || el.scrollTop || 0)) / h : 100
    if (pct > maxScroll) maxScroll = Math.min(100, pct)
  }

  // ── interactions
  const off = []
  const listen = (t, f, target = doc) => {
    if (!target.addEventListener) return
    const o = { capture: true, passive: true }
    target.addEventListener(t, f, o)
    off.push(() => target.removeEventListener(t, f, o))
  }
  const clicks = []
  let mutations = 0
  const Observer = env.MutationObserver
  const mo = Observer && new Observer(r => (mutations += r.length))
  if (mo) mo.observe(doc, { subtree: true, childList: true, attributes: true, characterData: true })
  const changes = () => (mutations += mo && mo.takeRecords ? mo.takeRecords().length : 0)
  listen('click', e => {
    const el = actionable(e.target) || e.target
    const touch = e.pointerType === 'touch' || e.pointerType === 'pen'
    record('click', el, null, touch ? { tap: 1 } : undefined)
    const t = now()
    while (clicks.length && t - clicks[0].t > RAGE_MS) clicks.shift()
    clicks.push({ el, t })
    const same = clicks.filter(c => c.el === el).length
    if (same === RAGE) record('rage', el, same)
    const plainLink = el.tagName === 'A' && !el.hasAttribute('target') && !el.hasAttribute('download') && route(attr(el, 'href'))
    // dead click: a control that changed nothing (no DOM mutation, no navigation, no focus move) within a second
    if (actionable(e.target) && !FIELDS.test(el.tagName) && (el.tagName !== 'A' || plainLink)) {
      const seen = changes()
      const href = loc().href
      const active = doc.activeElement
      setTimeout(() => {
        if (changes() === seen && loc().href === href && doc.activeElement === active) record('dead', el)
      }, DEAD_MS)
    }
  })
  listen('keydown', e => {
    const el = actionable(e.target)
    if (!el || (e.key !== 'Enter' && e.key !== ' ')) return
    const textual = el.tagName === 'TEXTAREA' || (el.tagName === 'INPUT' && !/^(checkbox|radio|button|submit|reset)$/.test(attr(el, 'type')))
    if (!textual) record('key', el, null, { key: e.key === ' ' ? 'space' : 'enter' })
  })
  const focused = new Map()
  const typed = new Map()
  const flushInputs = () => {
    for (const [el, n] of typed) record('input', el, n)
    typed.clear()
  }
  listen('focusin', e => {
    const el = actionable(e.target)
    if (el) focused.set(el, now())
  })
  listen('focusout', e => {
    const el = actionable(e.target)
    if (!el || !focused.has(el)) return
    if (typed.has(el)) record('input', el, typed.get(el)), typed.delete(el)
    record('focus', el, now() - focused.get(el))
    focused.delete(el)
  })
  listen('input', e => {
    const el = actionable(e.target)
    if (el) typed.set(el, (typed.get(el) || 0) + 1) // one coalesced `input` event per field visit, never the value
  })
  listen('change', e => {
    const el = e.target
    if (!el || el.nodeType !== 1) return
    const kind = attr(el, 'type')
    if (el.tagName === 'INPUT' && (kind === 'checkbox' || kind === 'radio')) record('toggle', el, secret(el) ? null : el.checked ? 1 : 0)
    else if (el.tagName === 'SELECT') record('toggle', el, secret(el) ? null : el.selectedIndex)
    else record('change', el)
  })
  listen('toggle', e => {
    if (e.target && e.target.tagName === 'DETAILS') record('toggle', e.target, e.target.hasAttribute('open') ? 1 : 0)
  })
  const invalid = new WeakSet()
  listen('submit', e => record('submit', e.target, null, { o: 'sent' }))
  listen('invalid', e => {
    const form = e.target && e.target.form
    if (!form || invalid.has(form)) return
    invalid.add(form)
    setTimeout(() => invalid.delete(form), RAGE_MS)
    record('submit', form, null, { o: 'invalid' })
  })
  for (const k of ['copy', 'cut', 'paste', 'drop']) listen(k, e => record(k, actionable(e.target) || e.target))
  listen('dragstart', e => record('drag', actionable(e.target) || e.target))
  listen('contextmenu', e => record('context', actionable(e.target) || e.target))
  for (const a of ['play', 'pause', 'seeked', 'ended'])
    listen(a, e => {
      const el = e.target
      if (el && /^(VIDEO|AUDIO)$/.test(el.tagName)) record('media', el, el.currentTime, { a })
    })
  const hovered = new WeakSet()
  let hover = null
  listen('pointerover', e => {
    const el = actionable(e.target)
    if (!el || hovered.has(el) || (hover && hover.el === el)) return
    if (hover) clearTimeout(hover.t)
    hover = { el, t: setTimeout(() => (hovered.add(el), (hover = null), record('hover', el, HOVER_MS)), HOVER_MS) }
  })
  listen('pointerout', e => {
    if (hover && actionable(e.target) === hover.el) clearTimeout(hover.t), (hover = null)
  })
  listen('scroll', scrolled, win)

  // ── SPA navigation, visibility, page exit
  const history = env.history
  if (history)
    for (const m of ['pushState', 'replaceState']) {
      const orig = history[m]
      if (typeof orig === 'function')
        history[m] = function (...a) {
          const r = orig.apply(this, a)
          navigated()
          return r
        }
    }
  listen('popstate', navigated, win)
  listen('visibilitychange', () => {
    if (visible()) visibleAt = now()
    else (visibleMs += now() - visibleAt), flushInputs(), send()
  })
  listen('pagehide', () => (leave(), send()), win)
  const track = (name, props) => {
    if (!TOKEN.test(String(name))) return
    const p = { name: String(name) }
    for (const [k, v] of Object.entries(props || {}).slice(0, 15)) if (/^\w{1,32}$/.test(k) && v !== Object(v)) p[k] = v
    record('track', null, null, p)
  }
  listen('brain:track', e => e.detail && track(e.detail.name, e.detail.props), win)
  listen('click', e => {
    const el = actionable(e.target)
    const name = el && attr(el, 'data-track')
    if (TOKEN.test(name)) track(name)
  })
  const stop = () => {
    stopped = true
    clearTimeout(timer)
    for (const f of off) f()
    if (mo) mo.disconnect()
  }
  page()
  return { track, send, stop, queue: () => queue, describe }
}
