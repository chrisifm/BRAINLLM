// Internal links keep the page's language (INTERNALS §37). The views compile once for every language, so their
// `href="/about"` reaches a page in `es` unprefixed: `brain build` rewrites the HTML it writes, and this module, loaded
// only by pages outside the default language, rewrites what the views mount afterwards. A link with `hreflang` points
// at a language on purpose and is left alone. go()/navigate() read the same table through __brainHref.
const G = globalThis
const SEL = 'a[href],form[action]'
let tables

// b: base path without its trailing slash; p: language prefix; re: paths with a page; s: translated routes.
export default function links(lang, b, p, re, s) {
  if (tables) return void (tables[lang] = { b, p, re: new RegExp(re), s })
  tables = { [lang]: { b, p, re: new RegExp(re), s } }
  const to = h => {
    const t = tables[document.documentElement.getAttribute('lang')]
    if (!t || !h || h[0] !== '/' || h[1] === '/') return h
    const a = t.b && (h + '/').startsWith(t.b + '/') ? h.slice(t.b.length) || '/' : h
    const m = /^[^?#]*/.exec(a)[0], key = m.replace(/\/+$/, '') || '/'
    if (key === t.p || key.startsWith(t.p + '/')) return h
    let n = t.s[key] || (t.re.test(key) && (key === '/' ? t.p : t.p + key))
    if (n && key !== '/' && m.endsWith('/') && !n.endsWith('/')) n += '/'
    return n ? t.b + n + a.slice(m.length) : h
  }
  G.__brainHref = to
  const fix = e => {
    const n = e.localName === 'form' ? 'action' : 'href', h = e.getAttribute(n), v = e.hasAttribute('hreflang') ? h : to(h)
    if (v === h) return
    e.setAttribute(n, v)
    if (n === 'href' && v.split(/[?#]/)[0] === location.pathname) e.setAttribute('aria-current', 'page')
  }
  const all = r => r.querySelectorAll?.(SEL).forEach(fix)
  new MutationObserver(l => l.forEach(m => {
    if (m.type === 'attributes') return fix(m.target)
    m.addedNodes.forEach(n => {
      if (n.matches?.(SEL)) fix(n)
      all(n)
    })
  })).observe(document, { subtree: true, childList: true, attributes: true, attributeFilter: ['href', 'action'] })
  all(document)
}
