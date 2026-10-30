// A real compiled Brain view -> compiler worker bridge -> official SDK -> preserved OPFS.
import assert from 'node:assert/strict'
import { createServer } from 'node:http'
import { readFile } from 'node:fs/promises'
import { resolve, sep, extname } from 'node:path'
import { launch, openPage } from '../../../bench/lib/cdp.mjs'
const root = resolve(process.argv[2] || '')
const headers = await readFile(resolve(root, '_headers'), 'utf8')
const policy = headers.match(/^\s+Content-Security-Policy:\s+(.+)$/m)?.[1]
assert.ok(policy?.includes("'wasm-unsafe-eval'"))
assert.ok(!policy.includes("'unsafe-eval'"))
const responseHeaders = path => {
  const output = {}; let active = false
  for (const line of headers.split('\n')) {
    if (!line.trim() || line.trimStart().startsWith('#')) continue
    if (!/^\s/.test(line)) {
      const expression = line.trim().split('*').map(part => part.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')).join('.*')
      active = new RegExp('^' + expression + '$').test(path)
    } else if (active) {
      const at = line.indexOf(':')
      if (at > 0) output[line.slice(0, at).trim()] = line.slice(at + 1).trim()
    }
  }
  return output
}
const documentHeaders = responseHeaders('/')
assert.equal(documentHeaders['Content-Security-Policy'], policy)
assert.equal(documentHeaders['Cross-Origin-Opener-Policy'], 'same-origin')
const failures = [], frames = []
let pending = 0, chrome, page
const server = createServer(async (req, res) => {
  const path = new URL(req.url, 'http://localhost').pathname
  if (++pending > 64) { pending--; res.writeHead(503).end(); return }
  try {
    const file = resolve(root, '.' + (path === '/' ? '/index.html' : decodeURIComponent(path)))
    if (!file.startsWith(root + sep)) { res.writeHead(403).end(); return }
    const body = await readFile(file)
    const extension = extname(file)
    res.setHeader('Content-Type', extension === '.wasm' ? 'application/wasm' : ['.js','.mjs','.jsbrain'].includes(extension) ? 'text/javascript' : extension === '.css' ? 'text/css' : 'text/html')
    for (const [key, value] of Object.entries(responseHeaders(path))) res.setHeader(key, value)
    res.end(body)
  } catch {
    if (failures.length < 16) failures.push({ path, status: 404 })
    res.writeHead(404).end('Missing compiled fixture artifact')
  } finally { pending-- }
})
server.requestTimeout = 15000
server.headersTimeout = 10000
server.keepAliveTimeout = 1000
const deadline = async (promise, ms) => {
  let timer
  try { return await Promise.race([promise, new Promise((_, reject) => { timer = setTimeout(() => reject(Error('Compiled SQLite browser proof deadline')), ms) })]) }
  finally { clearTimeout(timer) }
}
try {
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve))
  chrome = await deadline(launch(), 8000)
  page = await deadline(openPage(chrome.port, `http://127.0.0.1:${server.address().port}/`), 8000)
  page.on(event => {
    if (event.method === 'Runtime.exceptionThrown' && frames.length < 16) {
      const detail = event.params.exceptionDetails
      frames.push({ file: String(detail.url || '').split(/[?#]/)[0], line: detail.lineNumber + 1, column: detail.columnNumber + 1 })
    }
  })
  const result = await deadline(page.evaluate(`(async()=>{
    const pause=()=>new Promise(resolve=>setTimeout(resolve,100));
    let button;
    for(let i=0;i<100;i++){button=document.querySelector('button');if(button)break;await pause()}
    if(!button)throw Error('Compiled view did not mount');
    button.click();
    for(let i=0;i<250;i++){if(button.textContent==='1')return {compiledWorker:true,value:1,isolated:crossOriginIsolated};await pause()}
    throw Error('Compiled SQLite worker did not produce its SQL row');
  })()`), 40000)
  assert.equal(result.compiledWorker, true); assert.equal(result.value, 1)
  result.coop = documentHeaders['Cross-Origin-Opener-Policy'] || null
  result.coep = documentHeaders['Cross-Origin-Embedder-Policy'] || null
  assert.deepEqual(failures.filter(item => !item.path.endsWith('/favicon.ico')), [])
  process.stdout.write(JSON.stringify(result) + '\n')
} catch (error) {
  process.stderr.write(JSON.stringify({ phase: 'compiled-sqlite-browser', failures, frames }) + '\n')
  throw error
} finally {
  page?.close(); chrome?.close()
  server.closeAllConnections()
  await new Promise(resolve => server.close(resolve))
}
