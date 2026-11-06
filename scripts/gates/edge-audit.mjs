// Runs the actual selected CLI against an owned HTTP origin. No provider credentials or account calls.
import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { createServer } from 'node:http'
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

const binary = process.env.BRAIN_BIN
if (!binary) throw Error('Set BRAIN_BIN to the exact native binary under test')
const root = mkdtempSync(join(tmpdir(), 'brain-edge-audit-'))
const children = new Set()
const write = (path, text) => { mkdirSync(join(root, path, '..'), { recursive: true }); writeFileSync(join(root, path), text) }
const run = args => new Promise((resolve, reject) => {
  const env = { ...process.env, NO_COLOR: '1' }
  delete env.CLOUDFLARE_API_TOKEN; delete env.CLOUDFLARE_ZONE_ID
  const child = spawn(binary, ['edge', ...args, '--json'], { cwd: root, env, stdio: ['ignore', 'pipe', 'pipe'] })
  children.add(child)
  let stdout = '', stderr = ''
  const deadline = setTimeout(() => child.kill('SIGKILL'), 35000)
  child.stdout.on('data', data => { stdout += data; if (stdout.length > 2 * 1024 * 1024) child.kill('SIGKILL') })
  child.stderr.on('data', data => { stderr = (stderr + data).slice(-8192) })
  child.once('error', reject)
  child.once('exit', (code, signal) => { clearTimeout(deadline); children.delete(child); resolve({ code, signal, stdout, stderr }) })
})
let mode = 'cold'
let calls = []
let foreignCalls = 0
const foreign = createServer((req, res) => { foreignCalls++; res.end('unexpected redirect') })
const server = createServer((req, res) => {
  calls.push({ path: req.url, accept: req.headers.accept, cookie: req.headers.cookie, authorization: req.headers.authorization })
  if (mode === 'stall') return
  const asset = req.url === '/app/scripts/chunk-a1.js'
  const markdown = req.headers.accept === 'text/markdown'
  res.setHeader('Content-Type', asset ? (mode === 'asset-document' ? 'text/html' : 'application/javascript') : markdown ? 'text/markdown; charset=utf-8' : 'text/html; charset=utf-8')
  res.setHeader('Vary', mode === 'missing-vary' ? 'Accept-Encoding' : 'Accept, Accept-Encoding')
  res.setHeader('Cache-Control', asset ? 'public, max-age=31536000, immutable' : mode === 'ttl-mismatch' ? 'public, s-maxage=10' : 'public, s-maxage=60, stale-while-revalidate=60')
  res.setHeader('CF-Cache-Status', mode === 'hit' || mode === 'private-hit' ? 'HIT' : calls.length % 2 ? 'DYNAMIC' : 'MISS')
  if (mode === 'private-hit') { res.setHeader('Cache-Control', 'private, no-store'); res.setHeader('Set-Cookie', 'synthetic=opaque; HttpOnly') }
  if (mode === 'redirect') { res.statusCode = 302; res.setHeader('Location', `http://127.0.0.1:${foreign.address().port}/private`) }
  res.end(asset ? 'export const answer=42' : markdown ? '# Example' : '<h1>Example</h1>')
})
const total = setTimeout(() => { for (const child of children) child.kill('SIGKILL'); server.closeAllConnections(); server.close(); foreign.closeAllConnections(); foreign.close(); process.exitCode = 1 }, 120000)
try {
  write('tokens.cssbrain', ':root { --color-ink: #111111; }')
  write('brain.json', JSON.stringify({ name: 'edge-example', base: '/app/' }))
  write('dist/_server/manifest.json', JSON.stringify({ base: '/app/', build: 'synthetic-build', routes: [{ path: '/kept', cache: 60 }] }))
  write('dist/_headers', '/scripts/chunk-a1.js\n  Cache-Control: public, max-age=31536000, immutable\n  Vary: Accept-Encoding\n')
  write('dist/scripts/chunk-a1.js', 'export const answer=42')
  await new Promise((ok, no) => foreign.once('error', no).listen(0, '127.0.0.1', ok))
  await new Promise((ok, no) => server.once('error', no).listen(0, '127.0.0.1', ok))
  const origin = `http://127.0.0.1:${server.address().port}`
  const plan = await run(['plan']); assert.equal(plan.code, 0, plan.stderr || plan.stdout)
  const planned = JSON.parse(plan.stdout)
  assert.deepEqual(planned.immutableAssets, ['/app/scripts/chunk-a1.js'])
  assert.ok(planned.rules.some(rule => rule.path === '/app/scripts/chunk-a1.js' && rule.immutable))
  assert.ok(!planned.rules.some(rule => rule.path === '/app/_b/*'))
  for (const good of ['cold', 'hit']) {
    mode = good; calls = []
    const result = await run(['audit', '--url', origin]); assert.equal(result.code, 0, result.stderr || result.stdout)
    const audit = JSON.parse(result.stdout); assert.equal(audit.ok, true)
    assert.equal(audit.coverage.privateAuthenticatedPathsTested, false)
    assert.equal(audit.results.length, 2); assert.equal(calls.length, 6)
    assert.ok(calls.every(call => call.cookie === undefined && call.authorization === undefined))
    assert.deepEqual(calls.slice(0, 4).map(call => call.accept), ['text/html', 'text/html', 'text/markdown', 'text/markdown'])
    assert.equal(audit.results[0].cacheObservation, good === 'hit' ? 'shared-cache-observed' : 'not-observed-in-two-requests')
  }
  for (const [bad, error] of [['missing-vary', 'missing-vary-accept'], ['ttl-mismatch', 'declared-page-cache-mismatch'], ['asset-document', 'asset-returned-document'], ['private-hit', 'private-response-shared-cache'], ['redirect', 'unexpected-http-status']]) {
    mode = bad; calls = []
    const result = await run(['check', '--url', origin]); assert.equal(result.code, 2, result.stderr)
    const audit = JSON.parse(result.stdout); assert.equal(audit.ok, false)
    assert.ok(audit.results.some(item => item.errors.includes(error)), error)
    assert.ok(!result.stdout.includes('opaque'), 'cookie values must never be recorded')
    assert.ok(calls.every(call => call.path.startsWith('/app/')), 'redirects must not be followed')
  }
  assert.equal(foreignCalls, 0, 'redirect target must never be contacted')
  mode = 'stall'
  const started = Date.now()
  const stalled = await run(['audit', '--url', origin])
  assert.equal(stalled.code, 2); assert.ok(Date.now() - started < 7000, 'request deadline must be bounded')
  assert.equal(JSON.parse(stalled.stdout).error, 'edge HTTP request failed')
  const before = calls.length
  assert.equal((await run(['audit', '--url', `${origin}/private`])).code, 2)
  assert.equal(calls.length, before)
  assert.equal((await run(['plan', '--provider', 'cloudflare', '--url', 'https://example.com'])).code, 2, 'missing explicit zone must fail before networking')
  console.log('PASS edge audit: actual CLI; exact custom assets; repeated HTML/Markdown; cold status; mismatch/private/redirect controls; no credentials or provider calls')
} finally {
  clearTimeout(total)
  await Promise.all([...children].map(child => new Promise(ok => { if (child.exitCode !== null || child.signalCode !== null) return ok(); child.once('exit', ok); child.kill('SIGKILL') })))
  server.closeAllConnections(); foreign.closeAllConnections()
  await Promise.all([new Promise(ok => server.close(ok)), new Promise(ok => foreign.close(ok))])
  rmSync(root, { recursive: true, force: true })
}
