import test from 'node:test'
import assert from 'node:assert/strict'
import { createServer } from 'node:http'
import { prefetchPage } from '../brain-prefetch-shared.js'

test('shared transport uses real HTTP, bounds streamed UTF-8 bytes and never executes HTML', { timeout: 10000 }, async () => {
  const requests = []
  const server = createServer((request, response) => {
    requests.push({ path: request.url, intent: request.headers['x-brain-prefetch'] })
    response.setHeader('content-type', 'text/html')
    if (request.url === '/unicode') response.end('\u00e1'.repeat(200000))
    else if (request.url === '/overflow') response.end('a'.repeat(512 * 1024 + 1))
    else if (request.url === '/json') { response.setHeader('content-type', 'application/json'); response.end('{}') }
    else response.end('<main>Next</main><script>globalThis.__prefetchExecuted=true</script><script type="module">import X from"/_b/local.js";import Y from"https://outside.test/_b/remote.js"</script>')
  })
  server.requestTimeout = 2000; server.headersTimeout = 1000; server.keepAliveTimeout = 1
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve))
  const base = `http://127.0.0.1:${server.address().port}`
  const controller = new AbortController(), timeout = setTimeout(() => controller.abort(), 5000)
  try {
    const html = await prefetchPage(base + '/html', controller.signal)
    assert.ok(html?.ok)
    assert.deepEqual(html.assets, [base + '/_b/local.js'])
    assert.equal(globalThis.__prefetchExecuted, undefined)
    const unicode = await prefetchPage(base + '/unicode', controller.signal)
    assert.equal(unicode.bytes, 400000)
    assert.equal(unicode.text.length, 200000)
    assert.equal(await prefetchPage(base + '/overflow', controller.signal), null)
    assert.equal(await prefetchPage(base + '/json', controller.signal), null)
    assert.ok(requests.every(request => request.intent === '1'))
  } finally {
    clearTimeout(timeout); controller.abort(); server.closeAllConnections()
    await new Promise(resolve => server.close(resolve))
  }
})
