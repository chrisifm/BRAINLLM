// A media AVIF rendition decodes in real Chrome, and <picture> falls back when the AVIF source is refused.
// BRAIN_AVIF_OUT=/tmp/x.avif cargo test -p brain-backend --lib avif_preset, then BRAIN_AVIF_FILE=/tmp/x.avif node --test this file.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { createServer } from 'node:http'
import { readFileSync } from 'node:fs'
import { launch, openPage } from '../../bench/lib/cdp.mjs'

const file = process.env.BRAIN_AVIF_FILE
const png = Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==', 'base64')
test('AVIF decodes and <picture> falls back', { skip: !file, timeout: 60000 }, async () => {
  const types = { '/a.avif': ['image/avif', readFileSync(file)], '/f.png': ['image/png', png] }
  const server = createServer((req, res) => {
    const hit = types[req.url]
    res.setHeader('Content-Type', hit ? hit[0] : 'text/html')
    res.end(hit ? hit[1] : '<!doctype html><title>AVIF</title>')
  })
  let chrome, page
  try {
    await new Promise(resolve => server.listen(0, '127.0.0.1', resolve))
    chrome = await launch()
    page = await openPage(chrome.port, `http://127.0.0.1:${server.address().port}/`)
    const load = type => `new Promise(resolve => {
      document.body.innerHTML = '<picture><source srcset="/a.avif 48w" type="${type}"><img src="/f.png" alt="sample"></picture>'
      const img = document.querySelector('img')
      img.onload = () => resolve({ src: new URL(img.currentSrc).pathname, width: img.naturalWidth })
      img.onerror = () => resolve({ error: true })
    })`
    const avif = await page.evaluate(load('image/avif'))
    assert.equal(avif.src, '/a.avif', JSON.stringify(avif))
    assert.ok(avif.width > 0)
    assert.equal((await page.evaluate(load('image/x-unsupported'))).src, '/f.png')
  } finally {
    page?.close(); chrome?.close()
    await new Promise(resolve => server.close(resolve))
  }
})
