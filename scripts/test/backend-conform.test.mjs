import test from 'node:test'
import assert from 'node:assert/strict'
import { createServer } from 'node:http'
import { spawn } from 'node:child_process'
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
const binary = process.env.BRAIN_BIN ?? new URL('../../target/debug/brain', import.meta.url).pathname
function run(args, cwd) {
  return new Promise((resolve, reject) => {
    const child = spawn(binary, ['backend', 'conform', ...args, '--json'], { cwd, stdio: ['ignore', 'pipe', 'pipe'] })
    let stdout = '', stderr = ''
    const timer = setTimeout(() => child.kill('SIGKILL'), 45000)
    child.stdout.on('data', chunk => { stdout += chunk; if (stdout.length > 1024 * 1024) child.kill('SIGKILL') })
    child.stderr.on('data', chunk => { stderr = (stderr + chunk).slice(-65536) })
    child.once('error', error => { clearTimeout(timer); reject(error) })
    child.once('exit', (code, signal) => { clearTimeout(timer); resolve({ code, signal, stdout, stderr }) })
  })
}
test('backend conform observes bounded anonymous JSON GET/HEAD without sending writes or following redirects', { timeout: 90000 }, async () => {
  const root = mkdtempSync(join(tmpdir(), 'brain-conform-')), requests = []
  const server = createServer((request, response) => {
    requests.push({ method: request.method, path: request.url, accept: request.headers.accept, cookie: request.headers.cookie, authorization: request.headers.authorization })
    let status = 200, mime = 'application/json', body = '{"ok":true}'
    if (request.url === '/error') { status = 403; body = '{"error":{"code":"SYNTHETIC_PRIVATE_RESPONSE_VALUE","message":"synthetic-private-response-value"}}' }
    if (request.url === '/html') { status = 500; mime = 'text/html'; body = '<h1>synthetic-private-response-value</h1>' }
    if (request.url === '/bad-json') { status = 401; body = '{unreadable-private-response-value' }
    if (request.url === '/bad-envelope') { status = 400; body = '{"error":"synthetic-private-response-value"}' }
    if (request.url === '/redirect') { response.writeHead(302, { location: '/valid' }); return response.end() }
    if (request.url === '/head-mismatch') status = request.method === 'HEAD' ? 200 : 404
    if (request.url === '/huge') { response.writeHead(200, { 'content-type': mime, 'content-length': 1024 * 1024 }); return response.end() }
    if (request.url === '/chunks') { response.writeHead(200, { 'content-type': mime }); response.end('x'.repeat(300 * 1024)); return }
    response.writeHead(status, { 'content-type': mime, 'content-length': Buffer.byteLength(body) })
    response.end(body)
  })
  try {
    await new Promise((resolve, reject) => server.once('error', reject).listen(0, '127.0.0.1', resolve))
    const origin = `http://127.0.0.1:${server.address().port}`
    const valid = await run(['--path', '/valid', '--url', origin, '--path', '/error'], root)
    assert.equal(valid.signal, null)
    assert.equal(valid.code, 0, valid.stdout + valid.stderr)
    const report = JSON.parse(valid.stdout)
    assert.equal(report.ok, true)
    assert.equal(report.credentialsSent, false)
    assert.equal(report.redirectsFollowed, false)
    assert.match(report.scope, /not write-path/)
    assert.equal(report.observations.length, 4)
    assert.equal(report.observations.find(row => row.method === 'GET' && row.path === '/error').hasStableErrorCode, true)
    assert.ok(report.observations.every(row => row.findings.length === 0))
    assert.doesNotMatch(valid.stdout, /synthetic-private-response-value|SYNTHETIC_PRIVATE_RESPONSE_VALUE/)
    assert.ok(requests.every(request => ['GET', 'HEAD'].includes(request.method) && request.accept === 'application/json' && request.cookie === undefined && request.authorization === undefined))
    for (const [path, finding] of [['/html', 'explicit-json-request-received-non-json'], ['/bad-json', 'unreadable-json-body'], ['/bad-envelope', 'json-error-missing-stable-code-or-message'], ['/head-mismatch', 'head-get-status-mismatch'], ['/redirect', 'redirect-not-followed']]) {
      const before = requests.length
      const result = await run(['--url', origin, '--path', path], root)
      assert.equal(result.code, 1, result.stdout + result.stderr)
      assert.ok(JSON.parse(result.stdout).observations.some(row => row.findings.includes(finding)), path)
      assert.equal(requests.length - before, 2, 'two observations only, even for redirects')
      assert.doesNotMatch(result.stdout, /synthetic-private-response-value|unreadable-private-response-value/)
    }
    for (const path of ['/huge', '/chunks']) {
      const before = requests.length
      const result = await run(['--url', origin, '--path', path], root)
      assert.equal(result.code, 2, result.stdout + result.stderr)
      assert.match(JSON.parse(result.stdout).error, /256 KiB/)
      assert.equal(requests.length - before, 1, 'stop after the unverified bounded GET')
    }
    for (const args of [['--url', origin + '?private=value'], ['--url', origin, '--path', '//foreign.test'], ['--url', origin, '--path', '/api/a?private=value'], ['--url', origin, '--path', '/../private'], ['--url', origin, '--path', '/valid', '--path', '/valid'], ['--url', origin, '--url', origin], ['--url', origin, 'stray']]) {
      const before = requests.length
      const result = await run(args, root)
      assert.equal(result.code, 2, result.stdout + result.stderr)
      assert.equal(requests.length, before, 'invalid input must fail before any HTTP request')
    }
  } finally {
    server.closeAllConnections()
    if (server.listening) await new Promise(resolve => server.close(resolve))
    rmSync(root, { recursive: true, force: true })
  }
})
