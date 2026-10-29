// brain-router.js: Navigation API path and History fallback, region swap, title, island disposal, focus.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { install, MouseEvent } from '../shim/dom.js'

const doc = install()
const ce = doc.createElement.bind(doc)
doc.createElement = t => Object.assign(ce(t), t === 'script' ? { $fresh: true } : {})
const rt = await import('./rt.js')
const { router, redirect } = await rt.sibling('brain-router.js')
const pages = {
  '/b': '<main><h1>Page B</h1><p>b</p></main>',
  '/c': '<main><h1>Page C</h1></main>',
  '/v': '<main><div id="v"></div><script type="module">mountV()</script><script type="b/state">{}</script></main>',
  '/m': '<main><h1>Mount</h1><p><button type="button">Load</button></p><script type="module">const e=document.getElementById(\'brain-view\');e.$d=m(V,e,{})</script></main>',
  '/login': '<main><h1>Login</h1></main>',
  '/admin/x': '<main><h1>Admin</h1></main>',
}
let served = null // server-side redirect: next fetch ends on this URL
globalThis.fetch = async u => {
  const p = new URL(u, 'http://x.test').pathname
  const to = served
  served = null
  if (to) return { ok: true, status: 200, redirected: true, url: to, headers: { get: k => (k === 'content-type' ? 'text/html' : null) }, text: async () => pages[new URL(to).pathname] }
  return pages[p]
    ? { ok: true, status: 200, headers: { get: k => (k === 'content-type' ? 'text/html' : null) }, text: async () => pages[p] }
    : { ok: false, status: 404, headers: { get: k => (k === 'content-type' ? 'text/html' : null) }, text: async () => '' }
}
const parse = h => {
  const d = doc.createElement('html')
  d.innerHTML = h
  d.title = 'Title ' + (h.match(/<h1>(.*?)<\/h1>/) || [])[1]
  return d
}
const page = () => {
  doc.body.textContent = ''
  const m = doc.createElement('main')
  m.innerHTML = '<h1>Page A</h1><div><span>island</span></div>'
  doc.body.appendChild(m)
  let disposed = 0
  m.lastChild.$d = () => disposed++
  return { m, disposed: () => disposed }
}

test('Navigation API: intercepts same-origin navigations, swaps main inside a view transition, disposes islands', async () => {
  globalThis.location = new URL('http://x.test/a')
  let h, vt = 0
  globalThis.navigation = { addEventListener: (t, f) => (h = f), removeEventListener() {}, navigate() {} }
  let ready
  doc.startViewTransition = f => (vt++, f(), { ready: (ready = Promise.reject(new Error('Transition was skipped'))), updateCallbackDone: Promise.resolve() })
  const { m, disposed } = page()
  const r = router({ parse, hard: u => assert.fail('hard nav ' + u) })
  let handler
  h({ canIntercept: true, destination: { url: 'http://x.test/b' }, intercept: o => (handler = o.handler) })
  await handler()
  assert.equal(m.textContent, 'Page Bb')
  assert.equal(doc.title, 'Title Page B')
  assert.equal(disposed(), 1)
  assert.equal(vt, 1)
  await new Promise(r => setTimeout(r, 0)) // a skipped transition's ready rejection must be handled
  let ignored = true
  h({ canIntercept: true, destination: { url: 'https://other.test/x' }, intercept: () => (ignored = false) })
  h({ canIntercept: true, hashChange: true, destination: { url: 'http://x.test/a#x' }, intercept: () => (ignored = false) })
  assert.ok(ignored)
  r.stop()
  delete globalThis.navigation
  delete doc.startViewTransition
})

test('History fallback: link click → pushState, swap, focus on the new h1; 404 → full page load', async () => {
  const st = []
  globalThis.location = new URL('http://x.test/a')
  globalThis.history = {
    state: null,
    pushState: (s, _, u) => (st.push(u), (history.state = s), (globalThis.location = new URL(u, location.href))),
    replaceState: s => (history.state = s),
  }
  globalThis.scrollTo = () => {}
  const hard = []
  const { m } = page()
  const r = router({ parse, hard: u => hard.push(u) })
  const a = doc.createElement('a')
  a.setAttribute('href', '/c')
  m.appendChild(a)
  a.dispatchEvent(new MouseEvent('click', { bubbles: true, cancelable: true }))
  await new Promise(r => setTimeout(r, 5))
  assert.deepEqual(st, ['http://x.test/c'])
  assert.equal(m.textContent, 'Page C')
  assert.equal(doc.activeElement, m.firstChild)
  assert.equal(m.firstChild.getAttribute('tabindex'), '-1')
  await r.go('/missing')
  assert.deepEqual(hard, ['/missing'])
  r.stop()
})

