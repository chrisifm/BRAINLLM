// BRAINLLM client router (INTERNALS §19). Views are prerendered pages, so navigating = fetch the next page, swap its
// region (`main` by default), title and SEO meta, inside a View Transition when available. Islands keep working: their
// resume listeners are delegated on document; islands swapped out are disposed (`$d`, set by the core's resume).
// Navigation API (Baseline 2026) intercepts link navigations and back/forward and restores scroll and
// focus; without it, a click listener + History API do the same. Any failure falls back to a full page load.
// Page contract (dev server and build): the region holds the view and the script that mounts it, which stores the
// dispose on the region (`e.$d = mount(V, e, p)`); the router disposes it before swapping and runs the new one.
// Usage (app entry, emitted by the toolchain): `router()` · options: { region, skip(url) → bool, parse(html) → doc,
// guards }.
// Guards (INTERNALS §22): `guards: { '/admin': ctx => … }` or `guards: { '/admin': '/_b/<h>.js' }`, one per folder with
// `_guard.jsbrain` (the build emits the chunk URL; the router imports it the first time it is needed). Before navigating, every guard whose folder contains
// the target runs, outer to inner, with ctx = { url, path, query }. It allows by returning nothing/true, cancels with
// `false` and redirects with `redirect('/login')` (thrown or returned) or a string. The server runs the same guard,
// so a redirect it answers (fetch ends on another URL) is reflected in the address bar. A guard that throws anything
// else → full page load: the server decides.
import { prefetchPage } from './brain-prefetch-shared.js'
const G = globalThis
// Base path (<meta name="brain-base" content="/app/">, §23 #148): links in the HTML already carry it; guards and
// routes work with app paths (strip), and app paths given to go()/navigate()/redirect() get it (app). '/' when absent.
const base = () => {
  const m = G.document && document.querySelector('meta[name=brain-base]')
  return (m && m.getAttribute('content')) || '/'
}
const strip = p => {
  const b = base()
  return b !== '/' && (p + '/').startsWith(b) ? '/' + p.slice(b.length) : p
}
const app = u => {
  const b = base()
  return typeof u === 'string' && b !== '/' && u.startsWith('/') && !u.startsWith('//') && !(u + '/').startsWith(b) ? b + u.slice(1) : u
}

// navigate('/account', { replace }) (§41.5): an app path (the base is added), through the running router (no reload,
// guards run); without a router it is a normal page load.
let active = null
export function navigate(to, o = {}) {
  const raw = String(to)
  if (/[\\\x00-\x20\x7f]/.test(raw)) throw Error('Invalid navigation target')
  const u = app(raw)
  if (active) return active.go(u, o.replace ? 'replace' : true)
  o.replace ? location.replace(u) : location.assign(u)
}
G.__brainNavigate = navigate // brain-form follows a server function's { redirect } without importing the router

// route() lives in brain-env.js (it needs the core's signals; a page without islands downloads only this router):
// after every client navigation the router calls the hook it installs.
const routed = () => G.__brainRouted && G.__brainRouted()

// Active links (§41.5): every internal <a href> to the current page gets aria-current="page" (the CSS hook is
// [aria-current=page]); the others lose it. On start and after every client navigation.
const current = () => {
  if (!G.document || !G.location) return
  for (const a of document.querySelectorAll('a[href]')) {
    let u
    try {
      u = new URL(a.getAttribute('href'), location.href)
    } catch {
      continue
    }
    if (u.origin === location.origin && u.pathname.replace(/\/$/, '') === location.pathname.replace(/\/$/, '')) a.setAttribute('aria-current', 'page')
    else if (a.getAttribute('aria-current') === 'page') a.removeAttribute('aria-current')
  }
}

// A form() with unsaved changes (brain-form keeps them in globalThis.__brainDirty) asks before leaving (§41.5); one
// being submitted does not (#299): its submit() may navigate on success before it returns
const leave = () => {
  for (const f of G.__brainDirty || []) if (f.dirty && !f.busy) return !G.confirm || G.confirm('Leave this page? Changes you made will not be saved.')
  return true
}

