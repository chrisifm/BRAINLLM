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
w('_b/dict-nl.js', `import { add } from '/_b/brain-i18n.js'\nadd('nl', { 'home.title': 'Startpagina', 'home.hi': 'Hallo' })\n`)
w('_b/home.js', `import { tpl } from '/_b/brain.js'\nimport { t } from '/_b/brain-i18n.js'\nexport default function Home() { return tpl('<h1>' + t('home.hi') + '</h1>')() }\n`)
w('_server/home.server.mjs', `import { t } from './_rt/brain-i18n.js'\nexport const seo = () => ({ title: t('home.title') })\nexport function load() { return {} }\n`)
w('_server/pages/p.html', '<!doctype html><html lang="nl"><head><title>t</title></head><body><div id="brain-view"><!--brain:ssr--></div><script type="application/json" id="brain-props"><!--brain:props--></script></body></html>')
w('_server/manifest.json', JSON.stringify({
  v: 1, build: 'b1', core: '/_b/brain.js', rpc: {},
  routes: [{ path: '/nl', rx: '^/nl/?$', view: '/_b/home.js', layouts: [], server: '_server/home.server.mjs', page: '_server/pages/p.html', locale: 'nl', i18n: '/_b/dict-nl.js' }],
}))

const { createServer } = await import('../server/server.mjs')
const server = createServer({ dist, log() {} })
await new Promise(r => server.listen(0, '127.0.0.1', r))
after(() => server.close())

test('SSR in the route locale: the body and the <title> from <meta seo> are translated', async () => {
  const html = await (await fetch(`http://127.0.0.1:${server.address().port}/nl`)).text()
  assert.match(html, /<h1>Hallo<\/h1>/)
  assert.match(html, /<title>Startpagina<\/title>/)
  assert.equal(globalThis.__brainLocale ?? null, null) // nothing left behind
})
