// Both generated server imports and client imports execute the new pure helpers, under the actual build policy.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { mkdtempSync, writeFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { createServer } from '../../runtime/server/server.mjs'
import { launch, openPage } from '../../bench/lib/cdp.mjs'
const brain = process.env.BRAIN_BIN ?? new URL('../../target/debug/brain', import.meta.url).pathname

test('compiled list and text helpers execute in SSR and hydrate in Chrome', { timeout: 120000 }, async () => {
  const root = mkdtempSync(join(tmpdir(), 'brain-list-text-'))
  let chrome, page, server
  try {
    const run = (args, cwd) => {
      const result = spawnSync(brain, args, { cwd, encoding: 'utf8', timeout: 40000, maxBuffer: 1024 * 1024 })
      assert.equal(result.status, 0, result.stderr || result.stdout || result.error?.message)
    }
    run(['new', 'app'], root)
    const app = join(root, 'app')
    writeFileSync(join(app, 'views/index.brain'), `<meta seo title="Bounded helpers" keyword="bounded helpers" description="Bounded helpers: integer ranges and native text matching execute in both the server renderer and the browser.">
<script>
  export async function load() { return { values: range(4, 6), folded: text.fold('SERVER') } }
  const values = range(1, 4)
  const normalized = text.fold('CAF\\u00c9')
  const matches = text.fuzzy('compiler', ['compielr', 'unrelated'])
  let count = sig(0)
</script>
<main><h1>Bounded helpers</h1><p id="proof">{normalized}:{values.join(',')}:{matches.join(',')}:{props.values.join(',')}:{props.folded}</p><button on:click={() => count++}>Count {count}</button></main>
`)
    run(['build'], app)
    server = createServer({ dist: join(app, 'dist'), log() {} })
    await new Promise(resolve => server.listen(0, '127.0.0.1', resolve))
    const origin = `http://127.0.0.1:${server.address().port}`
    const expected = 'cafe:1,2,3:compielr:4,5:server'
    const response = await fetch(origin)
    assert.equal(response.status, 200)
    const html = await response.text()
    const proof = html.match(/id="proof"[^>]*>([\s\S]*?)<\/p>/)?.[1]?.replace(/<!--[\s\S]*?-->/g, '')
    assert.equal(proof, expected, 'SSR executes both private helper modules')
    chrome = await launch()
    page = await openPage(chrome.port, origin)
    for (let attempt = 0; attempt < 100; attempt++) {
      try { if (await page.evaluate(`document.readyState === 'complete' && document.querySelector('#proof')?.textContent === ${JSON.stringify(expected)}`)) break } catch {}
      await new Promise(resolve => setTimeout(resolve, 50))
    }
    assert.equal(await page.evaluate("document.querySelector('#proof')?.textContent"), expected)
    await page.evaluate("document.querySelector('button').click()")
    for (let attempt = 0; attempt < 100; attempt++) {
      if (await page.evaluate("document.querySelector('button')?.textContent === 'Count 1'")) break
      await new Promise(resolve => setTimeout(resolve, 30))
    }
    assert.equal(await page.evaluate("document.querySelector('button')?.textContent"), 'Count 1', 'control: the client mounted and installed its event handler')
    assert.equal(await page.evaluate("document.querySelector('#proof')?.textContent"), expected)
  } finally {
    page?.close(); chrome?.close()
    if (server) await new Promise(resolve => { server.close(resolve); server.closeAllConnections() })
    rmSync(root, { recursive: true, force: true })
  }
})
