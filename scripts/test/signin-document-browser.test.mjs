// Real document transitions under the production server's COOP and nonce CSP, without external provider accounts.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { mkdtempSync, mkdirSync, writeFileSync, copyFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { createServer } from '../../runtime/server/server.mjs'
import { launch, openPage } from '../../bench/lib/cdp.mjs'

test('real Chrome renews sign-in document policy in both router paths and preserves same-policy navigation', { timeout: 90000 }, async () => {
  const dist = mkdtempSync(join(tmpdir(), 'brain-signin-browser-'))
  let server, chrome, page
  const documents = []
  try {
    mkdirSync(join(dist, '_server')); mkdirSync(join(dist, '_b'))
    writeFileSync(join(dist, '_server/manifest.json'), JSON.stringify({ routes: [], rpc: {} }))
    for (const file of ['brain-router.js', 'brain-prefetch-shared.js']) copyFileSync(new URL(`../../runtime/${file}`, import.meta.url), join(dist, '_b', file))
    for (const provider of ['plain', 'apple', 'google']) {
      const marker = provider === 'plain' ? '' : `<meta name="brain-${provider}-signin" content="1">`
      writeFileSync(join(dist, `${provider}.html`), `<html><head>${marker}</head><body><main><h1>${provider}</h1></main><script type="module">import { router } from '/_b/brain-router.js'; if (new URL(location.href).searchParams.has('history')) Object.defineProperty(globalThis, 'navigation', { value: undefined }); globalThis.instance = crypto.randomUUID(); globalThis.route = router({prefetch:false}); globalThis.ready = true</script></body></html>`)
    }
    server = createServer({ dist, log() {} })
    server.prependListener('request', req => { if (req.headers['sec-fetch-dest'] === 'document') documents.push(req.url) })
    await new Promise(resolve => server.listen(0, '127.0.0.1', resolve))
    const origin = `http://127.0.0.1:${server.address().port}`
    chrome = await launch()
    page = await openPage(chrome.port, `${origin}/plain`)
    const waitFor = async expression => {
      for (let attempt = 0; attempt < 100; attempt++) {
        try { if (await page.evaluate(expression)) return } catch { /* A full navigation replaces the execution context. */ }
        await new Promise(resolve => setTimeout(resolve, 50))
      }
      assert.fail(`Document did not settle: ${expression}`)
    }
    await waitFor('globalThis.ready === true')
    for (const mode of ['', '?history=1']) {
      await page.evaluate('globalThis.ready = false')
      await page.send('Page.navigate', { url: `${origin}/plain${mode}` })
      await waitFor(`ready && location.pathname === '/plain' && location.search === ${JSON.stringify(mode)}`)
      assert.equal(await page.evaluate('typeof globalThis.navigation?.addEventListener === "function"'), mode === '', 'control: exercise the intended router path')
      let instance = await page.evaluate('instance')
      for (const [target, full] of [['apple', true], ['google', false], ['plain', true]]) {
        const count = documents.length
        // Scheduling lets Runtime.evaluate finish before a full navigation replaces its context.
        await page.evaluate(`setTimeout(() => route.go('/${target}${mode}'), 0); undefined`)
        await waitFor(`ready && location.pathname === '/${target}' && document.querySelector('h1').textContent === '${target}'`)
        const next = await page.evaluate('instance')
        assert.equal(next !== instance, full, `${mode || 'Navigation API'} -> ${target}`)
        assert.equal(documents.length - count, full ? 1 : 0)
        instance = next
      }
    }
  } finally {
    page?.close(); chrome?.close()
    if (server) await new Promise(resolve => { server.close(resolve); server.closeAllConnections() })
    rmSync(dist, { recursive: true, force: true })
  }
})
