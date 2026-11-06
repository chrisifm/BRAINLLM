// Inspect one actual response without silently substituting a sitemap audit or following redirects.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { createServer } from 'node:http'
import { spawn } from 'node:child_process'
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

const binary = process.env.BRAIN_BIN ?? new URL('../../target/debug/brain', import.meta.url).pathname
function run(args, cwd) {
  return new Promise((resolve, reject) => {
    const child = spawn(binary, args, { cwd, stdio: ['ignore', 'pipe', 'pipe'] })
    let stdout = '', stderr = ''
    const timer = setTimeout(() => child.kill('SIGKILL'), 45000)
    child.stdout.on('data', chunk => { stdout += chunk; if (stdout.length > 1024 * 1024) child.kill('SIGKILL') })
    child.stderr.on('data', chunk => { stderr = (stderr + chunk).slice(-65536) })
    child.once('error', error => { clearTimeout(timer); reject(error) })
    child.once('exit', (code, signal) => { clearTimeout(timer); resolve({ code, signal, stdout, stderr }) })
  })
}
test('live SEO preview reads exactly one response, reports its metadata and rejects misleading inputs', { timeout: 120000 }, async () => {
  const directory = mkdtempSync(join(tmpdir(), 'brain-live-preview-')), requests = []
  const server = createServer((request, response) => {
    requests.push({ path: request.url, cookie: request.headers.cookie, authorization: request.headers.authorization })
    if (request.url === '/base/redirect') { response.writeHead(302, { location: '/base/page' }); return response.end() }
    if (request.url === '/base/missing') { response.writeHead(404, { 'content-type': 'text/html' }); return response.end('<h1>Missing</h1>') }
    if (request.url === '/base/noindex') { response.writeHead(200, {'content-type':'text/html'}); return response.end('<!doctype html><html lang="en"><head><title>Private preview example</title><meta name="robots" content="noindex"></head><body><main><h1>Private preview example</h1></main></body></html>') }
    if (request.url === '/base/json') { response.writeHead(200, { 'content-type': 'application/json' }); return response.end('{}') }
    if (request.url === '/base/oversized') { response.writeHead(200, { 'content-type': 'text/html', 'content-length': 9 * 1024 * 1024 }); return response.end() }
    if (request.url === '/base/chunked') {
      response.writeHead(200, { 'content-type': 'text/html' })
      for (let i = 0; i < 9; i++) response.write('x'.repeat(1024 * 1024))
      return response.end()
    }
    response.writeHead(200, { 'content-type': 'text/html; charset=utf-8' })
    response.end('<!doctype html><html lang="en"><head><title>Live metadata example</title><meta name="description" content="Metadata returned by the running server, independent of any local build output."><meta property="og:title" content="Live social example"><script type="application/ld+json">{"@context":"https://schema.org","@type":"WebPage","name":"Live example"}</script></head><body><main><h1>Live metadata example</h1><p>A real response.</p></main></body></html>')
  })
  try {
    await new Promise((ok, no) => server.once('error', no).listen(0, '127.0.0.1', ok))
    const origin = `http://127.0.0.1:${server.address().port}/base`
    const result = await run(['seo', 'preview', '/page?example=1', '--url', origin, '--json'], directory)
    assert.equal(result.signal, null, result.stderr)
    assert.ok([0, 1].includes(result.code), result.stdout + result.stderr)
    const preview = JSON.parse(result.stdout)
    assert.equal(preview.httpStatus, 200)
    assert.equal(preview.url, origin + '/page?example=1')
    assert.equal(preview.serp.title, 'Live metadata example')
    assert.equal(preview.social.title, 'Live social example')
    assert.equal(preview.jsonld[0]['@type'], 'WebPage')
    assert.match(preview.scope, /one anonymous HTTP response/)
    assert.equal(requests.length, 1, 'a preview must not fetch a sitemap, assets, robots or other pages')
    assert.equal(requests[0].cookie, undefined)
    assert.equal(requests[0].authorization, undefined)
    for (const [path, code, message] of [['redirect', 1, /HTTP 302/], ['missing', 1, /HTTP 404/], ['json', 2, /HTML response/], ['oversized', 2, /8 MiB/], ['chunked', 2, /8 MiB/]]) {
      const before = requests.length
      const failed = await run(['seo', 'preview', '/' + path, '--url', origin, '--json'], directory)
      assert.equal(failed.code, code, failed.stdout + failed.stderr)
      assert.match(JSON.parse(failed.stdout).error, message)
      assert.equal(requests.length, before + 1, 'redirects remain visible and never follow to another response')
    }
    const verdict = await run(['seo','audit','--url',origin,'--path','/noindex','--delay-ms','0'],directory)
    assert.equal(verdict.code,1,verdict.stdout+verdict.stderr)
    assert.match(verdict.stdout,/B1858/)
    assert.doesNotMatch(verdict.stdout.trim(),/(?:^|\n)ok$/,'a failed live audit must not end with a passing verdict')
    const reordered = await run(['seo', 'preview', '--url', origin, '--json', '/page'], directory)
    assert.equal(JSON.parse(reordered.stdout).url, origin + '/page', 'options before the route must not silently select the home page')
    const audited = await run(['seo', '--url', origin, '--path', '/page', '--delay-ms', '0', '--json'], directory)
    assert.ok([0, 1].includes(audited.code), 'the existing implicit audit command remains compatible: ' + audited.stdout + audited.stderr)
    assert.ok(JSON.parse(audited.stdout).audit, 'implicit audit must remain an audit rather than a preview')
    for (const args of [['//foreign.example/path'], ['/page#fragment'], ['/pa\tge'], ['/pa\u007fge'], ['/page', '--path', '/other'], ['/page', '--sample=2'], ['/page', '--delay-ms', '0']]) {
      const before = requests.length
      const failed = await run(['seo', 'preview', ...args, '--url', origin, '--json'], directory)
      assert.equal(failed.code, 2, failed.stdout + failed.stderr)
      assert.equal(requests.length, before, 'invalid preview inputs must fail before network access')
    }
  } finally {
    server.closeAllConnections()
    if (server.listening) await new Promise(resolve => server.close(resolve))
    rmSync(directory, { recursive: true, force: true })
  }
})
