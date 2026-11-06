import { test } from 'node:test'
import assert from 'node:assert/strict'
import { openPage, readDebuggingAddress, waitDebuggingAddress } from '../../bench/lib/cdp.mjs'
import { mkdtempSync, writeFileSync, readFileSync, chmodSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

const uuidPath = '/devtools/browser/00000000-0000-4000-8000-000000000001'
function fakeChrome(root, addressPath) {
  const quote = value => "'" + value.replaceAll("'", "'\\''") + "'"
  const script = join(root, 'chrome.mjs'), executable = join(root, 'chrome'), expected = join(root, 'expected-port')
  writeFileSync(script, `import {writeFileSync} from 'node:fs'; import {join} from 'node:path'; import {createServer} from 'node:http';
const profile=process.argv.find(arg=>arg.startsWith('--user-data-dir=')).slice('--user-data-dir='.length);
const file=join(profile,'DevToolsActivePort'),addressPath=${JSON.stringify(addressPath)};
writeFileSync(file,'');
const server=createServer((req,res)=>{if(req.url!='/json/version'){res.writeHead(404).end();return}res.setHeader('Connection','close');res.end(JSON.stringify({webSocketDebuggerUrl:'ws://127.0.0.1:'+server.address().port+addressPath}))});
server.listen(0,'127.0.0.1',()=>{const port=server.address().port;writeFileSync(${JSON.stringify(expected)},String(port));
 setTimeout(()=>writeFileSync(file,String(port).slice(0,3)),70);
 setTimeout(()=>writeFileSync(file,String(port)+'\\n/devtools/browser'),140);
 setTimeout(()=>writeFileSync(file,String(port)+'\\n'+addressPath),180);
});
process.on('SIGTERM',()=>process.exit(0));`)
  writeFileSync(executable, `#!/bin/sh\nexec ${quote(process.execPath)} ${quote(script)} "$@"\n`)
  chmodSync(executable, 0o700)
  return { executable, expected }
}

test('Chrome address parsing rejects partial, malformed and oversized files', () => {
  const root = mkdtempSync(join(tmpdir(), 'brain-cdp-address-')), file = join(root, 'DevToolsActivePort')
  try {
    assert.equal(readDebuggingAddress(file), null)
    for (const text of ['', '123', '123\n', `123\n${uuidPath.slice(0, -1)}`, `0\n${uuidPath}`, `65536\n${uuidPath}`,
      `-1\n${uuidPath}`, `1e2\n${uuidPath}`, `1\n${uuidPath}?extra=value`, 'x'.repeat(4096)]) {
      writeFileSync(file, text)
      assert.equal(readDebuggingAddress(file), null)
    }
    for (const path of [uuidPath, '/devtools/browser']) for (const port of [1, 12345, 65535]) for (const separator of ['\n', '\r\n']) {
      writeFileSync(file, `${port}${separator}${path}${separator}`)
      assert.deepEqual(readDebuggingAddress(file), { port, path })
    }
  } finally { rmSync(root, { recursive: true, force: true }) }
})

test('Chrome port file readiness waits for complete contents and live discovery, not just file creation', { skip: process.platform === 'win32' }, async () => {
  for (const path of [uuidPath, '/devtools/browser']) {
    const root = mkdtempSync(join(tmpdir(), 'brain-cdp-port-')), fixture = fakeChrome(root, path)
    const previous = process.env.BRAIN_CHROME
    const counts = ['exit','SIGINT','SIGTERM'].map(signal => process.listenerCount(signal))
    let browser
    try {
      process.env.BRAIN_CHROME = fixture.executable
      const { launch } = await import(`../../bench/lib/cdp.mjs?partial-port-fixture-${path.endsWith('browser') ? 'bare' : 'uuid'}`)
      browser = await launch()
      assert.equal(browser.port, Number(readFileSync(fixture.expected, 'utf8')), 'a partial numeric prefix is not a complete port')
    } finally {
      browser?.close()
      if (previous === undefined) delete process.env.BRAIN_CHROME
      else process.env.BRAIN_CHROME = previous
      rmSync(root, { recursive: true, force: true })
    }
    assert.deepEqual(['exit','SIGINT','SIGTERM'].map(signal => process.listenerCount(signal)), counts)
  }
})

test('cache browser uses discovery to resolve an ambiguous bare-path prefix before opening its socket', { skip: process.platform === 'win32' }, async () => {
  for (const path of [uuidPath, '/devtools/browser']) {
    const root = mkdtempSync(join(tmpdir(), 'brain-cache-cdp-port-')), fixture = fakeChrome(root, path)
    const previous = process.env.BRAIN_CHROME, savedSocket = globalThis.WebSocket, urls = []
    let browser
    try {
      process.env.BRAIN_CHROME = fixture.executable
      globalThis.WebSocket = class {
        constructor(url) { urls.push(url); queueMicrotask(() => this.onopen?.()) }
        send(raw) { const request = JSON.parse(raw); queueMicrotask(() => this.onmessage?.({ data: JSON.stringify({ id: request.id, result: {} }) })) }
        close() {}
      }
      const { launchBrowser } = await import(`../../bench/cache/lib/browser.mjs?partial-port-fixture-${path.endsWith('browser') ? 'bare' : 'uuid'}`)
      browser = await launchBrowser(join(root, 'profile'))
      assert.deepEqual(urls, [`ws://127.0.0.1:${readFileSync(fixture.expected, 'utf8')}${path}`])
    } finally {
      await browser?.close()
      globalThis.WebSocket = savedSocket
      if (previous === undefined) delete process.env.BRAIN_CHROME
      else process.env.BRAIN_CHROME = previous
      rmSync(root, { recursive: true, force: true })
    }
  }
})

test('debugging discovery remains bounded and rejects a different advertised endpoint', async () => {
  const root = mkdtempSync(join(tmpdir(), 'brain-cdp-discovery-')), file = join(root, 'DevToolsActivePort')
  const saved = globalThis.fetch
  try {
    writeFileSync(file, `43210\n${uuidPath}`)
    for (const timeoutMs of [0, -1, 1.5, Infinity, 30001]) await assert.rejects(waitDebuggingAddress(file, { timeoutMs }), /1\.\.30000/)
    for (const body of [JSON.stringify({webSocketDebuggerUrl:`ws://example.test:43210${uuidPath}`}), 'x'.repeat(20000)]) {
      globalThis.fetch = async () => new Response(body)
      await assert.rejects(waitDebuggingAddress(file, {timeoutMs:60}), /startup deadline/)
    }
  } finally { globalThis.fetch = saved; rmSync(root, {recursive:true,force:true}) }
})

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
