// brain-links.js: links a view mounts on a page outside the default language keep that language.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { install } from '../shim/dom.js'

const doc = install()
const rt = await import('./rt.js')
let notify
globalThis.MutationObserver = class {
  constructor(f) { notify = f }
  observe() {}
}
globalThis.location = { pathname: '/es/about' }
const mod = await rt.sibling('brain-links.js')
const link = (href, extra = {}) => {
  const a = doc.createElement('a')
  a.setAttribute('href', href)
  for (const k in extra) a.setAttribute(k, extra[k])
  a.matches = () => true // the shim has no selector engine
  return a
}

test('rewrites page links as they mount and keeps what is not a page', () => {
  doc.documentElement.setAttribute('lang', 'es')
  mod.default('es', '', '/es', '^(?:|/about|/posts/[^/]+)/?$', { '/contact': '/es/contacto' })
  const to = globalThis.__brainHref
  assert.equal(to('/'), '/es')
  assert.equal(to('/posts/7?a=1#b'), '/es/posts/7?a=1#b')
  assert.equal(to('/contact'), '/es/contacto')
  assert.equal(to('/favicon.svg'), '/favicon.svg')
  assert.equal(to('//cdn/x'), '//cdn/x')
  const here = link('/about'), other = link('/about', { hreflang: 'en' })
  notify([{ type: 'childList', addedNodes: [here, other] }])
  assert.equal(here.getAttribute('href'), '/es/about')
  assert.equal(here.getAttribute('aria-current'), 'page', 'the active link is marked after the rewrite')
  assert.equal(other.getAttribute('href'), '/about', 'hreflang names a language on purpose')
})

test('a page in another language is left alone', () => {
  doc.documentElement.setAttribute('lang', 'en')
  assert.equal(globalThis.__brainHref('/about'), '/about')
})
