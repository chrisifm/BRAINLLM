import { test } from 'node:test'
import assert from 'node:assert/strict'
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { createServer } from '../server/server.mjs'

test('generic failures keep app language, public dictionary, safe style, base, privacy and explicit branding policy', { timeout: 15000 }, async () => {
  for (const base of ['/', '/site/']) for (const branding of [true, false]) {
    const dist = mkdtempSync(join(tmpdir(), 'brain-fallback-locale-'))
    let server
    try {
      mkdirSync(join(dist, '_server'))
      const dictionary = { 'http.error.title': 'Localized failure', 'http.error.message': '<script>unsafe message</script>', 'http.error.reference': 'Localized reference', 'http.error.home': 'Localized home' }
      writeFileSync(join(dist, '_server/manifest.json'), JSON.stringify({ base, compilerBranding: branding,
        errorFallback: { defaultLocale: 'fr', locales: { fr: dictionary, ar: dictionary } },
        routes: [{ path: '/fail', server: '_server/fail.mjs' }, { path: '/rtl', locale: 'ar', server: '_server/fail.mjs' }, { path: '/missing-locale', locale: 'de', server: '_server/fail.mjs' }], rpc: {}, errors: { 503: '503.html' } }))
      writeFileSync(join(dist, '_server/fail.mjs'), "export function load(){throw Error('private-exception-marker')}")
      writeFileSync(join(dist, '503.html'), '<html lang="it"><head><title>Authored page</title></head><body>Authored priority</body></html>')
      server = createServer({ dist, log() {} })
      await new Promise(resolve => server.listen(0, '127.0.0.1', resolve))
      const origin = `http://127.0.0.1:${server.address().port}`, at = path => origin + base.slice(0, -1) + path
      const response = await fetch(at('/fail?secret=private-query-marker'), { headers: { accept: 'text/html', 'x-request-id': 'known-reference' } })
      const html = await response.text()
      assert.equal(response.status, 500)
      assert.match(response.headers.get('cache-control'), /no-store/)
      assert.ok(response.headers.get('content-security-policy'))
      assert.match(html, /<html lang="fr">/)
      assert.match(html, /500 — Localized failure/)
      assert.match(html, /&lt;script&gt;unsafe message&lt;\/script&gt;/)
      assert.ok(!html.includes('<script>') && !html.includes('private-exception-marker') && !html.includes('private-query-marker'))
      assert.ok(html.includes('Localized reference') && html.includes('known-reference') && html.includes('Localized home'))
      assert.equal(html.includes('<!-- by BRAINLLM'), branding)
      const style = /<link rel="stylesheet" href="([^"]+)"/.exec(html)?.[1]
      assert.equal(style, base + '_brain/error.css')
      const css = await fetch(origin + style)
      assert.equal(css.status, 200)
      assert.match(css.headers.get('content-type'), /text\/css/)
      const source = await css.text()
      assert.match(source, /system-ui/)
      assert.match(source, /forced-colors/)
      assert.equal(source.includes('by BRAINLLM'), branding)
      const head = await fetch(origin + style, { method: 'HEAD' })
      assert.equal(head.status, 200)
      assert.equal(await head.text(), '')
      assert.equal((await fetch(origin + style, { method: 'POST' })).status, 405)
      assert.match(await (await fetch(at('/rtl'))).text(), /<html lang="ar" dir="rtl">/)
      const untranslated = await (await fetch(at('/missing-locale'))).text()
      assert.match(untranslated, /<html lang="de">/)
      assert.match(untranslated, /<h1 lang="en">500 — Internal Server Error/)
      const custom = await fetch(at('/503'))
      assert.equal(custom.status, 503)
      assert.match(await custom.text(), /Authored priority/)
      const json = await fetch(at('/fail'), { headers: { accept: 'application/json' } })
      assert.equal(json.status, 500)
      assert.ok(!JSON.stringify(await json.json()).includes('private-exception-marker'))
    } finally {
      if (server) await new Promise(resolve => { server.close(resolve); server.closeAllConnections() })
      rmSync(dist, { recursive: true, force: true })
    }
  }
})