const hist = () => {
  const st = []
  globalThis.location = new URL('http://x.test/a')
  globalThis.history = {
    state: null,
    pushState: (s, _, u) => (st.push(u), (history.state = s), (globalThis.location = new URL(u, location.href))),
    replaceState: (s, _, u) => (st.push('replace ' + u), (history.state = s), u && (globalThis.location = new URL(u, location.href))),
  }
  globalThis.scrollTo = () => {}
  return st
}

test('guards (History): outer→inner by folder, ctx, cancel, redirect thrown or returned, failure → full load', async () => {
  const st = hist(), hard = [], seen = []
  const { m } = page()
  let allow = true
  const r = router({
    parse,
    hard: u => hard.push(u),
    guards: {
      '/admin/': c => (seen.push('inner ' + c.path + ' ' + c.query.k), allow || redirect('/login')),
      '/admin': c => seen.push('outer'),
      '/c': () => false,
      '/boom': () => { throw Error('x') },
      '/str': async () => '/b',
    },
  })
  await r.go('/admin/x?k=1')
  assert.deepEqual(seen, ['outer', 'inner /admin/x 1'])
  assert.equal(m.textContent, 'Admin')
  allow = false
  await r.go('/admin/x')
  assert.equal(m.textContent, 'Login')
  assert.equal(location.pathname, '/login')
  await r.go('/c')
  assert.equal(m.textContent, 'Login') // cancelled: no fetch, no push
  await r.go('/str')
  assert.equal(m.textContent, 'Page Bb')
  await r.go('/boom')
  assert.deepEqual(hard, ['/boom'])
  assert.ok(!st.includes('http://x.test/c'))
  assert.equal(await r.check('/adminx'), true) // prefix matches whole segments only
  r.stop()
})

test('guards: back/forward blocked → the previous URL is pushed back; a server redirect updates the address bar', async () => {
  const st = hist(), on = {}
  globalThis.addEventListener = (t, f) => (on[t] = f)
  globalThis.removeEventListener = () => {}
  page()
  const r = router({ parse, hard: u => assert.fail('hard ' + u), guards: { '/c': () => false } })
  await r.go('/b')
  globalThis.location = new URL('http://x.test/c') // user pressed back/forward to /c
  on.popstate()
  await new Promise(r => setTimeout(r, 5))
  assert.equal(st.at(-1), 'http://x.test/b')
  served = 'http://x.test/login'
  await r.go('/b')
  assert.equal(st.at(-1), 'replace http://x.test/login')
  assert.equal(location.pathname, '/login')
  r.stop()
  delete globalThis.addEventListener
  delete globalThis.removeEventListener
})

test('guards (Navigation API): precommit cancels or redirects before commit; without precommit, replace after commit', async () => {
  globalThis.location = new URL('http://x.test/a')
  let h
  const navs = []
  globalThis.navigation = { currentEntry: { url: 'http://x.test/a' }, addEventListener: (t, f) => (h = f), removeEventListener() {}, navigate: (u, o) => navs.push([u, o.history]) }
  const { m } = page()
  const r = router({ parse, hard: u => assert.fail('hard ' + u), guards: { '/c': () => false, '/admin': () => redirect('/login') } })
  const fire = url => {
    let o
    h({ canIntercept: true, destination: { url }, intercept: x => (o = x) })
    return o
  }
  // precommit supported
  let o = fire('http://x.test/c')
  await assert.rejects(o.precommitHandler({ redirect() {} }), { name: 'AbortError' })
  let red
  o = fire('http://x.test/admin/x')
  await o.precommitHandler({ redirect: u => ((red = u), (navigation.currentEntry = { url: u })) })
  assert.equal(red, 'http://x.test/login')
  await o.handler()
  assert.equal(m.textContent, 'Login')
  // no precommit: handler decides after commit
  navigation.currentEntry = { url: 'http://x.test/login' }
  o = fire('http://x.test/c')
  await o.handler()
  assert.deepEqual(navs, [['http://x.test/login', 'replace']])
  r.stop()
  delete globalThis.navigation
})