// Same shapes as server.mjs, so one _guard.jsbrain works on both sides: { redirect, status } and { deny }
const R = Symbol('redirect') // older compiled guards
export function redirect(url, status = 303) {
  throw { redirect: String(url), status }
}
export const deny = (reason = 'Forbidden') => ({ deny: String(reason) })
// guard result → true (go on) · false (cancel) · string (redirect) · null (full load: the server decides, e.g. 403)
const outcome = v =>
  v && typeof v === 'object'
    ? R in v
      ? v[R]
      : typeof v.redirect === 'string'
        ? v.redirect
        : v.deny !== undefined
          ? null
          : true
    : v === false || typeof v === 'string'
      ? v
      : true

export function router(o = {}) {
  const region = o.region || 'main'
  const parse = o.parse || (h => new DOMParser().parseFromString(h, 'text/html'))
  const fullLoad = new Set(Array.isArray(o.fullLoad) ? o.fullLoad.map(p => { try { return new URL(p, location.href).pathname.replace(/\/+$/, '') || '/' } catch { return null } }).filter(Boolean) : [])
  let seq = 0, through = null, stale = false
  const ahead = predict(o, u => same(u, true))
  // Build signature (§35.7): the page's <meta name="brain-build">. A navigation response from another build, or the
  // service worker announcing one, turns client navigation into full loads so old and new code never mix.
  const build = () => {
    const m = document.querySelector('meta[name=brain-build]')
    return m ? m.getAttribute('content') : null
  }
  const sw = G.navigator && navigator.serviceWorker
  const onMsg = e => typeof e.data === 'string' && e.data.startsWith('brain:build ') && build() && e.data.slice(12) !== build() && (stale = true, ahead.stop())
  if (sw && sw.addEventListener) sw.addEventListener('message', onMsg)
  // a guard is a function, or the URL of its `_guard.jsbrain` chunk (BUILD #220): loaded the first time a navigation
  // reaches that folder, then cached by the module map; its default export runs with the same ctx
  const gs = Object.entries(o.guards || {})
    .map(([pre, g]) => [pre, typeof g === 'string' ? c => import(g).then(m => m.default(c)) : g])
    .sort((a, b) => a[0].length - b[0].length)
  current()
  // → true (go on) · false (cancel) · string (redirect) · null (guard failed: full load)
  async function check(u) {
    const url = new URL(u, location.href), path = strip(url.pathname) // guards see app paths (no base)
    for (const [pre, g] of gs) {
      if (!(path === pre || path.startsWith(pre.endsWith('/') ? pre : pre + '/'))) continue
      // The browser has no session (the cookie is HttpOnly): a guard that reads ctx.user cannot decide here. If it would
      // stop the navigation, the server decides on a full load instead (F12b: `ctx.user ? true : redirect('/login')`
      // used to send a signed-in user to /login on every client navigation).
      let v, asked = false
      const ctx = { url, path, query: Object.fromEntries(url.searchParams) }
      Object.defineProperty(ctx, 'user', { enumerable: true, get: () => ((asked = true), undefined) })
      try {
        v = await g(ctx)
      } catch (e) {
        if (asked) return null
        return e && typeof e === 'object' && (R in e || 'redirect' in e) ? outcome(e) : null
      }
      v = outcome(v)
      if (v !== true) return asked ? null : v
    }
    return true
  }

  // _loading.brain (#235): when the next page takes longer than 150 ms, the nearest loading view of the target route
  // (`loading: [{ rx: string | string[], view }]`, most specific first, emitted by the build with the `core` URL) is mounted in the
  // region until the page arrives; fast navigations never show it, so nothing flickers.
  const loads = o.loading || []
  function waiting(url, my) {
    const path = strip(new URL(url, location.href).pathname), l = loads.find(x => (Array.isArray(x.rx) ? x.rx : [x.rx]).some(rx => new RegExp(rx).test(path)))
    const w = { done: false, t: 0 }
    if (!l || !o.core) return w
    w.t = setTimeout(() => {
      Promise.all([import(o.core), import(l.view)])
        .then(([c, v]) => {
          const el = document.querySelector(region), mount = c.m || c.mount
          if (w.done || my !== seq || !el || !mount) return // the page arrived first
          for (const e of [el, ...walkAll(el)]) if (e.$d) e.$d(), (e.$d = null)
          for (const n of [...el.childNodes]) if (!/^(STYLE|LINK)$/.test(n.nodeName)) n.remove() // the view's CSS stays (BUILD #35)
          el.$d = mount(v.default, el, {})
        })
        .catch(() => {})
    }, 150)
    return w
  }

  async function load(url) {
    const my = ++seq
    let doc, r
    if (stale) return hard(url)
    const slow = waiting(url, my)
    // the previous route's background requests (prefetch) are dropped by brain-data's queue (§41.4)
    try {
      G.dispatchEvent && G.dispatchEvent(new Event('brain:navigate'))
    } catch {}
    try {
      r = (await ahead.take(url)) || (await fetch(url, { headers: { 'x-brain-nav': '1' } }))
      const b = r.headers && r.headers.get && r.headers.get('x-brain-build')
      if (b && build() && b !== build()) return void (my === seq && hard(url)) // a new deploy: load it whole
      const ct = (r.headers && r.headers.get && r.headers.get('content-type')) || 'text/html'
      if (!r.ok || !/html/.test(ct)) throw Error(r.status)
      doc = parse(await r.text())
    } catch {
      return void ((slow.done = true), clearTimeout(slow.t), my === seq && hard(url))
    }
    slow.done = true
    clearTimeout(slow.t)
    if (my !== seq) return // a newer navigation won
    // another language (§37): a full load brings its own dictionaries, `lang` and `dir`; locales never mix in memory
    const lang = d => d && d.documentElement && d.documentElement.getAttribute('lang')
    if (lang(doc) && lang(document) && lang(doc) !== lang(document)) return hard(url)
    if (r.redirected && r.url && G.history && r.url !== new URL(url, location.href).href) history.replaceState(history.state, '', r.url)
    const cur = document.querySelector(region), next = doc.querySelector(region)
    if (!cur || !next) return hard(url)
    const swap = () => {
      // read before the swap: afterwards the fetched page's scripts are in the document too
      const own = document.querySelector('script[nonce]'), nonce = own && (own.nonce || own.getAttribute('nonce'))
      for (const el of [cur, ...walkAll(cur)]) if (el.$d) el.$d(), (el.$d = null)
      cur.replaceChildren(...[...next.childNodes].map(n => (document.adoptNode ? document.adoptNode(n) : n)))
      // Scripts parsed by DOMParser never run: the region's own scripts (the view's mount, `e.$d = mount(...)`) are
      // recreated so they execute. Already-imported modules come from the module map: their state survives.
      // The page's CSP only trusts its own nonce: a recreated script carries the current page's nonce, never the one
      // that came with the fetched page (each server response has a fresh one).
      for (const sc of cur.querySelectorAll('script')) {
        if (sc.type && !/^(module|text\/javascript)$/.test(sc.type)) continue
        const n = document.createElement('script')
        for (const a of sc.attributes) if (a.name !== 'nonce') n.setAttribute(a.name, a.value)
        if (nonce) (n.nonce = nonce), n.setAttribute('nonce', nonce)
        n.textContent = sc.textContent
        sc.replaceWith(n)
      }
      // QA #291: the view's mount script stores its dispose (`e.$d = mount(...)`) after the swap, since module scripts
      // run later; until then the region shows the server's HTML with no handlers. Clicks and submits there are held and
      // replayed on the mounted view, by position, so the first click after a navigation is never lost.
      if (!cur.$d && [...cur.querySelectorAll('script[type=module]')].some(sc => /\$d\s*=/.test(sc.textContent))) hold(cur)
      for (const a of next.attributes || []) cur.setAttribute(a.name, a.value)
      if (doc.title) document.title = doc.title
      // SEO (§32, S04): the route owns description, og:*, twitter:*, robots, JSON-LD, canonical, hreflang and Markdown
      // alternates, prev/next and its route params (brain-params, read by route()): the current page's go, the next
      // page's come (none when it has none). Site links (icons, stylesheets, the RSS feed) and the region's own tags stay.
      const tags = (r, skip) =>
        [...((r && r.querySelectorAll && r.querySelectorAll('meta,script,link')) || [])].filter(e => {
          for (let p = e.parentNode; p; p = p.parentNode) if (p === skip) return false
          const a = n => e.getAttribute(n) || '', t = e.tagName
          return t == 'SCRIPT' ? a('type') == 'application/ld+json'
            : t == 'LINK' ? /^(canonical|prev|next)$/.test(a('rel')) || (a('rel') == 'alternate' && (a('hreflang') || a('type') == 'text/markdown'))
            : /^(og:|twitter:|robots$|description$|brain-params$)/.test(a('property') || a('name'))
        })
      if (document.head) {
        for (const e of tags(document.head)) e.remove()
        for (const e of tags(doc.head || doc, next)) document.head.appendChild(document.adoptNode ? document.adoptNode(e) : e)
      }
      routed()
      current()
      const oldPlan = document.getElementById('brain-next'), newPlan = doc.querySelector('#brain-next')
      if (oldPlan) oldPlan.remove()
      if (newPlan) document.body.appendChild(document.adoptNode ? document.adoptNode(newPlan) : newPlan)
      ahead.scan()
    }
    if (document.startViewTransition) {
      const vt = document.startViewTransition(swap)
      vt.ready.catch(() => {}) // rejects when a faster navigation skips this transition: expected, not an error
      await vt.updateCallbackDone
    } else swap()
  }
  // a full load must not be intercepted again (Navigation API): `through` lets that one navigation pass
  const hard = url => ((through = new URL(url, location.href).href), (o.hard || (u => location.assign(u)))(url))
  const same = (u, prefetch = false) => {
    try {
      const x = new URL(u, location.href), b = base()
      // same origin and inside the app's base path (§23 #148); anything else is a full load
      return x.origin === location.origin && (x.pathname + '/').startsWith(b) && !(o.skip && o.skip(x)) && (prefetch || !fullLoad.has(x.pathname.replace(/\/+$/, '') || '/'))
    } catch {
      return false
    }
  }

  const nav = G.navigation
  if (nav && nav.addEventListener) {
    const h = e => {
      if (through && e.destination.url === through) return void (through = null)
      if (e.navigationType === 'reload') return // a reload is always a real full load (e.g. after a deploy deleted a chunk)
      if (!e.canIntercept || e.hashChange || e.downloadRequest || e.formData || !same(e.destination.url)) return
      if (e.cancelable && !leave()) return e.preventDefault()
      const to = e.destination.url, from = nav.currentEntry && nav.currentEntry.url
      if (G.__brainURLOnly?.(from || location.href, to)) return e.intercept({ handler: () => G.__brainURLRestore?.() })
      if (!gs.length) return e.intercept({ handler: () => load(to) }) // scroll + focus reset handled by the browser
      let g
      e.intercept({
        // Chrome 141+: decided before the URL commits (cancel = the address bar never changes; native redirect).
        async precommitHandler(c) {
          g = await check(to)
          if (typeof g === 'string' && c.redirect) c.redirect(new URL(app(g), location.href).href), (g = true)
          else if (g === false) throw new DOMException('guard', 'AbortError')
        },
        async handler() {
          if (g === undefined) g = await check(to) // no precommit support: the URL already committed
          if (g === true) return load(nav.currentEntry ? nav.currentEntry.url : to)
          if (g === null) return hard(to)
          nav.navigate(g === false ? from : new URL(app(g), location.href).href, { history: 'replace' })
        },
      })
    }
    nav.addEventListener('navigate', h)
    ahead.scan()
    return (active = {
      go: (u, how) => nav.navigate(app(u), how === 'replace' ? { history: 'replace' } : undefined),
      check,
      stop: () => ((active = null), ahead.stop(), nav.removeEventListener('navigate', h), sw && sw.removeEventListener && sw.removeEventListener('message', onMsg)),
    })
  }

  // Fallback: History API. Scroll position is kept in history.state so back/forward restores it.
  const save = () => history.replaceState({ ...(history.state || {}), y: G.scrollY || 0 }, '')
  let here = location.href
  const go = async (u, push = true, hops = 0) => {
    if (!hops && !leave()) return push || history.pushState(history.state, '', here) // stay (back/forward: restore the URL)
    if (!same(u)) return hard(u)
    const g = gs.length ? await check(u) : true
    if (g === null || hops > 5) return hard(u)
    if (g === false) return push || history.pushState(history.state, '', here) // back/forward blocked: stay
    if (g !== true) return go(new URL(app(g), location.href).href, push, hops + 1)
    if (push) save(), history[push === 'replace' ? 'replaceState' : 'pushState']({ y: 0 }, '', u) // push: true | 'replace' | false (back/forward)
    await load(u)
    here = location.href
    const hash = new URL(u, location.href).hash, t = hash && document.getElementById(hash.slice(1))
    if (t) t.scrollIntoView()
    else G.scrollTo && G.scrollTo(0, push ? 0 : (history.state && history.state.y) || 0)
    focusMain()
  }
  const click = e => {
    if (e.defaultPrevented || e.button || e.metaKey || e.ctrlKey || e.shiftKey || e.altKey) return
    let a = e.target
    while (a && a.tagName !== 'A') a = a.parentNode
    if (!a || !a.getAttribute('href') || a.getAttribute('target') || a.hasAttribute('download') || !same(a.href || a.getAttribute('href'))) return
    const u = new URL(a.href || a.getAttribute('href'), location.href)
    if (u.pathname === location.pathname && u.hash) return // in-page anchor: native
    e.preventDefault()
    go(u.href)
  }
  const pop = () => {
    if (G.__brainURLOnly?.(here, location.href)) { here = location.href; G.__brainURLRestore?.(); return }
    return go(location.href, false)
  }
  const focusMain = () => {
    const m = document.querySelector(region), h = m && (m.querySelector('h1') || m)
    if (h && h.focus) {
      if (!h.hasAttribute('tabindex')) h.setAttribute('tabindex', '-1')
      h.focus({ preventScroll: true }) // screen readers announce the new view
    }
  }
  document.addEventListener('click', click)
  G.addEventListener && G.addEventListener('popstate', pop)
  ahead.scan()
  return (active = {
    go: (u, push = true) => go(app(u), push),
    check,
    stop: () => ((active = null), ahead.stop(), document.removeEventListener('click', click), G.removeEventListener && G.removeEventListener('popstate', pop)),
  })
}

