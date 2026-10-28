// #285 server.mjs stamps SSR HTML for adoption only when the manifest says so (`adopt: true`).
import { test, after } from 'node:test'
import assert from 'node:assert/strict'
import { copyFileSync, mkdirSync, mkdtempSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = fileURLToPath(new URL('.', import.meta.url))
const make = adopt => {
  const dist = mkdtempSync(join(tmpdir(), 'brain-adopt-ssr-'))
  const w = (rel, s) => (mkdirSync(join(dist, rel, '..'), { recursive: true }), writeFileSync(join(dist, rel), s))
  mkdirSync(join(dist, '_b'), { recursive: true })
  copyFileSync(join(here, '../brain.js'), join(dist, '_b/brain.js'))
  mkdirSync(join(dist, '_server'), { recursive: true })
  copyFileSync(join(here, '../shim/dom.js'), join(dist, '_server/dom.js'))
  copyFileSync(join(here, '../brain-hydrate.js'), join(dist, '_server/brain-hydrate.js'))
  w('_b/v.js', `import { tpl, insert } from '/_b/brain.js'\nexport default function V(p) { const r = tpl('<p>Hi <!></p>')(); insert(r, () => p.name, r.lastChild); return r }\n`)
  w('_server/v.server.mjs', `export function load() { return { name: 'Ada' } }\n`)
  w('_server/pages/p.html', '<!doctype html><html><head><title>t</title></head><body><div id="brain-view"><!--brain:ssr--></div><script type="application/json" id="brain-props"><!--brain:props--></script></body></html>')
  w('_server/manifest.json', JSON.stringify({ v: 1, build: 'b1', core: '/_b/brain.js', rpc: {}, adopt, routes: [{ path: '/', rx: '^/$', view: '/_b/v.js', layouts: [], server: '_server/v.server.mjs', page: '_server/pages/p.html' }] }))
  return dist
}
const { createServer } = await import('../server/server.mjs')
const servers = []
after(() => servers.forEach(s => s.close()))
const get = async dist => {
  const s = createServer({ dist, log() {} })
  servers.push(s)
  await new Promise(r => s.listen(0, '127.0.0.1', r))
  return (await fetch(`http://127.0.0.1:${s.address().port}/`)).text()
}

test('adopt: true → stamped HTML (data-hk, <!--[--> … <!--]-->); without it the HTML stays as before', async () => {
  assert.match(await get(make(true)), /<p data-hk="1">Hi <!--\[-->Ada<!--\]--><\/p>/)
  assert.match(await get(make(false)), /<p>Hi Ada<!----><\/p>/)
})