test('region contract: disposes the mount stored on the region and recreates its scripts so they run; b/state untouched', async () => {
  hist()
  const { m } = page()
  let gone = 0
  m.$d = () => gone++
  const r = router({ parse, hard: u => assert.fail('hard ' + u) })
  await r.go('/v')
  assert.equal(gone, 1)
  assert.equal(m.$d, null)
  const [mod, state] = m.querySelectorAll('script')
  assert.equal(mod.textContent, 'mountV()')
  assert.equal(mod.getAttribute('type'), 'module')
  assert.ok(mod.$fresh, 'module script recreated')
  assert.ok(!state.$fresh, 'b/state left as parsed')
  r.stop()
})

// --- #131: never mix builds ---
const withBuild = sig => {
  for (const m of doc.querySelectorAll('meta[name=brain-build]')) m.remove()
  const m = doc.createElement('meta')
  m.setAttribute('name', 'brain-build')
  m.setAttribute('content', sig)
  doc.body.appendChild(m)
}
const realFetch = globalThis.fetch
const fetchBuild = sig => async u => {
  const r = await realFetch(u)
  return { ...r, headers: { get: k => (k === 'x-brain-build' ? sig : 'text/html') } }
}

test('#131 a navigation response from another build → full load; same build → client swap', async () => {
  hist()
  const { m } = page()
  withBuild('b1')
  const hard = []
  const r = router({ parse, hard: u => hard.push(u) })
  globalThis.fetch = fetchBuild('b1')
  await r.go('/b')
  assert.equal(m.textContent, 'Page Bb')
  globalThis.fetch = fetchBuild('b2')
  await r.go('/c')
  assert.deepEqual(hard, ['/c'])
  assert.equal(m.textContent, 'Page Bb') // nothing from the new build was swapped into the old page
  globalThis.fetch = realFetch
  r.stop()
})

test('#131 the service worker announcing another build makes the next navigation a full load', async () => {
  hist()
  page()
  withBuild('b1')
  let onMsg
  const nav0 = Object.getOwnPropertyDescriptor(globalThis, 'navigator')
  Object.defineProperty(globalThis, 'navigator', { configurable: true, value: { serviceWorker: { addEventListener: (t, f) => (onMsg = f), removeEventListener() {} } } })
  const hard = []
  const r = router({ parse, hard: u => hard.push(u) })
  onMsg({ data: 'brain:build b1' }) // same build: ignored
  await r.go('/b')
  assert.deepEqual(hard, [])
  onMsg({ data: 'brain:build b9' })
  await r.go('/c')
  assert.deepEqual(hard, ['/c'])
  r.stop()
  if (nav0) Object.defineProperty(globalThis, 'navigator', nav0)
})

test('#131 Navigation API: the full load the router asks for is not intercepted again (no loop)', async () => {
  globalThis.location = new URL('http://x.test/a')
  let h
  globalThis.navigation = { currentEntry: { url: 'http://x.test/a' }, addEventListener: (t, f) => (h = f), removeEventListener() {}, navigate() {} }
  page()
  const hard = []
  const r = router({ parse, hard: u => hard.push(u) })
  let o
  h({ canIntercept: true, destination: { url: 'http://x.test/missing' }, intercept: x => (o = x) })
  await o.handler() // 404 → full load
  assert.deepEqual(hard, ['http://x.test/missing'])
  let intercepted = false
  h({ canIntercept: true, destination: { url: 'http://x.test/missing' }, intercept: () => (intercepted = true) })
  assert.equal(intercepted, false) // the browser loads it for real
  r.stop()
  delete globalThis.navigation
})

test('QA #124: a reload navigation is never intercepted (the chunk-missing reload must be a real full load)', () => {
  globalThis.location = new URL('http://x.test/a')
  let h
  globalThis.navigation = { currentEntry: { url: 'http://x.test/a' }, addEventListener: (t, f) => (h = f), removeEventListener() {}, navigate() {} }
  const r = router({ parse, hard: u => assert.fail('hard ' + u) })
  let intercepted = false
  h({ canIntercept: true, navigationType: 'reload', destination: { url: 'http://x.test/a' }, intercept: () => (intercepted = true) })
  assert.equal(intercepted, false)
  h({ canIntercept: true, navigationType: 'push', destination: { url: 'http://x.test/b' }, intercept: () => (intercepted = true) })
  assert.equal(intercepted, true)
  r.stop()
  delete globalThis.navigation
})

