import { test } from 'node:test'
import assert from 'node:assert/strict'
import { openPage } from '../../bench/lib/cdp.mjs'

test('CDP setup creates one blank target, bounds pending commands and rejects closed work', async () => {
  const saved = { fetch: globalThis.fetch, WebSocket: globalThis.WebSocket, setTimeout: globalThis.setTimeout }
  const requests = [], commands = [], scheduled = []
  class Socket {
    static OPEN = 1
    readyState = 1
    constructor() { queueMicrotask(() => this.onopen?.()) }
    send(raw) {
      const request = JSON.parse(raw); commands.push(request.method)
      if (request.method === 'Hang.command') return
      queueMicrotask(() => this.onmessage?.({data:JSON.stringify({id:request.id,result:{result:{value:'ready'}}})}))
    }
    close() { this.readyState = 3; this.onclose?.() }
  }
  let page
  try {
    globalThis.fetch = async (url, options) => {
      requests.push({url,method:options?.method || 'GET'})
      return {ok:true,json:async () => ({webSocketDebuggerUrl:'ws://example.test/owned-page'})}
    }
    globalThis.WebSocket = Socket
    // Advance only the command watchdog; protocol replies still arrive before the timer in microtasks.
    globalThis.setTimeout = (callback, delay, ...args) => {
      scheduled.push(delay)
      return saved.setTimeout(callback, delay === 15000 ? 5 : delay, ...args)
    }
    page = await openPage(12345, 'https://example.test/')
    assert.deepEqual(requests.map(request => request.method), ['GET', 'PUT'])
    assert.ok(requests[1].url.endsWith('/json/new?about:blank'))
    assert.deepEqual(commands.slice(0,3), ['Page.enable','Runtime.enable','Page.navigate'])
    await assert.rejects(page.send('Hang.command'), /command deadline: Hang.command/)
    assert.equal(await page.evaluate('ready'), 'ready')
    assert.equal(await page.evaluate('ready', 40000), 'ready')
    assert.equal(scheduled.at(-1), 40000, 'long operations retain their explicit consumer deadline')
    await assert.rejects(page.send('Hang.command', {}, 2), /command deadline/)
    for (const invalid of [0, -1, 0.5, Infinity, 120001]) await assert.rejects(page.evaluate('ready', invalid), /1..120000/)
    const all = await Promise.allSettled(Array.from({length:65}, () => page.send('Hang.command')))
    assert.equal(all.filter(result => /busy/.test(result.reason?.message)).length, 1)
    assert.equal(all.filter(result => /deadline/.test(result.reason?.message)).length, 64)
    const pending = page.send('Hang.command')
    page.close()
    await assert.rejects(pending, /connection closed/)
    await assert.rejects(page.send('Page.enable'), /unavailable/)
  } finally {
    page?.close()
    Object.assign(globalThis, saved)
  }
})

test('a missing Chrome executable fails cleanly without retaining process signal handlers', async () => {
  const previous = process.env.BRAIN_CHROME
  const counts = ['exit','SIGINT','SIGTERM'].map(signal => process.listenerCount(signal))
  try {
    process.env.BRAIN_CHROME = '/nonexistent/brain-test-chrome'
    const { launch } = await import('../../bench/lib/cdp.mjs?missing-chrome')
    await assert.rejects(launch(), /Chrome did not start: ENOENT/)
    assert.deepEqual(['exit','SIGINT','SIGTERM'].map(signal => process.listenerCount(signal)), counts)
  } finally {
    if (previous === undefined) delete process.env.BRAIN_CHROME
    else process.env.BRAIN_CHROME = previous
  }
})
