import { test } from 'node:test'
import assert from 'node:assert/strict'
import { readFileSync, mkdtempSync, writeFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { pathToFileURL } from 'node:url'
import { spawn } from 'node:child_process'
import { createServer } from 'node:http'
import { createInterface } from 'node:readline'
const rust = readFileSync(new URL('../../crates/brain-serve/src/load.rs', import.meta.url), 'utf8')
const runner = rust.split('const RUNNER: &str = r#"')[1]?.split('"#;')[0]
assert.ok(runner, 'test executes the actual embedded sidecar runner')
const runtime = new URL('../../runtime/server/server.mjs', import.meta.url).href
const once = (emitter, event) => new Promise(resolve => emitter.once(event, resolve))

test('real sidecar uses native authority once across guard/load, awaits logout and refuses legacy fallback', { timeout: 15000 }, async () => {
  const dir = mkdtempSync(join(process.env.TMPDIR || tmpdir(), 'brain-sidecar-native-'))
  let anonymous = false, me = 0, logout = 0, child, lines
  const user = { id: '00000000-0000-4000-8000-000000000001', email: 'synthetic@example.test', role: 'candidate', verified: true }
  const backend = createServer((req, res) => {
    assert.equal(req.headers['x-forwarded-host'], undefined)
    assert.equal(req.headers['x-forwarded-for'], undefined)
    assert.ok(!String(req.headers.cookie).includes('__Host-session'))
    assert.ok(!String(req.headers.cookie).includes('unrelated'))
    if (req.url === '/_brain/auth/me') {
      me++
      res.writeHead(anonymous ? 401 : 200, { 'content-type': 'application/json' })
      res.end(JSON.stringify(anonymous ? { error: 'Sign in required' } : { user }))
    } else if (req.url === '/_brain/auth/logout') {
      logout++
      assert.equal(req.method, 'POST')
      assert.equal(req.headers.origin, 'http://127.0.0.1:5182')
      res.writeHead(200, { 'content-type': 'application/json', 'set-cookie': ['__Host-brain=; Path=/; Secure; HttpOnly; Max-Age=0'] })
      setTimeout(() => res.end('{"ok":true}'), 20)
    } else { res.writeHead(404); res.end() }
  })
  let next = 0, pending = null, stderr = 0, stdout = 0
  try {
    backend.listen(0, '127.0.0.1'); await once(backend, 'listening')
    writeFileSync(join(dir, 'view.mjs'), 'export function load(ctx) { return { user:ctx.user, isURL:ctx.url instanceof URL } }; export function logout(ctx) { ctx.session.clear(); return {ok:true} }; export function forge(ctx) { ctx.session.set({role:"admin"}); return {ok:true} }')
    const file = pathToFileURL(join(dir, 'view.mjs')).href
    child = spawn(process.execPath, ['--input-type=module', '-e', runner], { env: { ...process.env, BRAIN_BACKEND_URL: 'http://127.0.0.1:' + backend.address().port }, stdio: ['pipe', 'pipe', 'pipe'] })
    child.stderr.on('data', chunk => { stderr += chunk.length; if (stderr > 65536) child.kill() })
    child.stdout.on('data', chunk => { stdout += chunk.length; if (stdout > 1048576) child.kill() })
    lines = createInterface({ input: child.stdout })
    lines.on('line', line => {
      if (line.length > 65536) return child.kill()
      const split = line.indexOf(' '), split2 = line.indexOf(' ', split + 1)
      if (!pending || line.slice(split + 1, split2) !== String(pending.id)) return
      const current = pending; pending = null; clearTimeout(current.timer)
      try { current.resolve({ tag: line.slice(0, split), value: JSON.parse(line.slice(split2 + 1)) }) } catch (error) { current.reject(error) }
    })
    const call = args => new Promise((resolve, reject) => {
      assert.equal(pending, null)
      const id = ++next, timer = setTimeout(() => { pending = null; reject(Error('owned sidecar request exceeded four seconds')) }, 4000)
      pending = { id, resolve, reject, timer }
      child.stdin.write(JSON.stringify({ id, nativeRequired: true, ...args }) + '\n')
    })
    const ctx = { url: '/', rid: '00000000-0000-4000-8000-000000000002', req: { method: 'GET', headers: { host: '127.0.0.1:5182', origin: 'http://127.0.0.1:5182', cookie: '__Host-brain=' + 'a'.repeat(64) + '; __Host-session=untrusted; unrelated=private-value', 'x-forwarded-host': 'spoofed.example' } }, cookies: { '__Host-session': 'untrusted' }, user: { role: 'admin' } }
    const chain = await call({ chain: runtime, guards: [], middleware: [], ctx, secret: '' })
    assert.equal(chain.tag, 'CHN'); assert.equal(chain.value.kind, 'ok')
    assert.deepEqual(JSON.parse(chain.value.user), user)
    assert.equal(me, 1)
    const loaded = await call({ file, fn: 'load', contextServer: runtime, preauthenticated: true, ctx: { ...ctx, user: JSON.parse(chain.value.user) }, secret: '' })
    assert.equal(loaded.tag, 'OK'); assert.deepEqual(loaded.value.user, user); assert.equal(loaded.value.isURL, true)
    assert.equal(me, 1, 'private guard result is reused only inside this request, without a second native /me')
    const signedOut = await call({ file, fn: 'logout', rpc: runtime, raw: '[]', key: 'synthetic', secret: '', ctx: { ...ctx, req: { ...ctx.req, method: 'POST' } } })
    assert.equal(signedOut.tag, 'RES'); assert.equal(signedOut.value.status, 200)
    assert.equal(logout, 1); assert.ok(signedOut.value.headers.includes('__Host-brain=')); assert.ok(signedOut.value.headers.includes('clear-site-data'))
    anonymous = true
    const anon = await call({ chain: runtime, guards: [], middleware: [], ctx, secret: '' })
    assert.equal(anon.value.kind, 'ok'); assert.equal(JSON.parse(anon.value.user), null)
    backend.closeAllConnections(); await new Promise(resolve => backend.close(resolve))
    const unavailable = await call({ chain: runtime, guards: [], middleware: [], ctx, secret: '' })
    assert.equal(unavailable.value.kind, 'error'); assert.equal(unavailable.value.status, '503')
    assert.equal(unavailable.value.user, undefined, 'unavailable authority cannot fall back to a legacy signed user')
  } finally {
    if (pending) { clearTimeout(pending.timer); pending = null }
    lines?.close()
    if (child && child.exitCode === null && child.signalCode === null) {
      let timer; child.kill('SIGTERM'); await Promise.race([once(child, 'exit'), new Promise(resolve => { timer = setTimeout(resolve, 1000) })]); clearTimeout(timer)
      if (child.exitCode === null && child.signalCode === null) { child.kill('SIGKILL'); await once(child, 'exit') }
    }
    backend.closeAllConnections(); if (backend.listening) await new Promise(resolve => backend.close(resolve))
    rmSync(dir, { recursive: true, force: true })
  }
})