test('BUILD #177: a recreated region script carries the current page nonce, not the one it came with', async () => {
  hist()
  const { m } = page()
  const own = doc.createElement('script')
  own.setAttribute('nonce', 'PAGE-NONCE')
  doc.body.appendChild(own)
  const r = router({ parse, hard: u => assert.fail('hard ' + u) })
  pages['/n'] = '<main><script type="module" nonce="OTHER-NONCE">x()</script></main>'
  await r.go('/n')
  const sc = m.querySelectorAll('script')[0]
  assert.equal(sc.getAttribute('nonce'), 'PAGE-NONCE')
  own.remove()
  r.stop()
})

test('#150 a page in another language (<html lang>) is a full load, never a client swap', async () => {
  hist()
  const { m } = page()
  doc.documentElement.setAttribute('lang', 'en')
  const hard = []
  const r = router({ parse: h => { const d = new DOMParser0(h); return d }, hard: u => hard.push(u) })
  pages['/nl/x'] = '<html lang="nl"><body><main><h1>Hallo</h1></main></body></html>'
  pages['/y'] = '<html lang="en"><body><main><h1>Y</h1></main></body></html>'
  await r.go('/y')
  assert.equal(m.textContent, 'Y')
  await r.go('/nl/x')
  assert.deepEqual(hard, ['/nl/x'])
  assert.equal(m.textContent, 'Y')
  r.stop()
  doc.documentElement.removeAttribute('lang')
})
// a parsed document with documentElement (what DOMParser gives in a browser)
function DOMParser0(h) {
  const html = doc.createElement('html')
  const lang = /<html lang="([^"]+)"/.exec(h)
  if (lang) html.setAttribute('lang', lang[1])
  html.innerHTML = h.replace(/<\/?(html|body)[^>]*>/g, '')
  return { documentElement: html, title: '', querySelector: s => html.querySelector(s) }
}

test('#65 client guards share the server shapes: {redirect} object and deny → full load (the server answers 403)', async () => {
  const st = hist(), hard = []
  page()
  const { deny: dn } = await rt.sibling('brain-router.js')
  const r = router({ parse, hard: u => hard.push(u), guards: { '/admin': () => dn('Admins only'), '/old': () => ({ redirect: '/b' }) } })
  await r.go('/admin/x')
  assert.deepEqual(hard, ['/admin/x'])
  await r.go('/old')
  assert.equal(location.pathname, '/b')
  r.stop()
})

test('F12b: a guard that reads ctx.user leaves the decision to the server (full load) instead of redirecting a signed-in user', async () => {
  const st = hist(), hard = []
  page()
  const r = router({
    parse,
    hard: u => hard.push(u),
    guards: {
      '/account': c => (c.user ? true : redirect('/login')), // the SPEC's guard, thrown redirect
      '/me': c => (c.user ? true : { redirect: '/login' }), // returned
      '/pass': c => (c.user, true), // reads it but lets the navigation go on: stays a client navigation
    },
  })
  await r.go('/account/settings')
  await r.go('/me')
  assert.deepEqual(hard, ['/account/settings', '/me']) // the server guard sees the session cookie and decides
  assert.notEqual(location.pathname, '/login')
  assert.equal(await r.check('/pass'), true)
  r.stop()
  void st
})

test('#216 base path: guards see app paths, go()/redirect get the base, paths outside the base are not client navigations', async () => {
  const st = hist(), hard = [], seen = []
  globalThis.location = new URL('http://x.test/app/')
  const { m } = page()
  const meta = doc.createElement('meta')
  meta.setAttribute('name', 'brain-base')
  meta.setAttribute('content', '/app/')
  doc.body.appendChild(meta)
  pages['/app/b'] = '<main><h1>App B</h1></main>'
  pages['/app/login'] = '<main><h1>App login</h1></main>'
  const r = router({ parse, hard: u => hard.push(u), guards: { '/admin': c => (seen.push(c.path), { redirect: '/login' }) } })
  await r.go('/b') // an app path: gets the base
  assert.equal(location.pathname, '/app/b')
  assert.equal(m.textContent, 'App B')
  await r.go('/app/admin/x') // already with the base: guard matched on the app path, redirect gets the base
  assert.deepEqual(seen, ['/admin/x'])
  assert.equal(location.pathname, '/app/login')
  assert.equal(await r.check('/elsewhere/admin'), true) // not under /admin once the base is stripped? it is not in the app at all
  const a = doc.createElement('a')
  a.setAttribute('href', '/other-app/page')
  m.appendChild(a)
  const ev = new MouseEvent('click', { bubbles: true, cancelable: true })
  a.dispatchEvent(ev)
  assert.equal(ev.defaultPrevented, false) // outside the base: the browser loads it
  meta.remove()
  r.stop()
})

