// Real SSR and streamed heads keep optional facts absent and resolve social images within the app base.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { mkdtempSync, mkdirSync, writeFileSync, copyFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { createServer } from '../server/server.mjs'
const graph = body => [...body.matchAll(/<script[^>]*type="application\/ld\+json">([\s\S]*?)<\/script>/g)].map(m => JSON.parse(m[1]))
test('optional SEO facts and image identities agree in SSR and streaming under root and base', { timeout: 15000 }, async () => {
  for (const base of ['/', '/app/']) {
    const dist = mkdtempSync(join(tmpdir(), 'brain-seo-values-'))
    let server
    try {
      mkdirSync(join(dist, '_b')); mkdirSync(join(dist, '_server'))
      copyFileSync(fileURLToPath(new URL('../brain.js', import.meta.url)), join(dist, '_b/core.js'))
      copyFileSync(fileURLToPath(new URL('../shim/dom.js', import.meta.url)), join(dist, '_server/dom.js'))
      writeFileSync(join(dist, '_b/page.js'), 'import {tpl} from "./core.js";export default function Page(){return tpl("<h1>Metadata example</h1>")()}')
      writeFileSync(join(dist, '_server/load.mjs'), `export async function load(ctx){if(ctx.url.pathname.includes('/live/'))await new Promise(r=>setTimeout(r,40));return {kind:ctx.params.kind}}
export function seo(p){const images={root:'/cover.svg',already:${JSON.stringify(base + 'cover.svg')},relative:'cover.svg',encoded:'/cover%20one.svg',absolute:'https://cdn.example.test/cover.svg',network:'//cdn.example.test/cover.svg'};const blank=p.kind==='empty'?'':p.kind==='blank'?'  ':p.kind==='null'?null:undefined;return {title:p.kind==='zero'?0:'Metadata example',schema:'Article',description:p.kind==='zero'?false:blank,lastmod:blank,'og:image':images[p.kind]??blank,'twitter:image':images[p.kind]??blank}}`)
      const prefix = base.slice(0, -1)
      writeFileSync(join(dist, '_server/page.html'), `<html><head><title>Template</title><link rel="canonical" href="https://example.test${prefix}/posts/:kind/"><script type="application/ld+json">{"@context":"https://schema.org","@graph":[{"@type":"WebPage","@id":"https://example.test${prefix}/posts/:kind/#webpage","url":"https://example.test${prefix}/posts/:kind/","primaryImageOfPage":{"@type":"ImageObject","url":"https://example.test${prefix}/fallback.svg"}}]}</script></head><body><!--brain:ssr--><script type="application/json" id="brain-props"><!--brain:props--></script></body></html>`)
      writeFileSync(join(dist, '_server/plain.mjs'), 'export async function load(){return {kind:"plain"}}')
      const route = { view: '/_b/page.js', layouts: [], server: '_server/load.mjs', page: '_server/page.html' }
      const live = []
      const images = { root: '/cover.svg', already: base + 'cover.svg', relative: 'cover.svg', encoded: '/cover%20one.svg', absolute: 'https://cdn.example.test/cover.svg', network: '//cdn.example.test/cover.svg' }
      for (const kind of ['empty', 'blank', 'null', 'absent', 'zero', ...Object.keys(images)]) {
        const blank = kind === 'empty' ? '' : kind === 'blank' ? '  ' : kind === 'null' ? null : undefined
        const seo = { title: kind === 'zero' ? 0 : 'Metadata example', schema: 'Article', description: kind === 'zero' ? false : blank, lastmod: blank, 'og:image': images[kind] ?? blank, 'twitter:image': images[kind] ?? blank }
        // The compiler permits streaming when SEO is static; the callback itself is not evidence of data dependency.
        writeFileSync(join(dist, `_server/live-${kind}.mjs`), `export async function load(){await new Promise(r=>setTimeout(r,40));return {kind:${JSON.stringify(kind)}}}export function seo(){return ${JSON.stringify(seo)}}`)
        writeFileSync(join(dist, `_server/live-${kind}.html`), `<html><head><title>Template</title><link rel="canonical" href="https://example.test${prefix}/live/${kind}/"><script type="application/ld+json">{"@context":"https://schema.org","@graph":[{"@type":"WebPage","url":"https://example.test${prefix}/live/${kind}/","primaryImageOfPage":{"@type":"ImageObject","url":"https://example.test${prefix}/fallback.svg"}}]}</script></head><body><!--brain:ssr--><script type="application/json" id="brain-props"><!--brain:props--></script></body></html>`)
        live.push({ ...route, path: `/live/${kind}`, rx: `^/live/${kind}/?$`, server: `_server/live-${kind}.mjs`, page: `_server/live-${kind}.html`, stream: true })
      }
      writeFileSync(join(dist, '_server/manifest.json'), JSON.stringify({ base, core: '/_b/core.js', redirects: { slash: 'always' }, routes: [{ ...route, path: '/posts/:kind', rx: '^/posts/(?<kind>[^/]+)/?$', stream: false }, { ...route, path: '/plain/:kind', rx: '^/plain/(?<kind>[^/]+)/?$', server: '_server/plain.mjs', stream: false }, { ...route, path: '/plain-live/:kind', rx: '^/plain-live/(?<kind>[^/]+)/?$', server: '_server/plain.mjs', stream: true }, ...live], rpc: {} }))
      server = createServer({ dist, log() {}, streamHoldMs: 0 })
      await new Promise(resolve => server.listen(0, '127.0.0.1', resolve))
      const origin = `http://127.0.0.1:${server.address().port}${prefix}`
      for (const mode of ['plain', 'plain-live']) {
        const response = await fetch(`${origin}/${mode}/example/`), body = await response.text()
        assert.equal(response.status, 200, body)
        const web = graph(body).flatMap(n => n['@graph'] ?? [n]).find(n => n['@type'] === 'WebPage')
        assert.equal(web.url, `https://example.test${prefix}/${mode}/example/`, body)
        assert.equal(web['@id'], `https://example.test${prefix}/${mode}/example/#webpage`, body)
      }
      for (const mode of ['posts', 'live']) {
        for (const kind of ['empty', 'blank', 'null', 'absent']) {
          const response = await fetch(`${origin}/${mode}/${kind}/`), body = await response.text()
          assert.equal(response.status, 200, body)
          const node = graph(body).find(n => n['@type'] === 'Article')
          assert.ok(node, body)
          for (const key of ['description', 'dateModified', 'image']) assert.equal(Object.hasOwn(node, key), false, body)
          assert.doesNotMatch(body, /<meta[^>]*content="\s*"/)
        }
        const zero = await (await fetch(`${origin}/${mode}/zero/`)).text()
        const node = graph(zero).find(n => n['@type'] === 'Article')
        assert.equal(node.headline, '0')
        assert.equal(node.description, 'false')
        for (const [kind, expected] of [['root', `https://example.test${prefix}/cover.svg`], ['already', `https://example.test${prefix}/cover.svg`], ['relative', `https://example.test${prefix}/${mode}/relative/cover.svg`], ['encoded', `https://example.test${prefix}/cover%20one.svg`], ['absolute', 'https://cdn.example.test/cover.svg'], ['network', 'https://cdn.example.test/cover.svg']]) {
          const response = await fetch(`${origin}/${mode}/${kind}/`), body = await response.text()
          assert.equal(response.status, 200, body)
          assert.equal(graph(body).find(n => n['@type'] === 'Article').image, expected, body)
          const web = graph(body).flatMap(n => n['@graph'] ?? [n]).find(n => n['@type'] === 'WebPage')
          assert.equal(web.url, `https://example.test${prefix}/${mode}/${kind}/`, body)
          if (web['@id']) assert.equal(web['@id'], `https://example.test${prefix}/${mode}/${kind}/#webpage`, body)
          assert.equal(web.primaryImageOfPage.url, expected, body)
          for (const [attr, key] of [['property', 'og:image'], ['name', 'twitter:image']]) assert.ok(body.includes(`<meta ${attr}="${key}" content="${expected}">`), body)
        }
      }
    } finally {
      if (server) await new Promise(resolve => { server.close(resolve); server.closeAllConnections() })
      rmSync(dist, { recursive: true, force: true })
    }
  }
})
