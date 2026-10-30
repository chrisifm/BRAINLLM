import { test } from 'node:test'
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import vm from 'node:vm'
import { prefetchPage } from '../brain-prefetch-shared.js'
const source = readFileSync(new URL('../brain-prefetch.js', import.meta.url), 'utf8')
function worker(fetch) {
  const messages = [], self = { location: new URL('https://site.test/_b/worker.js'), postMessage: m => messages.push(m) }
  const imported = "import { prefetchPage } from './brain-prefetch-shared.js'"
  assert.ok(source.includes(imported))
  const executable = `const prefetchPage = ${prefetchPage.toString()};\n${source.replace(imported, '')}`
  vm.runInNewContext(executable, { self, fetch, URL, AbortController, setTimeout, clearTimeout, TextDecoder, Uint8Array })
  return { messages, send: data => self.onmessage({ data }) }
}
test('worker fetches HTML with credentials and intent headers and returns immutable assets without executing scripts', async () => {
  let request
  const w = worker(async (url, options) => {
    request = { url, options }
    return new Response('<main>Next</main><script type="module">import V from"/_b/view.js";throw Error("must not execute")</script>', { headers: { 'content-type':'text/html','cache-control':'private,no-store' } })
  })
  await w.send({ id:1, url:'https://site.test/next' })
  assert.equal(request.options.credentials, 'same-origin')
  assert.equal(request.options.redirect, 'error')
  assert.equal(request.options.headers['x-brain-prefetch'], '1')
  assert.deepEqual(Array.from(w.messages[0].assets), ['https://site.test/_b/view.js'])
  assert.equal(w.messages[0].ok, true)
  assert.ok(w.messages[0].text.includes('must not execute'))
})
test('worker ignores other origins and rejects oversized or non-HTML responses', async () => {
  let calls = 0
  const w = worker(async () => { calls++; return new Response('large', { headers: { 'content-type':'text/html','content-length':'999999' } }) })
  await w.send({ id:1, url:'https://outside.test/next' })
  assert.equal(calls,0)
  await w.send({ id:2, url:'https://site.test/next' })
  assert.equal(w.messages[0].ok,false)
  const json = worker(async () => new Response('{}', { headers:{'content-type':'application/json'} }))
  await json.send({ id:3, url:'https://site.test/api/items' })
  assert.equal(json.messages[0].ok,false)
})
test('worker cancellation reaches the owned request and frees its slot', async () => {
  let aborted = false
  const w = worker(async (_,o) => new Promise((resolve,reject) => o.signal.addEventListener('abort', () => { aborted=true;reject(Error('aborted')) })))
  const pending = w.send({ id:1, url:'https://site.test/next' })
  await w.send({ id:1, cancel:true })
  await pending
  assert.equal(aborted,true)
  assert.equal(w.messages[0].ok,false)
})