test('#169 navigate()/route(): app paths through the router (base added, push or replace), route() signals follow it', async () => {
  const st = hist(), hard = []
  globalThis.location = new URL('http://x.test/app/')
  const { m } = page()
  const meta = (name, content) => {
    const e = doc.createElement('meta')
    e.setAttribute('name', name)
    e.setAttribute('content', content)
    return doc.head.appendChild(e)
  }
  const mb = meta('brain-base', '/app/'), mp = meta('brain-params', '{"id":"1"}')
  pages['/app/posts/2'] = `<meta name="brain-params" content='{"id":"2"}'><main><h1>Post 2</h1></main>`
  pages['/app/b'] = '<main><h1>App B</h1></main>'
  const { navigate } = await rt.sibling('brain-router.js')
  const { route } = await rt.sibling('brain-env.js')
  const r = router({ parse, hard: u => hard.push(u) })
  const R = route()
  const seen = []
  const stop = rt.fx(() => seen.push(`${R.path} ${R.params.id} ${R.query.q}`))
  assert.deepEqual(seen, ['/ 1 undefined'])
  await navigate('/posts/2?q=x')
  assert.equal(st.at(-1), '/app/posts/2?q=x')
  assert.equal(m.textContent, 'Post 2')
  await new Promise(r => setTimeout(r, 0))
  assert.deepEqual(seen.at(-1), '/posts/2 2 x')
  assert.equal(R.params.id, '2')
  await navigate('/b', { replace: true })
  assert.equal(st.at(-1), 'replace /app/b')
  assert.equal(location.pathname, '/app/b')
  assert.deepEqual(R.params, {}) // the next page has no params
  assert.equal(R.path, '/b')
  assert.deepEqual(hard, [])
  r.stop()
  stop()
  // no router running: a normal page load, still with the base
  const loads = []
  globalThis.location = Object.assign(new URL('http://x.test/app/b'), { assign: u => loads.push(u), replace: u => loads.push('replace ' + u) })
  navigate('/account')
  navigate('/login', { replace: true })
  assert.deepEqual(loads, ['/app/account', 'replace /app/login'])
  mb.remove()
  for (const e of doc.head.querySelectorAll('meta[name=brain-params]')) e.remove()
  mp.remove()
  // no base: paths stay as given, through the router
  const st2 = hist()
  page()
  const r2 = router({ parse, hard: u => hard.push(u) })
  await navigate('/c')
  assert.equal(st2.at(-1), '/c')
  assert.equal(route().path, '/c')
  assert.deepEqual(hard, [])
  r2.stop()
})

test('#169 route() while the server renders: from __brainRoute', async () => {
  const { route } = await rt.sibling('brain-env.js')
  globalThis.__brainRoute = { path: '/posts/9', params: { id: '9' }, query: { q: 'y' } }
  assert.deepEqual({ ...route() }, { path: '/posts/9', params: { id: '9' }, query: { q: 'y' } })
  delete globalThis.__brainRoute
})

test('#169 aria-current="page" on links to the current page, on start and after each navigation (layout links too)', async () => {
  const st = hist()
  globalThis.location = new URL('http://x.test/a')
  page()
  const nav = doc.createElement('nav')
  nav.innerHTML = '<a href="/a">A</a><a href="/b/">B</a><a href="http://other.test/a">ext</a>'
  doc.body.insertBefore(nav, doc.body.firstChild)
  const [a, b, ext] = nav.querySelectorAll('a')
  const r = router({ parse, hard: () => assert.fail('hard') })
  assert.equal(a.getAttribute('aria-current'), 'page')
  assert.equal(b.getAttribute('aria-current'), null)
  await r.go('/b')
  assert.equal(st.at(-1), '/b')
  assert.equal(a.getAttribute('aria-current'), null)
  assert.equal(b.getAttribute('aria-current'), 'page') // trailing slash ignored
  assert.equal(ext.getAttribute('aria-current'), null) // another origin never
  r.stop()
  nav.remove()
})

