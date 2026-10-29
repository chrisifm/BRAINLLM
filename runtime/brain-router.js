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
  const u = app(String(to))
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
  let seq = 0, through = null, stale = false
  const ahead = predict(o, u => same(u))
  // Build signature (§35.7): the page's <meta name="brain-build">. A navigation response from another build, or the
  // service worker announcing one, turns client navigation into full loads so old and new code never mix.
  const build = () => {
    const m = document.querySelector('meta[name=brain-build]')
    return m ? m.getAttribute('content') : null
  }
  const sw = G.navigator && navigator.serviceWorker
  const onMsg = e => typeof e.data === 'string' && e.data.startsWith('brain:build ') && build() && e.data.slice(12) !== build() && (stale = true)
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
  // (`loading: [{ rx, view }]`, most specific first, emitted by the build with the `core` URL) is mounted in the
  // region until the page arrives; fast navigations never show it, so nothing flickers.
  const loads = o.loading || []
  function waiting(url, my) {
    const path = strip(new URL(url, location.href).pathname), l = loads.find(x => new RegExp(x.rx).test(path))
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
      for (const s of ['meta[name=description]', 'link[rel=canonical]']) {
        const a = document.head && document.head.querySelector(s), b = doc.querySelector(s)
        if (a && b) a.replaceWith(document.adoptNode ? document.adoptNode(b) : b)
      }
      // SEO (§32): og:*, twitter:*, robots and JSON-LD follow the page; the region's own tags stay where they are
      const tags = (r, skip) =>
        [...((r && r.querySelectorAll && r.querySelectorAll('meta,script')) || [])].filter(e => {
          for (let p = e.parentNode; p; p = p.parentNode) if (p === skip) return false
          const k = e.getAttribute('property') || e.getAttribute('name') || ''
          return e.tagName === 'SCRIPT' ? e.getAttribute('type') === 'application/ld+json' : /^(og:|twitter:|robots$)/.test(k)
        })
      if (document.head) {
        for (const e of tags(document.head)) e.remove()
        for (const e of tags(doc.head || doc, next)) document.head.appendChild(document.adoptNode ? document.adoptNode(e) : e)
      }
      // the next page's route params (or none): route() reads them
      const pa = document.head && document.head.querySelector('meta[name=brain-params]'), pb = doc.querySelector('meta[name=brain-params]')
      if (pa) pa.remove()
      if (pb && document.head) document.head.appendChild(document.adoptNode ? document.adoptNode(pb) : pb)
      routed()
      current()
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
  const same = u => {
    try {
      const x = new URL(u, location.href), b = base()
      // same origin and inside the app's base path (§23 #148); anything else is a full load
      return x.origin === location.origin && (x.pathname + '/').startsWith(b) && !(o.skip && o.skip(x))
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
  const pop = () => go(location.href, false)
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
// target links, log-out style paths, and everything with `router({ prefetch: false })` or
// `<meta name="brain-prefetch" content="off">`. A page served `private`/`no-store` is reused for 30 s, others for 5 min.
// Each fetch says `x-brain-prefetch: 1`, so a server can tell it apart from a click.
const FRESH = 300000, FRESH_PRIVATE = 30000, MAX_PAGE = 12, PARALLEL = 2
const NOPE = /(^|\/)(log-?out|sign-?out|logoff)(\/|$)/i
export function predict(o = {}, same = () => false) {
  const off = { scan() {}, stop() {}, take: async () => null }
  const meta = G.document && document.querySelector && document.querySelector('meta[name=brain-prefetch]')
  if (o.prefetch === false || (meta && /^(off|false|0)$/i.test(meta.getAttribute('content') || '')) || !G.document || !G.fetch) return off
  const net = G.navigator && navigator.connection
  const thin = () => !!net && (net.saveData || /(^|-)2g$/.test(net.effectiveType || ''))
  const cache = new Map(), queue = [], seen = new Set(), pre = new Set()
  let busy = 0, count = 0, io = null, idle = 0
  const key = h => {
    try {
      const u = new URL(h, location.href)
      u.hash = ''
      return u.href
    } catch {
      return null
    }
  }
  const ok = a => {
    if (!a || !a.getAttribute || !a.getAttribute('href') || a.getAttribute('target') || a.hasAttribute('download')) return null
    for (let n = a; n && n.getAttribute; n = n.parentNode) if (n.getAttribute('data-prefetch') === 'false') return null
    const k = key(a.href || a.getAttribute('href'))
    if (!k || !same(k) || NOPE.test(new URL(k).pathname) || k === key(location.href)) return null
    return k
  }
  // the next page's chunks: module scripts and static imports in its HTML
  const chunks = html => {
    if (!document.head) return
    const re = /<script[^>]+type="module"[^>]+src="([^"]+)"|\bfrom\s*"([^"]+\.js)"|\bimport\(\s*"([^"]+\.js)"\s*\)/g
    let m, n = 0
    while ((m = re.exec(html)) && n < 8) {
      const u = key(m[1] || m[2] || m[3])
      if (!u || pre.has(u) || !same(u)) continue
      pre.add(u), n++
      const l = document.createElement('link')
      l.setAttribute('rel', 'modulepreload')
      l.setAttribute('href', u)
      document.head.appendChild(l)
    }
  }
  const get = k => {
    const at = Date.now()
    const p = fetch(k, { headers: { 'x-brain-nav': '1', 'x-brain-prefetch': '1' }, priority: 'low', credentials: 'same-origin' })
      .then(async r => {
        const ct = (r.headers && r.headers.get && r.headers.get('content-type')) || 'text/html'
        if (!r.ok || !/html/.test(ct)) return null
        const text = await r.text()
        chunks(text)
        const cc = (r.headers.get && r.headers.get('cache-control')) || ''
        // a stand-in for the Response that load() reads: headers, status, redirect and the body once per use
        return { ok: r.ok, status: r.status, redirected: r.redirected, url: r.url, headers: r.headers, text: async () => text, ttl: /private|no-store/i.test(cc) ? FRESH_PRIVATE : FRESH }
      })
      .catch(() => null)
    cache.set(k, { at, p })
    return p
  }
  const pump = () => {
    while (busy < PARALLEL && queue.length) {
      const k = queue.shift()
      if (cache.has(k)) continue
      busy++
      get(k).finally(() => (busy--, pump()))
    }
  }
  const later = f => (G.requestIdleCallback ? G.requestIdleCallback(f, { timeout: 2000 }) : setTimeout(f, 200))
  const want = (k, now) => {
    if (!k || cache.has(k)) return
    const i = queue.indexOf(k)
    if (i >= 0) queue.splice(i, 1)
    if (now) queue.unshift(k), pump()
    else if (count < MAX_PAGE) count++, queue.push(k), idle || (idle = 1) && later(() => ((idle = 0), pump()))
  }
  const intent = e => {
    if (net && net.saveData) return
    let a = e.target
    while (a && a.tagName !== 'A') a = a.parentNode
    want(ok(a), true)
  }
  const evs = ['pointerover', 'focusin', 'touchstart']
  for (const t of evs) document.addEventListener(t, intent, { passive: true, capture: true })
  if (G.IntersectionObserver)
    io = new IntersectionObserver(
      es => {
        for (const e of es) if (e.isIntersecting) io.unobserve(e.target), thin() || want(ok(e.target), false)
      },
      { rootMargin: '200px' }
    )
  return {
    scan() {
      count = 0
      queue.length = 0 // the previous view's links are no longer the likely next step
      if (!io) return
      for (const a of document.querySelectorAll('a[href]')) if (!seen.has(a)) seen.add(a), io.observe(a)
    },
    stop() {
      io && io.disconnect()
      for (const t of evs) document.removeEventListener(t, intent, { capture: true })
      cache.clear()
    },
    // the prefetched page for this URL when it is still fresh (used once: a later visit fetches again)
    async take(u) {
      const k = key(u), c = k && cache.get(k)
      if (!c) return null
      cache.delete(k)
      const r = await c.p
      return r && Date.now() - c.at < r.ttl ? r : null
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
