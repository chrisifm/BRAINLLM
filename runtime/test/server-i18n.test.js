// BUILD (QA #153): <meta seo title={t('…')}> translated in SSR. The route's dictionary (`route.i18n`) is loaded before
// render, the server module's seo() runs inside the page locale, and two copies of brain-i18n.js share one table.
import { test, after } from 'node:test'
import assert from 'node:assert/strict'
import { copyFileSync, mkdirSync, mkdtempSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = fileURLToPath(new URL('.', import.meta.url))
const dist = mkdtempSync(join(tmpdir(), 'brain-i18n-ssr-'))
const w = (rel, s) => (mkdirSync(join(dist, rel, '..'), { recursive: true }), writeFileSync(join(dist, rel), s))
for (const f of ['brain.js', 'brain-i18n.js']) mkdirSync(join(dist, '_b'), { recursive: true }), copyFileSync(join(here, '..', f), join(dist, '_b', f))
mkdirSync(join(dist, '_server/_rt'), { recursive: true })
copyFileSync(join(here, '../brain-i18n.js'), join(dist, '_server/_rt/brain-i18n.js')) // the server module's own copy
copyFileSync(join(here, '../shim/dom.js'), join(dist, '_server/dom.js'))
for (const f of ['brain.js', 'brain-form.js', 'brain-schema.js']) copyFileSync(join(here, '..', f), join(dist, '_server', f))
w('_b/dict-nl.js', `import { add } from '/_b/brain-i18n.js'\nadd('nl', { 'home.title': 'Startpagina', 'home.hi': 'Hallo' })\n`)
w('_b/home.js', `import { tpl } from '/_b/brain.js'\nimport { t } from '/_b/brain-i18n.js'\nexport default function Home() { return tpl('<h1>' + t('home.hi') + '</h1>')() }\n`)
w('_server/home.server.mjs', `import { t } from './_rt/brain-i18n.js'\nexport const seo = () => ({ title: t('home.title') })\nexport function load() { return {} }\n`)
w('_b/dict-en.js', `import { add } from '/_b/brain-i18n.js'\nadd('en', { 'home.hi': 'Hello' })\n`)
w('_b/late.js', `import { tpl } from '/_b/brain.js'\nexport default function Late(p) { return tpl('<h1>' + p.msg + '</h1>')() }\n`)
w('_server/late.server.mjs', `import { t, locale } from './_rt/brain-i18n.js'\nexport async function load() { await new Promise(r => setTimeout(r, 5)); return { msg: t('home.hi') + ' ' + locale() } }\nconst redirect = to => ({ redirect: to, status: 303 })\nexport async function send() { await new Promise(r => setTimeout(r, 1)); return redirect('/late/nl?m=' + t('home.hi')) }\nexport const $forms = {}\nexport const $server = ['send']\n`)
w('_server/pages/p.html', '<!doctype html><html lang="nl"><head><title>t</title></head><body><div id="brain-view"><!--brain:ssr--></div><script type="application/json" id="brain-props"><!--brain:props--></script></body></html>')
w('_b/link-proof.js', `import {tpl} from '/_b/brain.js';export default()=>tpl('<main><a href="/es">Home</a><a href="/contact/?q=1#part">Contact</a></main>')()`)
w('_server/link-proof.mjs', `export const seo=()=>({canonical:'/contact/'});export function load(){return {}}`)
w('_server/pages/links.html', '<!doctype html><html lang="es"><head><title>Links</title><link rel="canonical" href="https://example.test/"></head><body><div id="brain-view"><!--brain:ssr--></div><script type="application/json" id="brain-props"><!--brain:props--></script></body></html>')
w('_server/manifest.json', JSON.stringify({
  v: 1, build: 'b1', core: '/_b/brain.js', rpc: { hf: '_server/late.server.mjs' },
  routes: [{ path: '/nl', rx: '^/nl/?$', view: '/_b/home.js', layouts: [], server: '_server/home.server.mjs', page: '_server/pages/p.html', locale: 'nl', i18n: '/_b/dict-nl.js' },
    { path: '/late/nl', rx: '^/late/nl/?$', view: '/_b/late.js', layouts: [], server: '_server/late.server.mjs', page: '_server/pages/p.html', locale: 'nl', i18n: '/_b/dict-nl.js' },
    { path: '/late', rx: '^/late/?$', view: '/_b/late.js', layouts: [], server: '_server/late.server.mjs', page: '_server/pages/p.html', locale: 'en', i18n: '/_b/dict-en.js' },
    { path: '/es/contact-page', rx: '^/es/contact-page/?$', view: '/_b/link-proof.js', layouts: [], server: '_server/link-proof.mjs', page: '_server/pages/links.html', locale: 'es', hrefs: { p: '/es', re: '^(?:|/[^/]+)/?$', s: { '/contact': '/es/contact-page' } } }],
}))

const { createServer } = await import('../server/server.mjs')
const server = createServer({ dist, log() {} })
await new Promise(r => server.listen(0, '127.0.0.1', r))
after(() => server.close())

test('SSR preserves localized dynamic prefixes and translates trailing-slash links and explicit canonicals', async () => {
  const response = await fetch(`http://127.0.0.1:${server.address().port}/es/contact-page/`)
  const html=await response.text()
  assert.equal(response.status,200)
  assert.match(html, /href="\/es"/)
  assert.match(html, /href="\/es\/contact-page\/\?q=1#part"/)
  assert.match(html, /rel="canonical" href="https:\/\/example.test\/es\/contact-page\/"/)
  assert.doesNotMatch(html, /\/es\/es/)
})

test('SSR in the route locale: the body and the <title> from <meta seo> are translated', async () => {
  const html = await (await fetch(`http://127.0.0.1:${server.address().port}/nl`)).text()
  assert.match(html, /<h1>Hallo<\/h1>/)
  assert.match(html, /<title>Startpagina<\/title>/)
  assert.equal(globalThis.__brainLocale ?? null, null) // nothing left behind
})

test('t() and locale() inside load() read the page language, also across an await and with requests overlapping', async () => {
  const base = `http://127.0.0.1:${server.address().port}`
  const [a, b] = await Promise.all([fetch(`${base}/late/nl`), fetch(`${base}/late`)])
  const [nl, en] = [await a.text(), await b.text()]
  assert.match(nl, /<h1>Hallo nl<\/h1>/)
  assert.match(en, /<h1>Hello en<\/h1>/)
})

test('a form handler reads the page language after the validation, inside its own await', async () => {
  const r = await fetch(`http://127.0.0.1:${server.address().port}/late/nl`, { method: 'POST', redirect: 'manual', headers: { origin: `http://127.0.0.1:${server.address().port}`, 'content-type': 'application/x-www-form-urlencoded' }, body: '_brain=hf/send' })
  assert.equal(r.status, 303)
  assert.equal(r.headers.get('location'), '/late/nl?m=Hallo')
})
