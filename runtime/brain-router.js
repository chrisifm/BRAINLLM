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
const base = () => G.document?.querySelector('meta[name=brain-base]')?.getAttribute('content') || '/'
const strip = p => {
  const b = base()
  return b !== '/' && (p + '/').startsWith(b) ? '/' + p.slice(b.length) : p
}
const app = u => {
  const b = base()
  u = G.__brainHref?.(u) ?? u // the page's language (brain-links.js)
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
const routed = () => G.__brainRouted?.()

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
export function redirect(url, status = 303) {
  throw { redirect: String(url), status }
}
export const deny = (reason = 'Forbidden') => ({ deny: String(reason) })
// guard result → true (go on) · false (cancel) · string (redirect) · null (full load: the server decides, e.g. 403)
const outcome = v =>
  v && typeof v === 'object'
    ? typeof v.redirect === 'string'
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
  const fullLoad = new Set((o.fullLoad || []).map(p => new URL(p, location.href).pathname.replace(/\/+$/, '') || '/'))
  let seq = 0, through = null, stale = false
  const ahead = predict(o, (u, p = true) => same(u, p))
  // Build signature (§35.7): the page's <meta name="brain-build">. A navigation response from another build, or the
  // service worker announcing one, turns client navigation into full loads so old and new code never mix.
  const build = () => document.querySelector('meta[name=brain-build]')?.getAttribute('content')
  // The page's CSP nonce, read once while the document is still the server's: later, data scripts adopted from fetched
  // pages (JSON-LD, b/state) carry their own response's nonce and could come first, and a mount script recreated with
  // that nonce is blocked by the CSP.
  const own = document.querySelector('script[nonce]'), nonce = own?.nonce || own?.getAttribute('nonce')
  const sw = G.navigator?.serviceWorker
  const onMsg = e => /^brain:build /.test(e.data) && build() && e.data.slice(12) !== build() && (stale = true, ahead.stop())
  sw?.addEventListener?.('message', onMsg)
  // both routers stop through here, so neither keeps the service worker listener alive (B05)
  const quit = () => ((active = null), ahead.stop(), sw?.removeEventListener?.('message', onMsg))
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
        return typeof e?.redirect === 'string' ? e.redirect : null
      }
      v = outcome(v)
      if (v !== true) return asked ? null : v
    }
    return true
  }

  // Feedback (§19): a navigation marks the region aria-busy and route().navigating at once; past 150 ms it shows
  // #brain-nav: the app's own element with that id (a spinner in app.brain, `<div id="brain-nav" hidden>`) or a native
  // indeterminate <progress> bar at the top (label "Loading"; colored by accent-color on #brain-nav, no extra
  // DOM), so fast ones show nothing. _loading.brain (#235): past 400 ms the nearest loading view of the target route
  // (`loading: [{ rx, view }]`, most specific first, emitted by the build with the `core` URL) replaces the region until
  // the page arrives. Until then the current page stays as it is.
  const loads = o.loading || [], adopt = n => document.adoptNode?.(n) || n
  // the previous page's click hold ends with it, mounted or not
  const dispose = r => { r.$hold?.(); for (const e of [r, ...r.querySelectorAll('*')]) if (e.$d) e.$d(), (e.$d = null) }
  const fixed = (t, id, css) => {
    const b = document.body.appendChild(document.createElement(t))
    b.id = id
    b.style.cssText = 'position:fixed;' + css
    return b
  }
  const ui = on => {
    let b = document.getElementById('brain-nav')
    if (!b && on) (b = fixed('progress', 'brain-nav', 'top:0;left:0;width:100%;height:3px')).ariaLabel = 'Loading'
    if (b) b.hidden = !on
  }
  const busy = (my, on) => {
    const el = my === seq && document.querySelector(region)
    if (!el) return
    on ? el.setAttribute('aria-busy', 'true') : (el.removeAttribute('aria-busy'), ui())
    G.__brainRouted?.(on)
  }
  function waiting(url, my) {
    const path = strip(new URL(url, location.href).pathname), l = loads.find(x => [x.rx].flat().some(rx => new RegExp(rx).test(path)))
    const w = {}, at = (ms, f) => setTimeout(() => my === seq && !w.done && f(), ms)
    at(150, () => ui(1))
    at(8e3, () => (seq++, hard(url))) // a request that never answers (a hung prefetch included) ends in a full load
    if (l && o.core)
      at(400, () =>
        Promise.all([import(o.core), import(l.view)])
          .then(([c, v]) => {
            const el = document.querySelector(region), mount = c.m || c.mount
            if (w.done || my !== seq || !el || !mount) return // the page arrived first
            dispose(el)
            for (const n of [...el.childNodes]) if (!/^(STYLE|LINK)$/.test(n.nodeName)) n.remove() // the view's CSS stays (BUILD #35)
            el.$d = mount(v.default, el, {})
          })
          .catch(() => {}),
      )
    return w
  }

  let navFetch // the navigation in flight: a newer one aborts its fetch
  // → true when this navigation swapped the region (a newer one, a full load or a failure → falsy)
  async function load(url) {
    const my = ++seq
    if (stale) return hard(url)
    navFetch?.abort()
    const ctl = (navFetch = new AbortController())
    const slow = waiting(url, my)
    busy(my, 1)
    // the previous route's background requests (prefetch) are dropped by brain-data's queue (§41.4)
    G.dispatchEvent?.(new Event('brain:navigate'))
    try {
      const r = (await ahead.take(url)) || (await fetch(url, { headers: { 'x-brain-nav': '1' }, signal: ctl.signal }))
      if (my !== seq) return
      const hd = k => r.headers?.get?.(k), b = hd('x-brain-build')
      if (b && build() && b !== build()) return hard(url) // a new deploy: load it whole
      if (!r.ok || !/html/.test(hd('content-type') || 'html')) throw Error(r.status)
      const doc = parse(await r.text())
      if (my !== seq) return // a newer navigation won
      // another language (§37): a full load brings its own dictionaries, `lang` and `dir`; locales never mix in memory
      const lang = d => d?.documentElement?.getAttribute('lang')
      if (lang(doc) && lang(document) && lang(doc) !== lang(document)) return hard(url)
      if (r.redirected && r.url && r.url !== new URL(url, location.href).href) history.replaceState(history.state, '', r.url)
      const cur = document.querySelector(region), next = doc.querySelector(region)
      if (!cur || !next) return hard(url)
      // DOM, head and active links change in one callback (one frame); a newer navigation that started meanwhile skips it
      const swap = () => {
        if (my !== seq) return
        dispose(cur)
        cur.replaceChildren(...[...next.childNodes].map(adopt))
        // Scripts parsed by DOMParser never run: the region's own scripts (the view's mount, `e.$d = mount(...)`) are
        // recreated so they execute. Already-imported modules come from the module map: their state survives.
        // The page's CSP only trusts its own nonce: a recreated script carries the current page's nonce, never the one
        // that came with the fetched page (each server response has a fresh one).
        for (const sc of cur.querySelectorAll('script')) {
          if (sc.type && !/^(module|text\/javascript)$/.test(sc.type)) continue
          const n = document.createElement('script')
          for (const a of sc.attributes) if (a.name !== 'nonce') n.setAttribute(a.name, a.value)
          if (nonce) n.setAttribute('nonce', nonce)
          n.textContent = sc.textContent
          sc.replaceWith(n)
        }
        // QA #291: the view's mount script stores its dispose (`e.$d = mount(...)`) after the swap, since module scripts
        // run later; until then the region shows the server's HTML with no handlers. Clicks and submits there are held and
        // replayed on the mounted view, by position, so the first click after a navigation is never lost.
        if (!cur.$d && [...cur.querySelectorAll('script[type=module]')].some(sc => /\$d\s*=/.test(sc.textContent))) hold(cur)
        for (const a of next.attributes) cur.setAttribute(a.name, a.value)
        if (doc.title) document.title = doc.title
        // SEO (§32, S04): the route owns description, og:*, twitter:*, robots, JSON-LD, canonical, hreflang and Markdown
        // alternates, prev/next and its route params (brain-params, read by route()): the current page's go, the next
        // page's come (none when it has none). Site links (icons, stylesheets, the RSS feed) and the region's own tags stay.
        const tags = (r, skip) =>
          [...(r.querySelectorAll?.('meta,script,link') || [])].filter(e => {
            for (let p = e.parentNode; p; p = p.parentNode) if (p === skip) return false
            const a = n => e.getAttribute(n) || '', t = e.tagName
            return t == 'SCRIPT' ? a('type') == 'application/ld+json'
              : t == 'LINK' ? /^(canonical|prev|next)$/.test(a('rel')) || (a('rel') == 'alternate' && (a('hreflang') || a('type') == 'text/markdown'))
              : /^(og:|twitter:|robots$|description$|brain-params$)/.test(a('property') || a('name'))
          })
        for (const e of tags(document.head)) e.remove()
        for (const e of tags(doc.head || doc, next)) document.head.appendChild(adopt(e))
        routed()
        current()
        document.getElementById('brain-next')?.remove()
        const plan = doc.querySelector('#brain-next')
        if (plan) document.body.appendChild(adopt(plan))
        ahead.scan()
      }
      // View Transition when the browser has it (an app turns it off under prefers-reduced-motion with CSS, §19)
      if (document.startViewTransition) {
        const vt = document.startViewTransition(swap)
        vt.ready.catch(() => {}) // rejects when a faster navigation skips this transition: expected, not an error
        await vt.updateCallbackDone
      } else swap()
      return my === seq
    } catch {
      // a failed fetch, a parse or swap that threw: never a half-swapped page, the browser loads it whole
      my === seq && hard(url)
    } finally {
      slow.done = 1
      busy(my, 0)
    }
  }
  // After a swap, for assistive technology (§19): focus moves to the URL's target, else the page's h1, else the region,
  // unless the new page already holds focus; #brain-announce (polite, created at start) reads the new title.
  const place = (u, lost) => {
    const m = document.querySelector(region), a = document.activeElement
    if (!m || (a && a !== document.body && a !== m && (lost || m.contains(a)))) return
    const id = new URL(u, location.href).hash.slice(1), h = (id && document.getElementById(id)) || m.querySelector('h1') || m
    h.hasAttribute('tabindex') || h.setAttribute('tabindex', '-1')
    h.focus({ preventScroll: true })
  }
  const say = t => {
    let a = document.getElementById('brain-announce')
    if (!a) (a = fixed('div', 'brain-announce', 'clip-path:inset(50%)')).setAttribute('aria-live', 'polite')
    a.textContent = t
  }
  say('')
  const landed = u => {
    place(u)
    say(document.title)
    const m = document.querySelector(region)
    if (m?.$hold) m.$mounted = () => place(u, 1) // mounting replaces the server's nodes: a focus lost there comes back
  }
  // a full load must not be intercepted again (Navigation API): `through` lets that one navigation pass
  const hard = url => ((through = new URL(url, location.href).href), (o.hard || (u => location.assign(u)))(url))
  const same = (u, prefetch = false) => {
    try {
      const x = new URL(u, location.href), b = base()
      // same origin and inside the app's base path (§23 #148); anything else is a full load
      return x.origin === location.origin && (x.pathname + '/').startsWith(b) && !o.skip?.(x) && (prefetch || !fullLoad.has(x.pathname.replace(/\/+$/, '') || '/'))
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
      const to = e.destination.url, from = nav.currentEntry?.url
      if (G.__brainURLOnly?.(from || location.href, to)) return e.intercept({ focusReset: 'manual', handler: () => G.__brainURLRestore?.() })
      const shown = u => load(u).then(ok => ok && landed(u)) // the browser restores scroll; focus is ours (it would reset to body)
      if (!gs.length) return e.intercept({ focusReset: 'manual', handler: () => shown(to) })
      let g
      e.intercept({
        focusReset: 'manual',
        // Chrome 141+: decided before the URL commits (cancel = the address bar never changes; native redirect).
        async precommitHandler(c) {
          g = await check(to)
          if (typeof g === 'string' && c.redirect) c.redirect(new URL(app(g), location.href).href), (g = true)
          else if (g === false) throw new DOMException('guard', 'AbortError')
        },
        async handler() {
          if (g === undefined) g = await check(to) // no precommit support: the URL already committed
          if (g === true) return shown(nav.currentEntry ? nav.currentEntry.url : to)
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
      stop: () => (quit(), nav.removeEventListener('navigate', h)),
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
    if (!(await load(u))) return // a newer navigation (or a full load) owns scroll and focus now
    here = location.href
    const hash = new URL(u, location.href).hash, t = hash && document.getElementById(hash.slice(1))
    if (t) t.scrollIntoView()
    else G.scrollTo?.(0, push ? 0 : history.state?.y || 0)
    landed(u)
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
  document.addEventListener('click', click)
  G.addEventListener?.('popstate', pop)
  ahead.scan()
  return (active = {
    go: (u, push = true) => go(app(u), push),
    check,
    stop: () => (quit(), document.removeEventListener('click', click), G.removeEventListener?.('popstate', pop)),
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
  const meta = G.document?.querySelector?.('meta[name=brain-prefetch]')
  if (o.prefetch === false || (meta && /^(off|false|0)$/i.test(meta.getAttribute('content') || '')) || !G.document || !G.fetch) return off
  const net = G.navigator?.connection
  const thin = () => !!net && (net.saveData || /(^|-)2g$/.test(net.effectiveType || ''))
  const cache = new Map(), rank = new Map(), queue = [], pre = new Map(), running = new Map(), pending = new Map(), planned = new Map()
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
    if (!a?.getAttribute?.('href') || a.getAttribute('target') || a.hasAttribute('download')) return null
    for (let n = a; n?.getAttribute; n = n.parentNode) if (n.getAttribute('data-prefetch') === 'false') return null
    const k = key(a.href || a.getAttribute('href'))
    return safe(k) ? k : null
  }
  const warm = list => {
    if (dead) return
    let n = 0
    for (const href of list) {
      const u = key(href)
      if (!u || pre.has(u) || pre.size > 47 || !same(u) || !new URL(u).pathname.includes('/_b/') || ++n > 8) continue
      const l = document.createElement('link')
      l.setAttribute('rel', /\.(js|jsbrain)(\?|$)/.test(u) ? 'modulepreload' : 'prefetch')
      l.setAttribute('href', u)
      l.setAttribute('fetchpriority', 'low')
      document.head.appendChild(l)
      pre.set(u, l)
    }
  }
  const remove = k => { const c = cache.get(k); if (c) bytes -= c.bytes || 0; cache.delete(k) }
  const prune = () => {
    while (cache.size > MAX_KEPT || bytes > 4 * 1024 * 1024) remove(cache.keys().next().value)
  }
  const crashed = () => {
    worker?.terminate(); worker = null
    for (const p of pending.values()) { clearTimeout(p.timer); p.resolve(null) }
    pending.clear()
  }
  if (G.Worker && o.prefetchWorker) {
    try {
      const url = new URL(o.prefetchWorker, location.href)
      if (url.origin === location.origin) {
        worker = new Worker(url.href, { type: 'module' })
        worker.onmessage = e => {
          const m = e.data, p = pending.get(m?.id)
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
      const done = () => { const p = pending.get(id); if (p) { clearTimeout(p.timer); pending.delete(id); resolve(null); worker?.postMessage({ id, cancel: true }) } }
      const timer = setTimeout(done, 6500)
      pending.set(id, { resolve, timer })
      controller.signal.addEventListener('abort', done, { once: true })
      try { worker.postMessage({ id, url: k }) } catch { done(); crashed() }
    })
    return prefetchPage(k, controller.signal)
  }

  const get = k => {
    const controller = new AbortController()
    running.set(k, controller)
    const timer = setTimeout(() => controller.abort(), 6000)
    const c = { at: Date.now(), epoch, bytes: 0, t: timer }
    c.p = background(k, controller).then(m => {
      if (!m || dead || c.epoch !== epoch || controller.signal.aborted) { if (cache.get(k) === c) remove(k); return null }
      const headers = Array.isArray(m.headers) ? new Headers(m.headers) : m.headers
      const cc = headers.get('cache-control') || '', freshness = /(?:^|,)\s*(?:s-maxage|max-age)=(\d+)/i.exec(cc)
      if (m.assets) warm(m.assets)
      if (planned.has(k)) warm(planned.get(k))
      // Personalized and dynamically revalidated HTML always goes back through the server on navigation, unless that
      // navigation was already waiting for this very request (taken in flight: as fresh as a new one).
      // Speculation still warms its immutable code/assets; it never reuses another tab's prior session data.
      if (!c.taken && (/private|no-store/i.test(cc) || /no-cache/i.test(cc) && headers.get('x-brain-static') !== '1')) { if (cache.get(k) === c) remove(k); return null }
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
    if (dead || document.hidden || net?.saveData) return
    while (busy < PARALLEL && queue.length) {
      const k = queue.shift()
      if (cache.has(k)) continue
      busy++
      get(k).finally(() => { busy--; pump() })
    }
  }
  const later = f => G.requestIdleCallback ? G.requestIdleCallback(f, { timeout: 2000 }) : setTimeout(f, 200)
  // The queue is ordered by rank, lower first: intent, then main content by position, the compiler's plan, then footer
  // and aside. A click never waits here: take() or its own fetch.
  const want = (k, now, at = 1e6) => {
    if (!k || dead || cache.has(k) || count >= (now ? MAX_KEPT : MAX_PAGE) || (!now && !same(k, false))) return // only intent warms a full-load route
    const i = queue.indexOf(k)
    if (i >= 0) queue.splice(i, 1); else count++
    rank.set(k, now ? -1 : at)
    queue.push(k)
    queue.sort((a, b) => rank.get(a) - rank.get(b))
    if (now) pump()
    else if (!idle) idle = later(() => { idle = 0; pump() })
  }
  const intent = e => {
    let a = e.target
    while (a && a.tagName !== 'A') a = a.parentNode
    want(ok(a), true)
  }
  const evs = ['pointerover', 'focusin', 'touchstart']
  for (const t of evs) document.addEventListener(t, intent, { passive: true, capture: true })
  const forget = () => {
    epoch++; queue.length = 0; cache.clear(); bytes = 0; count = 0
    for (const c of running.values()) c.abort()
    running.clear()
  }
  document.addEventListener('submit', forget, { capture: true })
  G.addEventListener?.('brain:write', forget)
  if (G.IntersectionObserver) io = new IntersectionObserver(es => {
    for (const { target: a, isIntersecting } of es) {
      const k = ok(a), i = queue.indexOf(k)
      if (isIntersecting) thin() || want(k, false, (a.closest('footer,aside') ? 1e7 : 0) + a.getBoundingClientRect().top)
      else if (i >= 0) queue.splice(i, 1) // left the viewport before its turn
    }
  }, { rootMargin: '200px' })
  return {
    scan() {
      if (dead) return
      count = 0; queue.length = 0; planned.clear(); seen = new WeakSet(); io?.disconnect()
      const anchors = document.querySelectorAll('a[href]'), allowed = new Set()
      for (const a of anchors) { const k = ok(a); if (k) allowed.add(k); if (io && !seen.has(a)) { seen.add(a); io.observe(a) } }
      // Compiler-generated graph, validated against the current DOM: hidden/opted-out links cannot sneak in.
      const plan = document.getElementById('brain-next')
      if (plan && !thin()) {
        try {
          const map = JSON.parse(plan.textContent)
          if (map.v === 1) for (const item of map.next.slice(0, MAX_PAGE)) {
            const k = key(item.url)
            if (allowed.has(k) && safe(k)) { if (item.assets) planned.set(k, item.assets.slice(0, 16)); want(k, false) }
          }
        } catch {}
      }
    },
    stop() {
      if (dead) return
      forget(); dead = true; crashed(); io?.disconnect()
      for (const t of evs) document.removeEventListener(t, intent, { capture: true })
      document.removeEventListener('submit', forget, { capture: true })
      G.removeEventListener?.('brain:write', forget)
      for (const l of pre.values()) l.remove()
      pre.clear()
    },
    async take(u) {
      const k = key(u), c = k && cache.get(k)
      if (!c) return null
      remove(k)
      // The navigation reuses the request already on its way (never a second one) and drops its speculative deadline.
      clearTimeout(c.t)
      c.taken = 1
      const r = await c.p
      return r && !dead && c.epoch === epoch && Date.now() - c.at < r.ttl ? r : null
    },
  }
}

const HOLD = ['click', 'submit']
function hold(el) {
  const q = []
  const keep = e => {
    const path = []
    // a link needs no mounted view: it navigates now (a real href; a `#` one waits for its handler like a button)
    for (let n = e.target; n && n !== el; n = n.parentNode) {
      if (n.tagName === 'A' && (n.getAttribute('href') || '#')[0] != '#') return
      path.unshift([].indexOf.call(n.parentNode.childNodes, n))
    }
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
      el.$mounted?.()
      el.$mounted = null
      for (const [e, path] of q) {
        let n = el
        for (const i of path) n = n?.childNodes[i]
        n?.dispatchEvent(new e.constructor(e.type, e))
      }
    },
  })
}
