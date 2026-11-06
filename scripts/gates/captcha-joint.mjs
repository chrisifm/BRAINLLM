#!/usr/bin/env node
// A compiled server function must complete protected signup and login with one provider token.
// Run through backend-check: all database state, listeners and native children belong to this gate.
import assert from 'node:assert/strict'
import { createServer } from 'node:http'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { pathToFileURL } from 'node:url'
import { binary, brain, createDatabase, freePort, gate, own, ready, stop, write } from './lib.mjs'

await gate('captcha-joint', 4 * 60 * 1000, async directory => {
  const database = createDatabase('gate_captcha_joint')
  const backendPort = await freePort(), frontendPort = await freePort()
  const backendBase = `http://127.0.0.1:${backendPort}`, frontendBase = `http://127.0.0.1:${frontendPort}`
  const secret = 'synthetic-captcha-secret', operator = 'synthetic-operator-key-0123456789abcdef'
  let mode = 'normal', providerCalls = 0, native, frontend
  const spent = new Set()
  const provider = createServer((request, response) => {
    let body = '', bytes = 0
    request.on('data', chunk => {
      bytes += chunk.length
      if (bytes > 4096) { response.writeHead(413).end(); request.destroy(); return }
      body += chunk
    })
    request.on('end', () => {
      providerCalls++
      const values = new URLSearchParams(body), token = values.get('response')
      const success = mode === 'normal' && values.get('secret') === secret && !!token && !spent.has(token)
      if (success) spent.add(token)
      response.writeHead(mode === 'down' ? 503 : 200, { 'content-type': 'application/json' })
      response.end(JSON.stringify({ success, hostname: 'localhost' }))
    })
  })
  try {
    await new Promise((resolve, reject) => provider.once('error', reject).listen(0, '127.0.0.1', resolve))
    const env = {
      DATABASE_URL: database, PUBLIC_SITE_URL: frontendBase, CAPTCHA_SECRET: secret, CAPTCHA_OPERATOR_KEY: operator,
      BRAIN_MAIL_RESEND_KEY: '', BRAIN_MAIL_FROM: '', GOOGLE_CLIENT_ID: '', GOOGLE_CLIENT_SECRET: '',
      APPLE_CLIENT_ID: '', APPLE_NATIVE_APP_IDS: '', BRAIN_AUTH_LINKEDIN_CLIENT_ID: '', BRAIN_AUTH_LINKEDIN_CLIENT_SECRET: '', BRAIN_JWT_SECRET: '',
    }
    brain(['new', 'app', '--json'], directory, env)
    const app = join(directory, 'app')
    write(app, 'server/auth/captcha.jsbrain', `<meta server>\nexport default { verify_url: 'http://127.0.0.1:${provider.address().port}/siteverify', secret_env: 'CAPTCHA_SECRET', operator_key_env: 'CAPTCHA_OPERATOR_KEY', protect: ['signup', 'login'] }\n`)
    write(app, 'views/bridge.brain', `<meta seo title="Synthetic signup bridge" description="This page exercises signup and login through a compiled server function with an explicitly protected native authentication backend." keyword="example">
<script>
server: async function join(ctx, values) {
  await ctx.backend.signup(values)
  return ctx.backend.login(values)
}
</script>
<main><h1>Signup bridge</h1><button on:click={() => join({ email: 'example@example.test', password: 'synthetic-password-1234' })}>Join</button></main>\n`)
    brain(['backend', 'migrate', '--env', 'development', '--json'], app, env)
    brain(['build', '--env', 'development', '--site', 'https://example.test', '--json'], app, env)
    native = own(binary, ['backend', 'serve', '--env', 'development', '--port', String(backendPort), '--origin', frontendBase], app, env)
    await ready(backendBase + '/health', native)
    const health = await fetch(backendBase + '/health', { signal: AbortSignal.timeout(2000) })
    await health.body?.cancel()
    assert.equal(health.status, 200, 'the owned backend must be healthy before auth assertions')
    const dist = join(app, 'dist')
    const { createServer: frontendServer } = await import(pathToFileURL(join(dist, 'server.mjs')).href)
    frontend = frontendServer({ dist, backendURL: backendBase, log() {} })
    await new Promise((resolve, reject) => frontend.once('error', reject).listen(frontendPort, '127.0.0.1', resolve))
    const manifest = JSON.parse(readFileSync(join(dist, '_server/manifest.json'), 'utf8'))
    const owners = Object.entries(manifest.rpc).filter(([, path]) => /export async function join\(/.test(readFileSync(join(dist, path), 'utf8')))
    assert.equal(owners.length, 1, 'the CLI must emit exactly one RPC owner for the source function')
    const rpcKey = owners[0][0]
    const credentials = email => ({ email, password: 'synthetic-password-1234' })
    const rpc = async (email, token) => {
      const response = await fetch(`${frontendBase}/_r/${rpcKey}/join`, {
        method: 'POST', headers: { origin: frontendBase, 'content-type': 'application/json', 'x-brain-rpc': '1', 'x-brain-captcha': token },
        body: JSON.stringify([credentials(email)]), signal: AbortSignal.timeout(15000),
      })
      return { status: response.status, text: await response.text(), cookies: response.headers.getSetCookie() }
    }
    const firstCalls = providerCalls
    const joined = await rpc('first@example.test', 'synthetic-once-token')
    assert.equal(joined.status, 200, 'the actual compiled BFF signup/login must succeed')
    assert.equal(providerCalls - firstCalls, 1, 'login must spend the signup grant, not verify the token twice')
    assert.ok(joined.cookies.length > 0, 'a successful login must relay its native cookie')
    assert.ok(joined.cookies.every(cookie => /;\s*HttpOnly/i.test(cookie) && /;\s*Secure/i.test(cookie)))
    assert.ok(!joined.text.includes('login_grant') && !joined.text.includes('synthetic-once-token'), 'the RPC response must not expose captcha credentials')
    const replay = await rpc('replay@example.test', 'synthetic-once-token')
    assert.equal(replay.status, 403, 'a spent provider token must never create another account')
    assert.equal(replay.cookies.length, 0)
    mode = 'down'
    const unavailable = await rpc('down@example.test', 'synthetic-down-token')
    assert.equal(unavailable.status, 503, 'provider failure must remain unavailable through the real BFF')
    assert.equal(unavailable.cookies.length, 0)
    mode = 'normal'
    const post = async (base, path, email, headers) => {
      const response = await fetch(`${base}/_brain/auth/${path}`, {
        method: 'POST', headers: { origin: frontendBase, 'content-type': 'application/json', ...headers },
        body: JSON.stringify(credentials(email)), signal: AbortSignal.timeout(10000),
      })
      return { status: response.status, value: await response.json(), cookies: response.headers.getSetCookie() }
    }
    // Also drive the explicitly returned one-use grant through the site's native proxy, not a mock backend.
    const proxied = await post(frontendBase, 'signup', 'grant@example.test', { 'x-brain-captcha': 'synthetic-grant-token' })
    assert.equal(proxied.status, 200)
    const grant = proxied.value.login_grant
    assert.equal(typeof grant, 'string')
    const beforeGrantLogin = providerCalls
    assert.equal((await post(frontendBase, 'login', 'grant@example.test', { 'x-brain-login-grant': grant })).status, 200)
    const grantReplay = await post(frontendBase, 'login', 'grant@example.test', { 'x-brain-login-grant': grant })
    assert.equal(grantReplay.status, 403)
    assert.equal(grantReplay.cookies.length, 0)
    assert.equal(providerCalls, beforeGrantLogin, 'neither grant use nor replay may reach the provider')
    const beforeOperator = providerCalls
    const imported = await post(backendBase, 'signup', 'operator@example.test', { 'x-brain-operator-key': operator })
    assert.equal(imported.status, 200)
    assert.equal(imported.value.login_grant, undefined)
    assert.equal(providerCalls, beforeOperator, 'the declared operator import does not call the provider')
    assert.equal((await post(backendBase, 'login', 'operator@example.test', { 'x-brain-operator-key': operator })).status, 403)
    const browserOperator = await post(frontendBase, 'signup', 'browser-operator@example.test', { 'x-brain-operator-key': operator })
    assert.equal(browserOperator.status, 403, 'the site proxy must not forward an operator credential')
    assert.equal(browserOperator.cookies.length, 0)
    console.log('Captcha joint coverage: compiled RPC signup/login, single provider verification, token/grant replay refusal, provider failure, native operator signup only, proxy operator refusal')
  } finally {
    if (frontend) { frontend.closeAllConnections(); await new Promise(resolve => frontend.close(resolve)) }
    if (native) await stop(native.child)
    provider.closeAllConnections()
    if (provider.listening) await new Promise(resolve => provider.close(resolve))
  }
})
