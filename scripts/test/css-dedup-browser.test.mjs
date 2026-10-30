// Real computed-style equivalence for fixture CSS emitted by the Rust optimizer. No provider or package install.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { readFile, readdir, stat } from 'node:fs/promises'
import { join } from 'node:path'
import { launch, openPage } from '../../bench/lib/cdp.mjs'

const directory = process.env.BRAIN_CSS_FIXTURE_DIR
const deadline = (promise, ms, label) => {
  let timer
  return Promise.race([promise, new Promise((_, reject) => { timer = setTimeout(() => reject(Error(`${label} deadline`)), ms) })]).finally(() => clearTimeout(timer))
}
const read = async path => {
  assert.ok((await stat(path)).size <= 1024 * 1024, 'fixture exceeds 1 MiB')
  return readFile(path, 'utf8')
}
test('generated CSS dedup keeps real Chrome computed styles and component boundaries', { skip: !directory, timeout: 60000 }, async () => {
  let chrome, page
  try {
    chrome = await deadline(launch(), 10000, 'owned Chrome startup')
    page = await deadline(openPage(chrome.port, 'about:blank'), 10000, 'CDP page')
    assert.equal(await deadline(page.evaluate("typeof CSSScopeRule === 'function'"), 5000, 'scope feature'), true, 'browser must implement @scope; do not accept a vacuous comparison')
    const version = await deadline(page.send('Browser.getVersion'), 5000, 'browser version')
    console.log(`CSS proof browser: ${version.product}`)
    const names = (await readdir(directory)).filter(name => /^[a-z-]+$/.test(name)).sort()
    assert.ok(names.length >= 6 && names.length <= 16)
    const snapshot = async (css, body) => deadline(page.evaluate(`(() => {
      document.head.innerHTML = '';
      const style = document.createElement('style'); style.textContent = ${JSON.stringify(css)}; document.head.append(style);
      document.body.innerHTML = ${JSON.stringify(body)};
      return [...document.querySelectorAll('[id]')].map(element => {
        const style = getComputedStyle(element);
        return { id: element.id, color: style.color, background: style.backgroundColor,
          padding: style.paddingLeft, margin: style.marginLeft, fontWeight: style.fontWeight,
          tone: style.getPropertyValue('--tone'), gap: style.getPropertyValue('--gap'), tokens: style.getPropertyValue('--tokens') };
      });
    })()`), 5000, 'computed style snapshot')
    for (const name of names) {
      const base = join(directory, name)
      const [original, optimized, body, optimizedBody] = await Promise.all([
        read(join(base, 'original.css')), read(join(base, 'optimized.css')),
        read(join(base, 'original.html')), read(join(base, 'optimized.html')),
      ])
      const before = await snapshot(original, body), after = await snapshot(optimized, optimizedBody)
      assert.deepEqual(after, before, name)
      if (name === 'siblings' || name === 'nested') {
        assert.equal(before.find(element => element.id === 'a').padding, '1px', 'positive style must actually apply')
        assert.equal(before.find(element => element.id === 'b').padding, '1px')
        assert.equal(before.find(element => element.id === 'outside').padding, '0px', 'foreign root must stay outside')
      }
      if (name === 'minimal-difference') {
        assert.equal(before.find(element => element.id === 'a').padding, '1px')
        assert.equal(before.find(element => element.id === 'b').padding, '2px')
      }
      if (name === 'class-collision') assert.equal(after.find(element => element.id === 'external').color, 'rgb(0, 0, 255)')
    }
  } finally {
    page?.close()
    chrome?.close()
  }
})