test('#169 a form with unsaved changes asks before a client navigation (History: stays; back: URL restored)', async () => {
  const st = hist()
  page()
  const f = { dirty: true }
  globalThis.__brainDirty = new Set([f])
  const asked = []
  let answer = false
  globalThis.confirm = m => (asked.push(m), answer)
  const r = router({ parse, hard: () => assert.fail('hard') })
  await r.go('/b')
  assert.equal(location.pathname, '/a') // stayed
  assert.match(asked[0], /Leave this page\?/)
  answer = true
  await r.go('/b')
  assert.equal(location.pathname, '/b')
  f.dirty = false
  await r.go('/c')
  assert.equal(asked.length, 2) // clean: no question
  delete globalThis.confirm
  globalThis.__brainDirty = new Set()
  r.stop()
})

test('#106 SEO tags follow the page on client navigation: og:*, twitter:*, robots and JSON-LD replaced, others kept', async () => {
  const st = hist()
  page()
  doc.head.innerHTML = '<meta charset="utf-8"><meta property="og:title" content="A"><meta name="twitter:card" content="summary"><script type="application/ld+json">{"@type":"WebPage","name":"A"}</script><meta name="viewport" content="x">'
  pages['/seo'] = '<meta property="og:title" content="SEO page"><meta property="og:image" content="/seo.png"><meta name="robots" content="noindex"><script type="application/ld+json">{"@type":"Article"}</script><main><h1>SEO</h1><script type="application/ld+json">{"@type":"InRegion"}</script></main>'
  const r = router({ parse, hard: () => assert.fail('hard') })
  await r.go('/seo')
  const metas = [...doc.head.querySelectorAll('meta')].map(m => (m.getAttribute('property') || m.getAttribute('name') || 'charset') + '=' + (m.getAttribute('content') || ''))
  assert.deepEqual(metas.sort(), ['charset=', 'og:image=/seo.png', 'og:title=SEO page', 'robots=noindex', 'viewport=x'])
  const ld = [...doc.head.querySelectorAll('script')].map(s => s.textContent)
  assert.deepEqual(ld, ['{"@type":"Article"}']) // the region's JSON-LD stays in the region
  assert.ok(doc.querySelector('main').textContent.includes('InRegion'))
  r.stop()
  doc.head.innerHTML = ''
})

test('#235 _loading: mounted in the region when the next page takes > 150 ms, replaced when it arrives; fast pages never show it', async () => {
  const { writeFileSync, mkdtempSync } = await import('node:fs')
  const { join } = await import('node:path')
  const { tmpdir } = await import('node:os')
  const d = mkdtempSync(join(tmpdir(), 'brain-loading-'))
  writeFileSync(join(d, 'core.mjs'), `export const m = (C, el) => (el.appendChild(C()), () => (globalThis.__loadingGone = true))`)
  writeFileSync(join(d, 'loading.mjs'), `export default () => Object.assign(globalThis.document.createElement('p'), { className: 'spin', textContent: 'Loading…' })`)
  const st = hist()
  const { m } = page()
  const f0 = globalThis.fetch
  let delay = 400
  globalThis.fetch = async (u, i) => (await new Promise(r => setTimeout(r, delay)), f0(u, i))
  const r = router({ parse, hard: () => assert.fail('hard'), core: 'file://' + join(d, 'core.mjs'), loading: [{ rx: '^/(b|c)/?$', view: 'file://' + join(d, 'loading.mjs') }] })
  const nav = r.go('/b')
  await new Promise(res => setTimeout(res, 300))
  assert.equal(m.textContent, 'Loading…') // slow: the loading view is up
  await nav
  assert.equal(m.textContent, 'Page Bb')
  assert.equal(globalThis.__loadingGone, true) // disposed by the swap
  delay = 20
  globalThis.__loadingGone = false
  let seen = false
  const watch = setInterval(() => (seen ||= m.textContent === 'Loading…'), 5)
  await r.go('/c')
  clearInterval(watch)
  assert.equal(seen, false) // fast: never shown
  assert.equal(m.textContent, 'Page C')
  globalThis.fetch = f0
  r.stop()
})