// Predictive prefetch (§19, 2026-10-29): no dead time during a visit. Every internal link that scrolls into view is
// fetched in the background while the browser is idle (at most 2 at a time, 12 per page), and hovering, focusing or
// touching a link moves it to the front of the queue. The fetched page is kept in memory and the next navigation to it
// swaps without waiting for the network; its module chunks get `<link rel=modulepreload>` so the view mounts at once.
// The current view never waits: requests run at low priority and only when idle. Skipped: Save-Data and 2G (only
// intent prefetches there, and never with Save-Data), `data-prefetch="false"` on the link or an ancestor, download or
// target links, files (`.pdf`, `.zip`…), `/api/` and `/auth/`, action-style paths (log out, unsubscribe, confirm,
// verify), URLs carrying a token in the query, and everything with `router({ prefetch: false })` or
// `<meta name="brain-prefetch" content="off">`. Private/no-store and dynamic no-cache HTML is never reused; static
// pages and explicitly cacheable public HTML stay in bounded tab memory for at most 5 min,
// and any write empties the memory first: a form submit, a `brain:write` (brain-data writes, every server function
// call, log-out included), so the next view never shows data from before the write or another session. Each fetch says
// `x-brain-prefetch: 1` (`ctx.prefetch` in the server's `load`), so a server can tell it apart from a click.
const FRESH = 300000, MAX_PAGE = 12, MAX_KEPT = 24, PARALLEL = 2
const NOPE = /(^|\/)(log-?out|sign-?out|logoff|unsubscribe|confirm|verify|api|auth|_brain|_r)(\.html?)?(\/|$)/i
const FILE = /\.(?!html?$)[a-z0-9]{2,5}$/i
const SECRET = /(^|[?&])(token|code|key|sig|signature|auth|otp|state)=/i
export function predict(o = {}, same = () => false) {
  const off = { scan() {}, stop() {}, take: async () => null }
  const meta = G.document && document.querySelector && document.querySelector('meta[name=brain-prefetch]')
  if (o.prefetch === false || (meta && /^(off|false|0)$/i.test(meta.getAttribute('content') || '')) || !G.document || !G.fetch) return off
  const net = G.navigator && navigator.connection
  const thin = () => !!net && (net.saveData || /(^|-)2g$/.test(net.effectiveType || ''))
  const cache = new Map(), queue = [], pre = new Map(), running = new Map(), pending = new Map(), planned = new Map()
  let seen = new WeakSet(), busy = 0, count = 0, io = null, idle = 0, epoch = 0, dead = false, bytes = 0, worker = null, serial = 0
  const key = h => {
    try { const u = new URL(h, location.href); u.hash = ''; return u.href } catch { return null }
  }
  const safe = k => {
    if (!k || !same(k) || k === key(location.href)) return false
    const u = new URL(k)
    return !NOPE.test(u.pathname) && !FILE.test(u.pathname) && !SECRET.test(u.search)
  }
  const ok = a => {
    if (!a || !a.getAttribute || !a.getAttribute('href') || a.getAttribute('target') || a.hasAttribute('download')) return null
    for (let n = a; n && n.getAttribute; n = n.parentNode) if (n.getAttribute('data-prefetch') === 'false') return null
    const k = key(a.href || a.getAttribute('href'))
    return safe(k) ? k : null
  }
  const warm = list => {
    if (!document.head || dead) return
    let n = 0
    for (const href of list) {
      const u = key(href)
      if (!u || pre.has(u) || !same(u) || !new URL(u).pathname.includes('/_b/') || ++n > 8) continue
      const l = document.createElement('link')
      l.setAttribute('rel', /\.(js|jsbrain)(\?|$)/.test(u) ? 'modulepreload' : 'prefetch')
      l.setAttribute('href', u)
      l.setAttribute('fetchpriority', 'low')
      document.head.appendChild(l)
      pre.set(u, l)
      while (pre.size > 48) { const [k, node] = pre.entries().next().value; node.remove(); pre.delete(k) }
    }
  }
  const remove = k => { const c = cache.get(k); if (c) bytes -= c.bytes || 0; cache.delete(k) }
  const prune = () => {
    for (const [k, c] of cache) if (Date.now() - c.at > FRESH) remove(k)
    while (cache.size > MAX_KEPT || bytes > 4 * 1024 * 1024) remove(cache.keys().next().value)
  }
  const crashed = () => {
    worker && worker.terminate(); worker = null
    for (const p of pending.values()) { clearTimeout(p.timer); p.resolve(null) }
    pending.clear()
  }
  if (G.Worker && typeof o.prefetchWorker === 'string') {
    try {
      const url = new URL(o.prefetchWorker, location.href)
      if (url.origin === location.origin) {
        worker = new Worker(url.href, { type: 'module' })
        worker.onmessage = e => {
          const m = e.data, p = m && pending.get(m.id)
          if (!p) return
          clearTimeout(p.timer); pending.delete(m.id); p.resolve(m.ok ? m : null)
        }
        worker.onerror = crashed
        worker.onmessageerror = crashed
      }
    } catch { crashed() }
  }
  const background = (k, controller) => {
    if (worker) return new Promise(resolve => {
      const id = ++serial
      const done = () => { const p = pending.get(id); if (p) { clearTimeout(p.timer); pending.delete(id); resolve(null); worker && worker.postMessage({ id, cancel: true }) } }
      const timer = setTimeout(done, 6500)
      pending.set(id, { resolve, timer })
      controller.signal.addEventListener('abort', done, { once: true })
      try { worker.postMessage({ id, url: k }) } catch { done(); crashed() }
    })
    return prefetchPage(k, controller.signal)
  }

  const get = k => {
    const own = epoch, controller = new AbortController(), at = Date.now()
    running.set(k, controller)
    const timer = setTimeout(() => controller.abort(), 6000)
    const c = { at, epoch: own, bytes: 0, p: null }
    c.p = background(k, controller).then(m => {
      if (!m || dead || own !== epoch || controller.signal.aborted) { if (cache.get(k) === c) remove(k); return null }
      const headers = Array.isArray(m.headers) ? new Headers(m.headers) : m.headers
      const cc = headers.get('cache-control') || '', freshness = /(?:^|,)\s*(?:s-maxage|max-age)=(\d+)/i.exec(cc)
      if (m.assets) warm(m.assets)
      if (planned.has(k)) warm(planned.get(k))
      // Personalized and dynamically revalidated HTML always goes back through the server on navigation.
      // Speculation still warms its immutable code/assets; it never reuses another tab's prior session data.
      if (/private|no-store/i.test(cc) || /no-cache/i.test(cc) && headers.get('x-brain-static') !== '1') { if (cache.get(k) === c) remove(k); return null }
      // Transient page memory belongs to this tab and write epoch, never to CacheStorage or the worker.
      c.bytes = m.text.length * 2
      if (cache.get(k) === c) bytes += c.bytes
      prune()
      return { ok: m.ok, status: m.status, redirected: false, url: m.url, headers, text: async () => m.text, ttl: Math.min(FRESH, freshness ? +freshness[1] * 1000 : FRESH) }
    }).catch(() => null).finally(() => { clearTimeout(timer); if (running.get(k) === controller) running.delete(k) })
    cache.set(k, c); prune()
    return c.p
  }
  const pump = () => {
    if (dead || document.hidden || net && net.saveData) return
    while (busy < PARALLEL && queue.length) {
      const k = queue.shift()
      if (cache.has(k)) continue
      busy++
      get(k).finally(() => { busy--; pump() })
    }
  }
  const later = f => G.requestIdleCallback ? G.requestIdleCallback(f, { timeout: 2000 }) : setTimeout(f, 200)
  const cancelIdle = () => { if (idle) { G.cancelIdleCallback ? G.cancelIdleCallback(idle) : clearTimeout(idle); idle = 0 } }
  const want = (k, now) => {
    if (!k || dead || cache.has(k) || count >= (now ? MAX_KEPT : MAX_PAGE)) return
    const i = queue.indexOf(k)
    if (i >= 0) queue.splice(i, 1); else count++
    if (now) { queue.unshift(k); pump() }
    else { queue.push(k); if (!idle) idle = later(() => { idle = 0; pump() }) }
  }
  const intent = e => {
    if (net && net.saveData) return
    let a = e.target
    while (a && a.tagName !== 'A') a = a.parentNode
    want(ok(a), true)
  }
  const evs = ['pointerover', 'focusin', 'touchstart']
  for (const t of evs) document.addEventListener(t, intent, { passive: true, capture: true })
  const forget = () => {
    epoch++; cancelIdle(); queue.length = 0; cache.clear(); bytes = 0; count = 0
    for (const c of running.values()) c.abort()
    running.clear()
  }
  document.addEventListener('submit', forget, { capture: true })
  if (G.addEventListener) G.addEventListener('brain:write', forget)
  const visible = () => { if (!document.hidden) pump() }
  document.addEventListener('visibilitychange', visible)
  if (G.IntersectionObserver) io = new IntersectionObserver(es => {
    for (const e of es) if (e.isIntersecting) { io.unobserve(e.target); if (!thin()) want(ok(e.target), false) }
  }, { rootMargin: '200px' })
  return {
    scan() {
      if (dead) return
      count = 0; cancelIdle(); queue.length = 0; planned.clear(); seen = new WeakSet(); io && io.disconnect()
      const anchors = document.querySelectorAll('a[href]'), allowed = new Set()
      for (const a of anchors) { const k = ok(a); if (k) allowed.add(k); if (io && !seen.has(a)) { seen.add(a); io.observe(a) } }
      // Compiler-generated graph, validated against the current DOM: hidden/opted-out links cannot sneak in.
      const plan = document.getElementById && document.getElementById('brain-next')
      if (plan && !thin()) {
        try {
          const map = JSON.parse(plan.textContent)
          if (map.v === 1 && Array.isArray(map.next)) for (const item of map.next.slice(0, MAX_PAGE)) {
            const k = key(item.url)
            if (allowed.has(k) && safe(k)) { if (Array.isArray(item.assets)) planned.set(k, item.assets.slice(0, 16)); want(k, false) }
          }
        } catch {}
      }
    },
    stop() {
      if (dead) return
      forget(); dead = true; crashed(); io && io.disconnect()
      for (const t of evs) document.removeEventListener(t, intent, { capture: true })
      document.removeEventListener('submit', forget, { capture: true })
      document.removeEventListener('visibilitychange', visible)
      G.removeEventListener && G.removeEventListener('brain:write', forget)
      for (const l of pre.values()) l.remove()
      pre.clear()
    },
    async take(u) {
      const k = key(u), c = k && cache.get(k)
      if (!c) return null
      remove(k)
      // A click must not inherit a slow speculative request's six-second deadline.
      let timer
      const r = await Promise.race([c.p, new Promise(resolve => { timer = setTimeout(() => resolve(null), 150) })])
      clearTimeout(timer)
      if (!r) { const controller = running.get(k); controller && controller.abort() }
      return r && !dead && c.epoch === epoch && Date.now() - c.at < r.ttl ? r : null
    },
  }
}

