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

test('absolute rel=external navigation stays native on both router paths', () => {
  for (const native of [false, true]) {
    globalThis.location = new URL('http://x.test/a')
    let onNavigate
    globalThis.navigation = native ? { addEventListener: (_, handler) => { onNavigate = handler }, removeEventListener() {} } : undefined
    page()
    const anchor = doc.createElement('a')
    anchor.setAttribute('href', 'http://x.test/live-only/')
    anchor.setAttribute('rel', 'noopener ExTeRnAl')
    doc.body.appendChild(anchor)
    const r = router({ parse, hard: () => assert.fail('the browser owns this navigation') })
    if (native) onNavigate({ sourceElement: anchor, canIntercept: true, destination: { url: 'http://x.test/live-only/' }, intercept: () => assert.fail('external link intercepted') })
    else {
      const event = new MouseEvent('click', { bubbles: true, cancelable: true, button: 0 })
      anchor.dispatchEvent(event)
      assert.equal(event.defaultPrevented, false)
    }
    r.stop()
  }
  delete globalThis.navigation
})

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
  const fire = (url, cancelable = true) => {
    let o
    h({ canIntercept: true, cancelable, destination: { url }, intercept: x => (o = x) })
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
  // back/forward traverse is not cancelable: intercept() throws with a precommitHandler, so none is passed
  o = fire('http://x.test/c', false)
  assert.equal(o.precommitHandler, undefined)
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

test('a navigation restarts instance ids: the view mounted after the swap keeps its authored ids, not ~2', async () => {
  // The compiled prelude of a component with ids (brain-codegen, #268): the first instance of a page keeps `id`, later
  // ones get `id~2`. A layout header mounted again on every client navigation must not count the page it replaced.
  let $e, $n = 0
  const suffix = () => { if ($e !== globalThis.__brainIds) $e = globalThis.__brainIds, $n = 0; return $n++ ? '~' + $n : '' }
  hist()
  page()
  assert.equal(suffix(), '', 'first page: the header keeps its ids')
  const r = router({ parse, hard: u => assert.fail('hard ' + u) })
  await r.go('/v')
  assert.equal(suffix(), '', 'after the swap the new header is the only live instance')
  assert.equal(suffix(), '~2', 'a second instance on the same page still gets ~2')
  await r.go('/b')
  assert.equal(suffix(), '')
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

test('a client guard that redirects off the site (//host, /\\host, /<tab>/host) leaves the decision to the server (full load)', async () => {
  for (const to of ['//evil.test/x', '/\\evil.test/x', '/\t/evil.test/x', 'https://evil.test/x']) {
    const st = hist(), hard = []
    page()
    const r = router({ parse, hard: u => hard.push(u), guards: { '/next': () => ({ redirect: to }) } })
    await r.go('/next')
    assert.deepEqual(hard.map(u => new URL(u, location.href).pathname), ['/next'], to)
    r.stop()
  }
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
  assert.deepEqual({ ...route() }, { path: '/posts/9', params: { id: '9' }, query: { q: 'y' }, navigating: false })
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

test('S04 route-owned head links follow the page: canonical, hreflang/Markdown alternates, prev/next replaced; site links kept', async () => {
  const st = hist()
  page()
  doc.head.innerHTML = '<link rel="icon" href="/favicon.svg"><link rel="stylesheet" href="/_b/app.css"><link rel="alternate" type="application/rss+xml" href="/rss.xml"><meta name="description" content="A"><link rel="canonical" href="https://x.test/a"><link rel="alternate" hreflang="en" href="https://x.test/a"><link rel="alternate" hreflang="es" href="https://x.test/es/a"><link rel="alternate" type="text/markdown" href="/a.md">'
  pages['/p2'] = '<meta name="description" content="P"><link rel="canonical" href="https://x.test/p2"><link rel="alternate" hreflang="en" href="https://x.test/p2"><link rel="prev" href="/p1"><main><h1>P2</h1></main>'
  pages['/bare'] = '<main><h1>Bare</h1></main>'
  const r = router({ parse, hard: () => assert.fail('hard') })
  const links = () => [...doc.head.querySelectorAll('link')].map(l => [l.getAttribute('rel'), l.getAttribute('hreflang') || l.getAttribute('type') || '', l.getAttribute('href')].join(' ')).sort()
  await r.go('/p2')
  assert.deepEqual(links(), ['alternate application/rss+xml /rss.xml', 'alternate en https://x.test/p2', 'canonical  https://x.test/p2', 'icon  /favicon.svg', 'prev  /p1', 'stylesheet  /_b/app.css'])
  assert.equal(doc.head.querySelector('meta[name=description]').getAttribute('content'), 'P')
  await r.go('/bare') // a page without its own: nothing of the previous page survives
  assert.deepEqual(links(), ['alternate application/rss+xml /rss.xml', 'icon  /favicon.svg', 'stylesheet  /_b/app.css'])
  assert.equal(doc.head.querySelector('meta[name=description]'), null)
  r.stop()
  doc.head.innerHTML = ''
})

test('#235 _loading: mounted in the region when the next page takes > 400 ms, replaced when it arrives; fast pages never show it', async () => {
  const { writeFileSync, mkdtempSync } = await import('node:fs')
  const { join } = await import('node:path')
  const { tmpdir } = await import('node:os')
  const d = mkdtempSync(join(tmpdir(), 'brain-loading-'))
  writeFileSync(join(d, 'core.mjs'), `export const m = (C, el) => (el.appendChild(C()), () => (globalThis.__loadingGone = true))`)
  writeFileSync(join(d, 'loading.mjs'), `export default () => Object.assign(globalThis.document.createElement('p'), { className: 'spin', textContent: 'Loading…' })`)
  const st = hist()
  const { m } = page()
  const f0 = globalThis.fetch
  let delay = 900
  globalThis.fetch = async (u, i) => (await new Promise(r => setTimeout(r, delay)), f0(u, i))
  const r = router({ parse, hard: () => assert.fail('hard'), core: 'file://' + join(d, 'core.mjs'), loading: [{ rx: '^/(b|c)/?$', view: 'file://' + join(d, 'loading.mjs') }] })
  const nav = r.go('/b')
  await new Promise(res => setTimeout(res, 700))
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
    '<a href="https://other.test/x">X</a><a href="/a">Here</a><a href="/v" download>V</a><a href="http://x.test/live-only/" rel="noopener ExTeRnAl">Live</a></main>'
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
  doc.body.querySelector('span').dispatchEvent(Object.assign(new MouseEvent('focusin', { bubbles: true })))
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
  doc.body.querySelector('a').dispatchEvent(new MouseEvent('focusin', { bubbles: true }))
  await new Promise(r => setTimeout(r, 5))
  assert.equal(hits, 0)
  q.stop()
  m.remove()
  globalThis.fetch = was
  const real = globalThis.fetch
  globalThis.fetch = async () => ({ ok: true, status: 200, headers: { get: k => (k === 'content-type' ? 'text/html' : k === 'cache-control' ? 'private, no-store' : null) }, text: async () => '<main></main>' })
  doc.body.innerHTML = '<main><a href="/b">B</a></main>'
  const p = predict({}, () => true)
  doc.body.querySelector('a').dispatchEvent(new MouseEvent('focusin', { bubbles: true }))
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
  for (const a of doc.body.querySelectorAll('a')) a.dispatchEvent(new MouseEvent('focusin', { bubbles: true }))
  await new Promise(r => setTimeout(r, 10))
  assert.deepEqual(asked.sort(), ['/c?page=2', '/docs/intro.html'])
  globalThis.dispatchEvent(new Event('brain:write'))
  assert.equal(await p.take('http://x.test/c?page=2'), null) // forgotten: it may show data from before the write
  doc.body.innerHTML = '<main><a href="/b">B</a><form></form></main>'
  doc.body.querySelector('a').dispatchEvent(new MouseEvent('focusin', { bubbles: true }))
  await new Promise(r => setTimeout(r, 10))
  doc.body.querySelector('form').dispatchEvent(new Event('submit', { bubbles: true }))
  assert.equal(await p.take('http://x.test/b'), null)
  p.stop()
  globalThis.fetch = real
  delete globalThis.addEventListener, delete globalThis.removeEventListener, delete globalThis.dispatchEvent
})

test('prefetch graph works without IntersectionObserver and never bypasses link opt-outs', async () => {
  globalThis.location = new URL('http://x.test/a')
  const { predict } = await rt.sibling('brain-router.js')
  const real = globalThis.fetch, asked = []
  globalThis.fetch = async (u, o) => (asked.push(new URL(u).pathname), real(u, o))
  globalThis.requestIdleCallback = f => setTimeout(f, 0)
  doc.body.innerHTML = '<a href="/b">B</a><div data-prefetch="false"><a href="/c">C</a></div><script type="application/json" id="brain-next">{"v":1,"next":[{"url":"/b"},{"url":"/c"},{"url":"/logout"}]}</script>'
  const p = predict({}, () => true)
  p.scan()
  await new Promise(r => setTimeout(r, 10))
  assert.deepEqual(asked, ['/b'])
  assert.equal(await (await p.take('http://x.test/b')).text(), pages['/b'])
  p.stop()
  globalThis.fetch = real
  delete globalThis.requestIdleCallback
})

test('prefetch write epoch rejects an in-flight response already awaited by navigation', async () => {
  globalThis.location = new URL('http://x.test/a')
  const { predict } = await rt.sibling('brain-router.js')
  const on = {}
  Object.assign(globalThis, {
    addEventListener: (t, f) => (on[t] ||= new Set()).add(f),
    removeEventListener: (t, f) => on[t]?.delete(f),
    dispatchEvent: e => (on[e.type] || []).forEach(f => f(e)),
  })
  const real = globalThis.fetch
  let finish, aborted = false
  globalThis.fetch = async (_, o) => {
    o.signal.addEventListener('abort', () => { aborted = true })
    await new Promise(resolve => { finish = resolve })
    return { ok: true, status: 200, headers: new Headers({ 'content-type': 'text/html', 'cache-control': 'private' }), text: async () => '<main>Previous user</main>' }
  }
  doc.body.innerHTML = '<a href="/b">B</a>'
  const p = predict({}, () => true)
  doc.body.querySelector('a').dispatchEvent(new MouseEvent('focusin', { bubbles: true }))
  const taking = p.take('http://x.test/b')
  globalThis.dispatchEvent(new Event('brain:write'))
  finish()
  assert.equal(await taking, null)
  assert.equal(aborted, true)
  p.stop()
  globalThis.fetch = real
  delete globalThis.addEventListener; delete globalThis.removeEventListener; delete globalThis.dispatchEvent
})

test('prefetch uses the dedicated worker bridge and terminates it with the router', async () => {
  globalThis.location = new URL('http://x.test/a')
  const { predict } = await rt.sibling('brain-router.js')
  const sent = [], instances = []
  globalThis.Worker = class {
    constructor(url) { this.url = url; instances.push(this) }
    postMessage(m) {
      sent.push(m)
      if (!m.cancel) queueMicrotask(() => this.onmessage({ data: { id: m.id, ok: true, status: 200, url: m.url, headers: [['content-type','text/html']], text: '<main>Worker page</main>', bytes: 24, assets: [] } }))
    }
    terminate() { this.terminated = true }
  }
  doc.body.innerHTML = '<a href="/b">B</a>'
  const p = predict({ prefetchWorker: '/_b/worker.js' }, () => true)
  doc.body.querySelector('a').dispatchEvent(new MouseEvent('focusin', { bubbles: true }))
  await new Promise(r => setTimeout(r, 5))
  assert.equal(instances[0].url, 'http://x.test/_b/worker.js')
  assert.equal(sent[0].url, 'http://x.test/b')
  assert.equal(await (await p.take('http://x.test/b')).text(), '<main>Worker page</main>')
  p.stop()
  assert.equal(instances[0].terminated, true)
  delete globalThis.Worker
})

test('prefetch keeps static build HTML but revalidates private/dynamic pages and honors public max-age', async () => {
  globalThis.location = new URL('http://x.test/a')
  const { predict } = await rt.sibling('brain-router.js')
  const real = globalThis.fetch
  try {
    for (const control of ['private, no-store', 'no-cache', 'no-store']) {
      globalThis.fetch = async () => ({ ok:true, status:200, headers:new Headers({'content-type':'text/html','cache-control':control}), text:async()=>'<main>Dynamic</main>' })
      doc.body.innerHTML = '<a href="/b">B</a>'
      const p=predict({},()=>true)
      doc.body.querySelector('a').dispatchEvent(new MouseEvent('focusin',{bubbles:true}))
      await new Promise(r=>setTimeout(r,5))
      assert.equal(await p.take('http://x.test/b'),null,control)
      p.stop()
    }
    globalThis.fetch = async () => ({ok:true,status:200,headers:new Headers({'content-type':'text/html','cache-control':'no-cache','x-brain-static':'1'}),text:async()=>'<main>Static build</main>'})
    const staticPage=predict({},()=>true)
    doc.body.querySelector('a').dispatchEvent(new MouseEvent('focusin',{bubbles:true}))
    await new Promise(r=>setTimeout(r,5))
    assert.equal(await (await staticPage.take('http://x.test/b')).text(),'<main>Static build</main>')
    staticPage.stop()
    globalThis.fetch = async () => ({ok:true,status:200,headers:new Headers({'content-type':'text/html','cache-control':'public, max-age=1'}),text:async()=>'<main>Public data</main>'})
    const publicPage=predict({},()=>true)
    doc.body.querySelector('a').dispatchEvent(new MouseEvent('focusin',{bubbles:true}))
    await new Promise(r=>setTimeout(r,5))
    const now=Date.now
    try {Date.now=()=>now()+2000;assert.equal(await publicPage.take('http://x.test/b'),null)} finally {Date.now=now;publicPage.stop()}
  } finally {globalThis.fetch=real}
})

test('sign-in document policies force full navigation in both directions and preserve same-policy swaps', async () => {
  delete globalThis.navigation
  const oldFetch = globalThis.fetch
  const originalHead = doc.head.innerHTML
  const marker = provider => `<meta name="brain-${provider}-signin" content="1">`
  try {
    for (const [from, to, reload] of [['', 'apple', true], ['', 'google', true], ['apple', '', true], ['google', '', true], ['apple', 'google', false], ['', '', false]]) {
      hist()
      const { m, disposed } = page()
      doc.head.innerHTML = from ? marker(from) : ''
      globalThis.fetch = async () => ({ ok: true, status: 200, headers: new Headers({ 'content-type': 'text/html' }), text: async () => `<head>${to ? marker(to) : ''}</head><main><h1>Target</h1></main>` })
      const hard = [], instance = router({ parse, prefetch: false, hard: u => hard.push(u) })
      try {
        await instance.go('/b')
        assert.deepEqual(hard, reload ? ['/b'] : [], `${from} -> ${to}`)
        assert.equal(m.textContent, reload ? 'Page Aisland' : 'Target')
        assert.equal(disposed(), reload ? 0 : 1)
      } finally { instance.stop() }
    }
  } finally { globalThis.fetch = oldFetch; doc.head.innerHTML = originalHead }
})

test('Navigation API reloads a sign-in redirect before disposing or mounting the current view', async () => {
  const oldFetch = globalThis.fetch, originalHead = doc.head.innerHTML
  hist()
  const { m, disposed } = page()
  doc.head.innerHTML = ''
  let listener, handler
  globalThis.navigation = { addEventListener: (_, f) => { listener = f }, removeEventListener() {} }
  globalThis.fetch = async () => ({ ok: true, status: 200, redirected: true, url: 'http://x.test/login', headers: new Headers({ 'content-type': 'text/html' }), text: async () => '<head><meta name="brain-apple-signin" content="1"></head><main>Sign in</main>' })
  const hard = [], instance = router({ parse, prefetch: false, hard: u => hard.push(u) })
  try {
    listener({ canIntercept: true, destination: { url: 'http://x.test/b' }, intercept: o => { handler = o.handler } })
    await handler()
    assert.deepEqual(hard, ['http://x.test/b'])
    assert.equal(m.textContent, 'Page Aisland')
    assert.equal(disposed(), 0)
  } finally { instance.stop(); delete globalThis.navigation; globalThis.fetch = oldFetch; doc.head.innerHTML = originalHead }
})

test('inert and body-only sign-in markers cannot change the document policy', async () => {
  delete globalThis.navigation
  const oldFetch = globalThis.fetch, originalHead = doc.head.innerHTML
  try {
    for (const content of [
      '<head><!-- <meta name="brain-apple-signin" content="1"> --></head><main>Target</main>',
      '<head><template><meta name="brain-google-signin" content="1"></template></head><main>Target</main>',
      '<head><meta name="brain-apple-signin" content="0"></head><main>Target</main>',
      '<head></head><main><meta name="brain-google-signin" content="1">Target</main>',
    ]) {
      hist(); page(); doc.head.innerHTML = ''
      globalThis.fetch = async () => ({ ok: true, status: 200, headers: new Headers({ 'content-type': 'text/html' }), text: async () => content })
      const hard = [], instance = router({ parse, prefetch: false, hard: u => hard.push(u) })
      try { await instance.go('/b'); assert.deepEqual(hard, [], content) } finally { instance.stop() }
    }
  } finally { globalThis.fetch = oldFetch; doc.head.innerHTML = originalHead }
})

test('zero-JS destinations keep intent prefetch but use full document navigation', async () => {
  delete globalThis.navigation
  hist()
  page()
  const hard = [], asked = [], oldFetch = globalThis.fetch
  globalThis.fetch = async (u, options) => { asked.push([new URL(u, location.href).pathname, options?.headers?.['x-brain-prefetch']]); return oldFetch(u, options) }
  const r = router({ parse, hard: u => hard.push(u), fullLoad: ['/b'] })
  try {
    const a = doc.createElement('a'); a.setAttribute('href', '/b'); doc.querySelector('main').appendChild(a)
    a.dispatchEvent(new MouseEvent('focusin', { bubbles: true }))
    await new Promise(resolve => setTimeout(resolve, 10))
    assert.ok(asked.some(([path, prefetch]) => path === '/b' && prefetch === '1'))
    const click = new MouseEvent('click', { bubbles: true, cancelable: true }); a.dispatchEvent(click)
    assert.equal(click.defaultPrevented, false)
    await r.go('/b/')
    assert.deepEqual(hard, ['/b/'])
    assert.equal(doc.querySelector('main h1').textContent, 'Page A')
  } finally { r.stop(); globalThis.fetch = oldFetch }
})

test('loading groups preserve ordered array matches, individual regex and fallback priority', async () => {
  const { writeFileSync, mkdtempSync, rmSync } = await import('node:fs')
  const { join } = await import('node:path')
  const { tmpdir } = await import('node:os')
  const { pathToFileURL } = await import('node:url')
  const directory = mkdtempSync(join(tmpdir(), 'brain-loading-groups-'))
  const core = pathToFileURL(join(directory, 'core.mjs')).href
  const first = pathToFileURL(join(directory, 'first.mjs')).href
  const fallback = pathToFileURL(join(directory, 'fallback.mjs')).href
  writeFileSync(join(directory, 'core.mjs'), 'export const m=(C,el)=>(el.appendChild(C()),()=>{})')
  writeFileSync(join(directory, 'first.mjs'), 'export default()=>Object.assign(document.createElement("p"),{textContent:"First loader"})')
  writeFileSync(join(directory, 'fallback.mjs'), 'export default()=>Object.assign(document.createElement("p"),{textContent:"Fallback loader"})')
  const originalFetch = globalThis.fetch
  try {
    for (const [loading, expected] of [
      [[{ rx: ['^/unused$', '^/b$'], view: first }, { rx: '^/b$', view: fallback }], 'First loader'],
      [[{ rx: '^/b$', view: fallback }], 'Fallback loader'],
      [[{ rx: ['^/unused$', '^/other$'], view: first }, { rx: '^/b$', view: fallback }], 'Fallback loader']
    ]) {
      hist()
      const { m } = page()
      globalThis.fetch = async (url, options) => { await new Promise(resolve => setTimeout(resolve, 900)); return originalFetch(url, options) }
      const instance = router({ parse, hard: () => assert.fail('unexpected full navigation'), core, loading })
      try {
        const navigation = instance.go('/b')
        await new Promise(resolve => setTimeout(resolve, 700))
        assert.equal(m.textContent, expected)
        await navigation
        assert.equal(m.textContent, 'Page Bb')
      } finally { instance.stop() }
    }
  } finally {
    globalThis.fetch = originalFetch
    rmSync(directory, { recursive: true, force: true })
  }
})


test('navigate rejects raw controls, whitespace and backslashes before URL normalization', async () => {
  const { navigate } = await rt.sibling('brain-router.js')
  const old = globalThis.location, calls = []
  globalThis.location = { assign: v => calls.push(v), replace: v => calls.push(v) }
  try {
    for (const target of ['/\n/evil.test', '/\r/evil.test', '/\t/evil.test', '/ path', '/\\evil.test', '/x\u0000y', '/x\u007fy']) {
      assert.throws(() => navigate(target), { message: 'Invalid navigation target' })
    }
    assert.equal(calls.length, 0)
    navigate('/safe'); navigate('https://example.test/path?q=ok')
    assert.deepEqual(calls, ['/safe', 'https://example.test/path?q=ok'])
  } finally { globalThis.location = old }
})

// --- client navigation without flicker, races or stuck states ---
// A fetch whose responses the test releases by hand: no timing assumptions.
const gated = () => {
  const f0 = globalThis.fetch, waiting = new Map(), signals = new Map()
  globalThis.fetch = (u, o = {}) => {
    const p = new URL(u, 'http://x.test').pathname
    signals.set(p, o.signal)
    return new Promise(res => waiting.set(p, () => res(f0(u, o))))
  }
  const until = async p => { for (let i = 0; i < 500 && !waiting.has(p); i++) await new Promise(r => setTimeout(r, 2)) }
  return {
    release: async p => (await until(p), waiting.get(p)()),
    signal: p => signals.get(p),
    restore: () => (globalThis.fetch = f0),
  }
}
const settle = () => new Promise(r => setTimeout(r, 5))

test('a superseded navigation never swaps, scrolls or moves focus after the newer one (History)', async () => {
  hist()
  const scrolls = []
  globalThis.scrollTo = (x, y) => scrolls.push(y)
  const { m } = page()
  const g = gated()
  const r = router({ parse, hard: u => assert.fail('hard ' + u) })
  const slow = r.go('/b'), fast = r.go('/c')
  await g.release('/c')
  await fast
  assert.equal(m.textContent, 'Page C')
  assert.equal(scrolls.length, 1)
  assert.equal(doc.activeElement, m.firstChild)
  doc.body.focus && doc.body.focus()
  await g.release('/b')
  await slow
  assert.equal(m.textContent, 'Page C') // the stale response is dropped
  assert.equal(scrolls.length, 1) // and it does not scroll the newer page back to the top
  g.restore()
  r.stop()
})

test('a newer navigation aborts the fetch of the one it replaced, and a finished one is left alone', async () => {
  hist()
  page()
  const real = globalThis.fetch, seen = []
  globalThis.fetch = (u, o) => (seen.push({ u, signal: o.signal }), u === '/b'
    ? new Promise((_, rej) => o.signal.addEventListener('abort', () => rej(new DOMException('Aborted', 'AbortError'))))
    : real(u, o))
  const r = router({ parse, hard: u => assert.fail('hard ' + u) })
  const slow = r.go('/b')
  while (!seen.length) await null
  assert.equal(seen[0].signal.aborted, false)
  const fast = r.go('/c')
  assert.equal(seen[0].signal.aborted, true)
  await slow
  await fast
  assert.equal(seen[1].signal.aborted, false)
  globalThis.fetch = real
  r.stop()
})

test('a swap that throws becomes a full page load: no half-swapped page, no unhandled rejection', async () => {
  hist()
  page()
  const hard = []
  const r = router({ parse, hard: u => hard.push(u) })
  const hook = globalThis.__brainRouted
  globalThis.__brainRouted = on => { if (on === undefined) throw Error('route hook failed') }
  try {
    await r.go('/b')
  } finally {
    globalThis.__brainRouted = hook
  }
  assert.deepEqual(hard, ['/b'])
  r.stop()
})

test('a view transition whose navigation was superseded does not swap the old page in (no flash of stale content)', async () => {
  hist()
  const { m } = page()
  const queued = []
  doc.startViewTransition = f => {
    let done
    const p = new Promise(r => (done = r))
    queued.push(() => (f(), done()))
    return { ready: Promise.resolve(), updateCallbackDone: p }
  }
  const g = gated()
  const r = router({ parse, hard: u => assert.fail('hard ' + u) })
  const first = r.go('/b')
  await g.release('/b')
  for (let i = 0; i < 200 && !queued.length; i++) await settle()
  const second = r.go('/c') // starts before the first transition ran its callback
  queued.shift()()
  assert.equal(m.textContent, 'Page Aisland') // the superseded callback swapped nothing
  await g.release('/c')
  for (let i = 0; i < 200 && !queued.length; i++) await settle()
  queued.shift()()
  await Promise.all([first, second])
  assert.equal(m.textContent, 'Page C')
  delete doc.startViewTransition
  g.restore()
  r.stop()
})

test('feedback: the region is aria-busy and route().navigating is true while a navigation waits; #brain-nav shows only past the delay', async () => {
  hist()
  const { m } = page()
  const { route } = await rt.sibling('brain-env.js')
  const rr = route()
  const own = doc.createElement('div')
  own.id = 'brain-nav'
  own.hidden = true
  doc.body.appendChild(own)
  const g = gated()
  const r = router({ parse, hard: u => assert.fail('hard ' + u) })
  const nav = r.go('/b')
  assert.equal(m.getAttribute('aria-busy'), 'true')
  assert.equal(rr.navigating, true)
  assert.equal(own.hidden, true) // not before the delay
  await new Promise(r => setTimeout(r, 100))
  assert.equal(own.hidden, true) // still hidden at 100 ms: a fast navigation shows nothing
  for (let i = 0; i < 1000 && own.hidden; i++) await settle()
  assert.equal(own.hidden, false) // the app's own #brain-nav replaces the built-in bar
  assert.equal(doc.querySelectorAll('progress').length, 0)
  await g.release('/b')
  await nav
  assert.equal(m.hasAttribute('aria-busy'), false)
  assert.equal(rr.navigating, false)
  assert.equal(own.hidden, true)
  own.remove()
  // without one, the router adds a fixed <progress id="brain-nav"> the first time a navigation waits
  const nav2 = r.go('/c')
  for (let i = 0; i < 1000 && !doc.getElementById('brain-nav'); i++) await settle()
  const bar = doc.getElementById('brain-nav')
  assert.equal(bar.tagName, 'PROGRESS')
  assert.equal(bar.hidden, false)
  await g.release('/c')
  await nav2
  assert.equal(bar.hidden, true)
  bar.remove()
  g.restore()
  r.stop()
})

test('prefetch: a navigation reuses the request in flight, however slow (never a second fetch)', async () => {
  globalThis.location = new URL('http://x.test/a')
  const { predict } = await rt.sibling('brain-router.js')
  const real = globalThis.fetch, asked = []
  let finish
  globalThis.fetch = async (u, o) => {
    asked.push(new URL(u).pathname)
    await new Promise(r => (finish = r))
    return real(u, o)
  }
  doc.body.innerHTML = '<a href="/b">B</a>'
  const a = doc.body.querySelector('a')
  const p = predict({}, () => true)
  a.dispatchEvent(new MouseEvent('pointerover', { bubbles: true }))
  for (let i = 0; i < 500 && !asked.length; i++) await settle()
  assert.deepEqual(asked, ['/b'])
  const taking = p.take('http://x.test/b')
  await new Promise(r => setTimeout(r, 200)) // slower than any old 150 ms cut-off
  finish()
  const got = await taking
  assert.equal(await got.text(), pages['/b'])
  assert.deepEqual(asked, ['/b'])
  p.stop()
  globalThis.fetch = real
})

test('prefetch: a dynamic (no-cache) page taken while its request is in flight is used, not fetched twice', async () => {
  globalThis.location = new URL('http://x.test/a')
  const { predict } = await rt.sibling('brain-router.js')
  const real = globalThis.fetch, asked = []
  let finish
  globalThis.fetch = async u => {
    asked.push(new URL(u).pathname)
    await new Promise(r => (finish = r))
    return { ok: true, status: 200, headers: new Headers({ 'content-type': 'text/html', 'cache-control': 'no-cache' }), text: async () => '<main>Dynamic</main>' }
  }
  doc.body.innerHTML = '<a href="/b">B</a>'
  const p = predict({}, () => true)
  doc.body.querySelector('a').dispatchEvent(new MouseEvent('focusin', { bubbles: true }))
  for (let i = 0; i < 500 && !asked.length; i++) await settle()
  const taking = p.take('http://x.test/b')
  await settle()
  finish()
  assert.equal(await (await taking).text(), '<main>Dynamic</main>')
  assert.deepEqual(asked, ['/b'])
  p.stop()
  globalThis.fetch = real
})

test('B05: repeated start/stop leaves no service worker listener behind (History and Navigation API)', async () => {
  const nav0 = Object.getOwnPropertyDescriptor(globalThis, 'navigator')
  const live = new Set()
  Object.defineProperty(globalThis, 'navigator', { configurable: true, value: { serviceWorker: { addEventListener: (t, f) => live.add(f), removeEventListener: (t, f) => live.delete(f) } } })
  try {
    for (const api of [false, true]) {
      if (api) globalThis.navigation = { addEventListener() {}, removeEventListener() {}, navigate() {} }
      else hist()
      page()
      for (let i = 0; i < 5; i++) {
        const r = router({ parse, hard: u => assert.fail('hard ' + u) })
        assert.equal(live.size, 1)
        r.stop()
        assert.equal(live.size, 0, (api ? 'Navigation API' : 'History') + ' stop() keeps the listener')
      }
      delete globalThis.navigation
    }
  } finally {
    if (nav0) Object.defineProperty(globalThis, 'navigator', nav0)
    else delete globalThis.navigator
  }
})

test('prefetch reuse through the router: a click on a link whose prefetch is in flight fetches the page once', async () => {
  hist()
  const { m } = page()
  const real = globalThis.fetch, asked = []
  let finish
  globalThis.fetch = async (u, o) => {
    asked.push(new URL(u, 'http://x.test').pathname)
    await new Promise(r => (finish = r))
    return real(u, o)
  }
  const a = doc.createElement('a')
  a.setAttribute('href', '/b')
  doc.body.appendChild(a)
  const r = router({ parse, hard: u => assert.fail('hard ' + u) })
  a.dispatchEvent(new MouseEvent('pointerover', { bubbles: true }))
  for (let i = 0; i < 500 && !asked.length; i++) await settle()
  const nav = r.go('/b')
  await new Promise(r => setTimeout(r, 200))
  finish()
  await nav
  assert.equal(m.textContent, 'Page Bb')
  assert.deepEqual(asked, ['/b'])
  r.stop()
  globalThis.fetch = real
})

const click = el => el.dispatchEvent(new MouseEvent('click', { bubbles: true, cancelable: true }))
const until = async f => { for (let i = 0; i < 500 && !f(); i++) await settle() }

test('first click: a click during an in-flight prefetch navigates at once and reuses that request', async () => {
  hist()
  const { m } = page()
  const real = globalThis.fetch, asked = []
  let finish
  globalThis.fetch = async (u, o) => {
    asked.push(new URL(u, 'http://x.test').pathname)
    await new Promise(r => (finish = r))
    return real(u, o)
  }
  const a = doc.createElement('a')
  a.setAttribute('href', '/b')
  doc.body.appendChild(a)
  const r = router({ parse, hard: u => assert.fail('hard ' + u) })
  a.dispatchEvent(new MouseEvent('pointerover', { bubbles: true }))
  await until(() => asked.length)
  click(a)
  assert.equal(m.getAttribute('aria-busy'), 'true') // the click started the navigation before the prefetch ended
  finish()
  await until(() => m.textContent === 'Page Bb')
  assert.equal(m.textContent, 'Page Bb')
  assert.deepEqual(asked, ['/b'])
  r.stop()
  globalThis.fetch = real
})

test('first click: a failed prefetch is never reused: the click fetches again and navigates', async () => {
  hist()
  const { m } = page()
  const real = globalThis.fetch, asked = []
  globalThis.fetch = async (u, o) => {
    asked.push(new URL(u, 'http://x.test').pathname)
    if (asked.length === 1) throw Error('prefetch failed')
    return real(u, o)
  }
  const a = doc.createElement('a')
  a.setAttribute('href', '/b')
  doc.body.appendChild(a)
  const r = router({ parse, hard: u => assert.fail('hard ' + u) })
  a.dispatchEvent(new MouseEvent('pointerover', { bubbles: true }))
  await until(() => asked.length)
  await settle()
  click(a)
  await until(() => m.textContent === 'Page Bb')
  assert.equal(m.textContent, 'Page Bb')
  assert.deepEqual(asked, ['/b', '/b'])
  r.stop()
  globalThis.fetch = real
})

test('first click: a link clicked before the view chunk arrives navigates; it is not held for the mount', async () => {
  hist()
  const { m } = page()
  pages['/ml'] = '<main><a href="/c">C</a><script type="module">const e=document.getElementById(\'brain-view\');e.$d=m(V,e,{})</script></main>'
  const r = router({ parse, hard: u => assert.fail('hard ' + u) })
  await r.go('/ml')
  click(m.querySelector('a')) // the mount script has not run: its chunk is still on its way
  await until(() => m.textContent === 'Page C')
  assert.equal(m.textContent, 'Page C')
  r.stop()
})

test('first click: a page without a mount script does not inherit the previous page\'s click hold', async () => {
  hist()
  const { m } = page()
  const r = router({ parse, hard: u => assert.fail('hard ' + u) })
  await r.go('/m')
  pages['/plain'] = '<main><button type="button">Go</button></main>'
  await r.go('/plain')
  const seen = []
  m.querySelector('button').addEventListener('click', () => seen.push('button'))
  click(m.querySelector('button'))
  assert.deepEqual(seen, ['button'])
  r.stop()
})

test('first click: a navigation that never answers ends in a full load, never in nothing', async () => {
  hist()
  page()
  const real = globalThis.fetch, st = globalThis.setTimeout, hard = []
  globalThis.fetch = () => new Promise(() => {})
  globalThis.setTimeout = (f, ms, ...a) => st(f, ms >= 8000 && ms < 9000 ? 1 : ms, ...a)
  try {
    const a = doc.createElement('a')
    a.setAttribute('href', '/b')
    doc.body.appendChild(a)
    const r = router({ parse, prefetch: false, hard: u => hard.push(u) })
    click(a)
    await until(() => hard.length)
    assert.deepEqual(hard, ['http://x.test/b'])
    r.stop()
  } finally {
    globalThis.fetch = real
    globalThis.setTimeout = st
  }
})

test('CSP: recreated scripts keep the page nonce on every navigation, even once fetched head data carries another', async () => {
  hist()
  const { m } = page()
  const ld = doc.createElement('script')
  ld.setAttribute('type', 'application/ld+json')
  ld.setAttribute('nonce', 'PAGE-NONCE')
  doc.head.appendChild(ld)
  const body = (nonce, x) => `<script type="application/ld+json" nonce="${nonce}">{}</script><main><script type="b/state" nonce="${nonce}">{}</script><script type="module" nonce="${nonce}">${x}()</script></main>`
  pages['/n1'] = body('N1', 'a')
  pages['/n2'] = body('N2', 'b')
  const r = router({ parse, hard: u => assert.fail('hard ' + u) })
  for (const p of ['/n1', '/n2', '/n1']) {
    await r.go(p)
    assert.equal(m.querySelector('script[type=module]').getAttribute('nonce'), 'PAGE-NONCE', p)
  }
  for (const s of doc.head.querySelectorAll('script')) s.remove()
  r.stop()
})

test('viewport prefetch: visible links only, never fullLoad routes; saveData asks nothing; a failure leaves later clicks alone', async () => {
  hist()
  const { m } = page()
  const io = []
  globalThis.IntersectionObserver = class {
    constructor(f) { this.f = f; this.els = []; io.push(this) }
    observe(el) { this.els.push(el) }
    unobserve() {}
    disconnect() {}
  }
  globalThis.requestIdleCallback = f => setTimeout(f, 0)
  const real = globalThis.fetch, asked = []
  globalThis.fetch = async (u, o) => {
    asked.push(new URL(u, 'http://x.test').pathname)
    if (asked.length === 1) throw Error('prefetch failed')
    return real(u, o)
  }
  doc.body.insertAdjacentHTML?.('beforeend', '')
  for (const h of ['/b', '/c']) {
    const a = doc.createElement('a')
    a.setAttribute('href', h)
    doc.body.appendChild(a)
  }
  const r = router({ parse, fullLoad: ['/c'], hard: () => {} })
  const see = () => io[0].f(io[0].els.map(target => ({ target, isIntersecting: true })))
  see()
  await until(() => asked.length)
  await settle()
  assert.deepEqual(asked, ['/b'])
  click(doc.body.querySelector('a[href="/b"]'))
  await until(() => m.textContent === 'Page Bb')
  assert.equal(m.textContent, 'Page Bb')
  r.stop()
  // saveData: nothing is asked ahead of time
  const nav0 = Object.getOwnPropertyDescriptor(globalThis, 'navigator')
  Object.defineProperty(globalThis, 'navigator', { configurable: true, value: { connection: { saveData: true } } })
  asked.length = 0
  io.length = 0
  page()
  const a = doc.createElement('a')
  a.setAttribute('href', '/b')
  doc.body.appendChild(a)
  const r2 = router({ parse, hard: () => {} })
  see()
  await new Promise(r => setTimeout(r, 20))
  assert.deepEqual(asked, [])
  r2.stop()
  if (nav0) Object.defineProperty(globalThis, 'navigator', nav0)
  else delete globalThis.navigator
  globalThis.fetch = real
  delete globalThis.IntersectionObserver
  delete globalThis.requestIdleCallback
})

test('prefetch queue: intent, then main by position, then footer and aside; a link leaving the viewport is dropped; a click never waits', async () => {
  hist()
  const { m } = page()
  const io = []
  globalThis.IntersectionObserver = class {
    constructor(f) { this.f = f; io.push(this) }
    observe() {}
    unobserve() {}
    disconnect() {}
  }
  let idle
  globalThis.requestIdleCallback = f => (idle = f, 1)
  const real = globalThis.fetch, asked = [], open = []
  globalThis.fetch = async (u, o) => {
    asked.push(new URL(u, 'http://x.test').pathname)
    await new Promise(r => open.push(r))
    return real(u, o)
  }
  for (const p of ['/m1', '/m2', '/m3', '/gone', '/foot', '/side', '/hover', '/clicked']) pages[p] = `<main><h1>${p}</h1></main>`
  const link = (h, top, parent = doc.body) => {
    const a = doc.createElement('a')
    a.setAttribute('href', h)
    a.getBoundingClientRect = () => ({ top })
    parent.appendChild(a)
    return a
  }
  const foot = doc.createElement('footer'), side = doc.createElement('aside')
  doc.body.appendChild(foot)
  doc.body.appendChild(side)
  const f = link('/foot', 10, foot), s = link('/side', 5, side)
  const m3 = link('/m3', 300), m1 = link('/m1', 100), m2 = link('/m2', 200), gone = link('/gone', 50), hover = link('/hover', 900), clicked = link('/clicked', 950)
  const r = router({ parse, hard: u => assert.fail('hard ' + u) })
  const see = (a, isIntersecting = true) => io[0].f([{ target: a, isIntersecting }])
  see(f) // the footer intersects first and still runs after the main links
  see(s)
  see(m3); see(m1); see(m2); see(gone)
  see(gone, false)
  hover.dispatchEvent(new MouseEvent('pointerover', { bubbles: true })) // jumps ahead; takes one of the two slots
  idle()
  assert.deepEqual(asked, ['/hover', '/m1'])
  for (let n = 0; n < 8; n++) { open.splice(0).forEach(f => f()); await settle() }
  assert.deepEqual(asked.slice(0, 2).sort(), ['/hover', '/m1'])
  assert.deepEqual(asked.slice(2), ['/m2', '/m3', '/side', '/foot']) // aside at 5 and footer at 10 rank by their position too
  assert.equal(asked.includes('/gone'), false)
  // a click never waits on the queue, even with every slot taken
  asked.length = 0
  for (const a of [m1, m2, m3]) doc.body.appendChild(a)
  const busy1 = doc.createElement('a'), busy2 = doc.createElement('a')
  pages['/busy1'] = pages['/busy2'] = '<main><h1>busy</h1></main>'
  busy1.setAttribute('href', '/busy1'); busy2.setAttribute('href', '/busy2')
  doc.body.appendChild(busy1); doc.body.appendChild(busy2)
  busy1.dispatchEvent(new MouseEvent('pointerover', { bubbles: true }))
  busy2.dispatchEvent(new MouseEvent('pointerover', { bubbles: true }))
  assert.deepEqual(asked, ['/busy1', '/busy2'])
  click(clicked)
  await until(() => asked.length === 3)
  assert.deepEqual(asked, ['/busy1', '/busy2', '/clicked'])
  for (let n = 0; n < 8; n++) { open.splice(0).forEach(f => f()); await settle() }
  await until(() => m.textContent === '/clicked')
  assert.equal(m.textContent, '/clicked')
  r.stop()
  globalThis.fetch = real
  delete globalThis.IntersectionObserver
  delete globalThis.requestIdleCallback
})

// Focus and announcement after a client navigation (§19). The Navigation API resets focus to body unless told otherwise.
const viaNavigation = () => {
  globalThis.location = new URL('http://x.test/a')
  let h
  globalThis.navigation = { addEventListener: (t, f) => (h = f), removeEventListener() {}, navigate() {} }
  delete doc.startViewTransition
  return async (url, o = {}) => {
    let opts
    h({ canIntercept: true, destination: { url }, intercept: x => (opts = x), ...o })
    await opts.handler()
    return opts
  }
}

test('Navigation API: focus stays ours, lands on the new h1 and the title is announced', async () => {
  const go = viaNavigation()
  const { m } = page()
  const r = router({ parse, hard: u => assert.fail('hard ' + u) })
  const live = doc.getElementById('brain-announce')
  assert.equal(live.getAttribute('aria-live'), 'polite') // present before the first navigation: screen readers need that
  assert.equal(live.textContent, '')
  const opts = await go('http://x.test/b')
  assert.equal(opts.focusReset, 'manual')
  assert.equal(doc.activeElement, m.firstChild)
  assert.equal(m.firstChild.textContent, 'Page B')
  assert.equal(m.firstChild.getAttribute('tabindex'), '-1')
  assert.equal(live.textContent, 'Title Page B')
  r.stop()
  delete globalThis.navigation
})

test('without an h1 focus lands on the region, and a URL fragment target wins over the h1', async () => {
  const go = viaNavigation()
  const { m } = page()
  m.id = 'brain-view'
  pages['/plain'] = '<div id="brain-view"><p>no heading</p></div>'
  pages['/frag'] = '<div id="brain-view"><h1>Frag</h1><section id="part">x</section></div>'
  const r = router({ parse, region: '#brain-view', hard: u => assert.fail('hard ' + u) })
  await go('http://x.test/plain')
  assert.equal(doc.activeElement.getAttribute('id') ?? doc.activeElement.id, 'brain-view')
  await go('http://x.test/frag#part')
  assert.equal(doc.activeElement.getAttribute('id'), 'part')
  r.stop()
  delete pages['/plain'], delete pages['/frag']
  delete globalThis.navigation
})

test('a focus the new page set itself is respected: no move to the h1, even after its mount', async () => {
  const go = viaNavigation()
  const { m } = page()
  pages['/self'] = '<main><h1>Self</h1><input id="own"></main>'
  globalThis.__brainRouted = on => { if (on === undefined) m.querySelector('#own')?.focus() }
  const r = router({ parse, hard: u => assert.fail('hard ' + u) })
  await go('http://x.test/self')
  assert.equal(doc.activeElement.getAttribute('id'), 'own')
  assert.equal(doc.getElementById('brain-announce').textContent, 'Title Self') // the title is still read
  r.stop()
  delete globalThis.__brainRouted
  delete pages['/self']
  delete globalThis.navigation
})

test('mounting that replaces the server nodes brings a lost focus back, but never yanks one the user moved', async () => {
  const go = viaNavigation()
  const { m } = page()
  m.id = 'brain-view'
  const r = router({ parse, hard: u => assert.fail('hard ' + u) })
  await go('http://x.test/m')
  assert.equal(doc.activeElement, m.querySelector('h1'))
  // the mount script runs now: new nodes, the focused h1 is gone
  m.textContent = ''
  doc.activeElement = doc.body
  m.innerHTML = '<h1>Mount</h1>'
  m.$d = () => {}
  assert.equal(doc.activeElement, m.querySelector('h1'))
  // again, with the user already on a nav link outside the region
  m.$d(); m.$d = null
  await go('http://x.test/m')
  const nav = doc.body.appendChild(doc.createElement('a'))
  nav.setAttribute('href', '#x')
  nav.focus()
  m.textContent = ''
  m.innerHTML = '<h1>Mount</h1>'
  m.$d = () => {}
  assert.equal(doc.activeElement, nav)
  r.stop()
  delete globalThis.navigation
})

test('a failed navigation neither moves focus nor announces', async () => {
  const go = viaNavigation()
  page()
  const hard = []
  const r = router({ parse, hard: u => hard.push(u) })
  doc.activeElement = doc.body
  await go('http://x.test/missing')
  assert.deepEqual(hard, ['http://x.test/missing'])
  assert.equal(doc.activeElement, doc.body)
  assert.equal(doc.getElementById('brain-announce').textContent, '')
  r.stop()
  delete globalThis.navigation
})

test('predictive prefetch: the rank map stays bounded across many navigations with distinct links', async () => {
  globalThis.location = new URL('http://x.test/a')
  const { predict } = await rt.sibling('brain-router.js')
  const io = []
  globalThis.IntersectionObserver = class {
    constructor(f) { this.f = f; io.push(this) }
    observe(el) { ;(this.els ||= []).push(el) }
    unobserve() {}
    disconnect() { this.els = [] }
  }
  globalThis.requestIdleCallback = f => setTimeout(f, 0)
  const real = globalThis.fetch, RealMap = globalThis.Map, made = []
  globalThis.fetch = async () => ({ ok: true, status: 200, headers: { get: k => (k === 'content-type' ? 'text/html' : null) }, text: async () => '<main></main>' })
  // the maps predict() builds are counted from outside, without a hook in the router
  globalThis.Map = class extends RealMap { constructor(...a) { super(...a); made.push(this) } }
  let p
  try { p = predict({}, () => true) } finally { globalThis.Map = RealMap }
  try {
    for (let round = 0; round < 40; round++) {
      doc.body.innerHTML = '<main>' + Array.from({ length: 10 }, (_, i) => `<a href="/r${round}-${i}">L</a>`).join('') + '</main>'
      p.scan()
      io[0].f(io[0].els.map(target => ({ target, isIntersecting: true })))
      await new Promise(r => setTimeout(r, 2))
    }
    await new Promise(r => setTimeout(r, 20))
    const biggest = Math.max(...made.map(m => m.size))
    assert.ok(biggest <= 24, `a prefetch map holds ${biggest} entries after 400 distinct links`)
  } finally {
    p.stop()
    globalThis.fetch = real
    delete globalThis.IntersectionObserver
    delete globalThis.requestIdleCallback
  }
})