test('BUILD #220: a guard given as the URL of its chunk is imported on first use and runs like a function guard', async () => {
  const { writeFileSync, mkdtempSync } = await import('node:fs')
  const { join } = await import('node:path')
  const { tmpdir } = await import('node:os')
  const d = mkdtempSync(join(tmpdir(), 'brain-guard-chunk-'))
  writeFileSync(join(d, 'guard.mjs'), `globalThis.__guardLoads = (globalThis.__guardLoads || 0) + 1\nexport default c => (c.query.ok ? true : { redirect: '/login' })\n`)
  const st = hist()
  page()
  const r = router({ parse, hard: () => assert.fail('hard'), guards: { '/admin': 'file://' + join(d, 'guard.mjs') } })
  assert.equal(globalThis.__guardLoads, undefined) // nothing loaded before a navigation needs it
  await r.go('/admin/x')
  assert.equal(location.pathname, '/login')
  await r.go('/admin/x?ok=1')
  assert.equal(location.pathname, '/admin/x')
  assert.equal(globalThis.__guardLoads, 1) // imported once
  r.stop()
})

test('QA #291: a click on the server HTML before the view mounts is held and replayed on the mounted view, once', async () => {
  hist()
  const { m } = page()
  const r = router({ parse, hard: u => assert.fail('hard ' + u) })
  await r.go('/m')
  const seen = []
  const out = e => seen.push('document:' + e.type)
  doc.addEventListener('click', out)
  m.querySelector('button').dispatchEvent(new MouseEvent('click', { bubbles: true, cancelable: true }))
  assert.deepEqual(seen, [], 'held while the view is not mounted')
  assert.equal(m.$d, undefined) // `e.$d && e.$d()` in the mount script finds nothing to dispose
  // what the mount script does: replace the server HTML with the live view, then store its dispose
  m.innerHTML = '<h1>Mount</h1><p><button type="button">Load</button></p>'
  m.querySelector('button').addEventListener('click', () => seen.push('live button'))
  const d = () => {}
  m.$d = d
  assert.deepEqual(seen, ['live button', 'document:click'])
  assert.equal(m.$d, d)
  m.querySelector('button').dispatchEvent(new MouseEvent('click', { bubbles: true, cancelable: true }))
  assert.equal(seen.filter(x => x === 'live button').length, 2, 'not held any more')
  doc.removeEventListener('click', out)
  r.stop()
})

test('#299: a form being submitted does not ask: its submit() may navigate on success before it returns', async () => {
  hist()
  page()
  const f = { dirty: true, busy: true } // submit() in flight: the server accepted, the app navigates to the new item
  globalThis.__brainDirty = new Set([f])
  const asked = []
  globalThis.confirm = m => (asked.push(m), false)
  const r = router({ parse, hard: () => assert.fail('hard') })
  await r.go('/b')
  assert.equal(location.pathname, '/b')
  assert.deepEqual(asked, [])
  f.busy = false // submit failed: still unsaved, the question is back
  await r.go('/c')
  assert.equal(location.pathname, '/b')
  assert.equal(asked.length, 1)
  delete globalThis.confirm
  globalThis.__brainDirty = new Set()
  r.stop()
})

test('predictive prefetch: visible links load while idle, intent jumps the queue, take() hands the page to the next navigation once', async () => {
  globalThis.location = new URL('http://x.test/a')
  const { predict } = await rt.sibling('brain-router.js')
  const io = []
  globalThis.IntersectionObserver = class {
    constructor(f) {
      this.f = f
      io.push(this)
    }
    observe(el) {
      ;(this.els ||= []).push(el)
    }
    unobserve() {}
    disconnect() {}
  }
  globalThis.requestIdleCallback = f => setTimeout(f, 0)
  const asked = []
  const real = globalThis.fetch
  globalThis.fetch = async (u, o) => (asked.push([new URL(u).pathname, o && o.headers && o.headers['x-brain-prefetch']]), real(u, o))
  doc.body.textContent = ''
  doc.body.innerHTML =
    '<main><a href="/b">B</a><a href="/c">C</a><a href="/logout">Out</a><div data-prefetch="false"><a href="/m">M</a></div>' +
    '<a href="https://other.test/x">X</a><a href="/a">Here</a><a href="/v" download>V</a></main>'
  const p = predict({}, u => new URL(u).origin === 'http://x.test')
  p.scan()
  const links = io[0].els
  io[0].f(links.map(target => ({ target, isIntersecting: true })))
  await new Promise(r => setTimeout(r, 10))
  assert.deepEqual(asked.map(a => a[0]).sort(), ['/b', '/c'])
  assert.ok(asked.every(a => a[1] === '1'))
  const r = await p.take('http://x.test/b#top')
  assert.equal(await r.text(), pages['/b'])
  assert.equal(await p.take('http://x.test/b'), null) // used once
  // intent: hovering a link that was never visible fetches it at once
  doc.body.innerHTML = '<main><a href="/login"><span>Sign in</span></a></main>'
  doc.body.querySelector('span').dispatchEvent(Object.assign(new MouseEvent('pointerover', { bubbles: true })))
  await new Promise(r => setTimeout(r, 10))
  assert.ok(asked.some(a => a[0] === '/login'))
  p.stop()
  globalThis.fetch = real
  delete globalThis.IntersectionObserver
  delete globalThis.requestIdleCallback
})