const HOLD = ['click', 'submit']
function hold(el) {
  if (el.$hold) el.$hold() // a newer navigation drops what the previous page held
  const q = []
  const keep = e => {
    const path = []
    for (let n = e.target; n && n !== el; n = n.parentNode) path.unshift([].indexOf.call(n.parentNode.childNodes, n))
    q.push([e, path])
    e.preventDefault()
    e.stopImmediatePropagation()
  }
  const done = () => {
    clearTimeout(t)
    for (const k of HOLD) el.removeEventListener(k, keep, true)
    delete el.$d
    el.$hold = null
  }
  // a page whose script never mounts (it failed to load) stops holding after 5 s: later clicks act on the HTML as is
  const t = setTimeout(done, 5000)
  for (const k of HOLD) el.addEventListener(k, keep, true)
  el.$hold = done
  Object.defineProperty(el, '$d', {
    configurable: true,
    get: () => undefined,
    set(v) {
      done()
      el.$d = v
      for (const [e, path] of q) {
        let n = el
        for (const i of path) n = n && n.childNodes[i]
        if (n) n.dispatchEvent(new e.constructor(e.type, e))
      }
    },
  })
}
function walkAll(n, out = []) {
  for (const c of n.childNodes) if (c.nodeType === 1) out.push(c), walkAll(c, out)
  return out
}
