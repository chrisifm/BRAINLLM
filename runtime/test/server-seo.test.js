// server.mjs SEO details (INTERNALS §32-§33): `schema="T"` becomes a JSON-LD node in the SSR head (never a
// `<meta name="schema">`).
import { test, after } from 'node:test'
import assert from 'node:assert/strict'
import { copyFileSync, mkdirSync, mkdtempSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = fileURLToPath(new URL('.', import.meta.url))
const dist = mkdtempSync(join(tmpdir(), 'brain-seo-'))
const w = (rel, s) => (mkdirSync(join(dist, rel, '..'), { recursive: true }), writeFileSync(join(dist, rel), s))
mkdirSync(join(dist, '_b'), { recursive: true })
copyFileSync(join(here, '../brain.js'), join(dist, '_b/core.js'))
mkdirSync(join(dist, '_server'), { recursive: true })
copyFileSync(join(here, '../shim/dom.js'), join(dist, '_server/dom.js'))
w('_b/post.js', `import { tpl, insert } from './core.js'
const t = tpl('<main><h1><!></h1></main>')
export default function Post($p) { const r = t(); insert(null, () => $p.name, r.firstChild.firstChild); return r }
`)
w('_server/post.mjs', `export async function load(ctx) { return { name: ctx.params.slug } }
export const seo = p => ({ title: 'Post ' + p.name, description: 'All about ' + p.name, schema: 'Article', 'og:image': '/c.png', lastmod: '2026-10-01' })`)
w('_server/thing.mjs', `export async function load(ctx) { return { name: ctx.params.id } }
export const seo = p => ({ title: 'Thing ' + p.name, schema: 'Product', jsonld: { '@type': 'Product', name: 'Own' } })`)
// the build already wrote the static schema node (prerender seo_head) into the SSR page template
w('_server/pages/post.html', '<!doctype html><html><head><title>Post</title><link rel="canonical" href="https://x.test/posts/:slug"><script type="application/ld+json">{"@context":"https://schema.org","@type":"Article","headline":"Post"}</script></head><body><div id="brain-view"><!--brain:ssr--></div><script type="application/json" id="brain-props"><!--brain:props--></script></body></html>')
w('_server/pages/thing.html', '<!doctype html><html><head><title>Thing</title></head><body><div id="brain-view"><!--brain:ssr--></div><script type="application/json" id="brain-props"><!--brain:props--></script></body></html>')
w('index.html', '<!doctype html><title>Home</title><main><h1>Home</h1></main>')
w('pricing.html', '<!doctype html><title>Pricing</title><main><h1>Pricing</h1></main>')
w('about.html', '<!doctype html><title>About</title><main><h1>About</h1></main>')
w('_server/manifest.json', JSON.stringify({
  v: 1, build: 'b1', core: '/_b/core.js',
  routes: [
    { path: '/posts/:slug', rx: '^/posts/(?<slug>[^/]+)/?$', view: '/_b/post.js', layouts: [], server: '_server/post.mjs', page: '_server/pages/post.html' },
    { path: '/things/:id', rx: '^/things/(?<id>[^/]+)/?$', view: '/_b/post.js', layouts: [], server: '_server/thing.mjs', page: '_server/pages/thing.html' },
  ],
}))

const { createServer } = await import('../server/server.mjs')
const server = createServer({ dist, log() {} })
await new Promise(r => server.listen(0, '127.0.0.1', r))
const base = `http://127.0.0.1:${server.address().port}`
after(() => server.close())
const get = (p, h = {}) => fetch(base + p, { headers: h })
const ld = html => [...html.matchAll(/<script[^>]*type="application\/ld\+json">([\s\S]*?)<\/script>/g)].map(m => JSON.parse(m[1]))

test('SSR: schema="Article" becomes one JSON-LD node with the request values, never a <meta name="schema">', async () => {
  const h = await (await get('/posts/hello')).text()
  assert.doesNotMatch(h, /<meta[^>]*name="schema"/)
  const nodes = ld(h)
  assert.equal(nodes.length, 1, 'the template node from the build is replaced, not duplicated')
  assert.deepEqual(nodes[0], {
    '@context': 'https://schema.org', '@type': 'Article', headline: 'Post hello', description: 'All about hello',
    url: 'https://x.test/posts/hello', image: 'https://x.test/c.png', dateModified: '2026-10-01',
  })
})

test('SSR: an explicit jsonld wins over the schema shortcut', async () => {
  const h = await (await get('/things/a')).text()
  assert.doesNotMatch(h, /name="schema"/)
  assert.deepEqual(ld(h).map(n => n.name), ['Own'])
})