test('predictive prefetch: off by option, by meta, and stale private pages are not reused', async () => {
  globalThis.location = new URL('http://x.test/a')
  const { predict } = await rt.sibling('brain-router.js')
  assert.equal(await predict({ prefetch: false }, () => true).take('http://x.test/b'), null)
  const m = doc.createElement('meta')
  m.setAttribute('name', 'brain-prefetch')
  m.setAttribute('content', 'off')
  doc.head.appendChild(m)
  let hits = 0
  const was = globalThis.fetch
  globalThis.fetch = async () => (hits++, null)
  doc.body.innerHTML = '<main><a href="/b">B</a></main>'
  const q = predict({}, () => true)
  doc.body.querySelector('a').dispatchEvent(new MouseEvent('pointerover', { bubbles: true }))
  await new Promise(r => setTimeout(r, 5))
  assert.equal(hits, 0)
  q.stop()
  m.remove()
  globalThis.fetch = was
  const real = globalThis.fetch
  globalThis.fetch = async () => ({ ok: true, status: 200, headers: { get: k => (k === 'content-type' ? 'text/html' : k === 'cache-control' ? 'private, no-store' : null) }, text: async () => '<main></main>' })
  doc.body.innerHTML = '<main><a href="/b">B</a></main>'
  const p = predict({}, () => true)
  doc.body.querySelector('a').dispatchEvent(new MouseEvent('pointerover', { bubbles: true }))
  await new Promise(r => setTimeout(r, 5))
  const now = Date.now
  Date.now = () => now() + 31000
  assert.equal(await p.take('http://x.test/b'), null)
  Date.now = now
  p.stop()
  globalThis.fetch = real
  delete globalThis.addEventListener, delete globalThis.removeEventListener, delete globalThis.dispatchEvent
})

test('predictive prefetch: action, file, api and token links are skipped; a write forgets every fetched page', async () => {
  globalThis.location = new URL('http://x.test/a')
  const { predict } = await rt.sibling('brain-router.js')
  const asked = []
  const real = globalThis.fetch
  globalThis.fetch = async (u, o) => (asked.push(new URL(u).pathname + new URL(u).search), real(u, o))
  // the window: brain-data and brain-rpc announce writes on it (a minimal emitter: the DOM shim owns `Event`)
  const on = {}
  Object.assign(globalThis, {
    addEventListener: (t, f) => (on[t] ||= new Set()).add(f),
    removeEventListener: (t, f) => on[t] && on[t].delete(f),
    dispatchEvent: e => (on[e.type] || []).forEach(f => f(e)),
  })
  doc.body.innerHTML =
    '<main><a href="/unsubscribe">U</a><a href="/report.pdf">P</a><a href="/api/items">A</a><a href="/auth/google">G</a>' +
    '<a href="/logout.html">L</a><a href="/b?token=abc">T</a><a href="/docs/intro.html">D</a><a href="/c?page=2">C</a></main>'
  const p = predict({}, u => new URL(u).origin === 'http://x.test')
  for (const a of doc.body.querySelectorAll('a')) a.dispatchEvent(new MouseEvent('pointerover', { bubbles: true }))
  await new Promise(r => setTimeout(r, 10))
  assert.deepEqual(asked.sort(), ['/c?page=2', '/docs/intro.html'])
  globalThis.dispatchEvent(new Event('brain:write'))
  assert.equal(await p.take('http://x.test/c?page=2'), null) // forgotten: it may show data from before the write
  doc.body.innerHTML = '<main><a href="/b">B</a><form></form></main>'
  doc.body.querySelector('a').dispatchEvent(new MouseEvent('pointerover', { bubbles: true }))
  await new Promise(r => setTimeout(r, 10))
  doc.body.querySelector('form').dispatchEvent(new Event('submit', { bubbles: true }))
  assert.equal(await p.take('http://x.test/b'), null)
  p.stop()
  globalThis.fetch = real
  delete globalThis.addEventListener, delete globalThis.removeEventListener, delete globalThis.dispatchEvent
})
