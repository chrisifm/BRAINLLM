#!/usr/bin/env node
// Signed application identity, invalid native sessions, backend outages and rate limits remain distinct.
// This synthetic gate does not identify the cause of an earlier production incident.
import assert from 'node:assert/strict'
import { createServer } from 'node:http'
import { createHmac } from 'node:crypto'
import { join } from 'node:path'
import { binary, brain, freePort, gate, own, ready, stop, write } from './lib.mjs'

await gate('session-boundary', 3 * 60 * 1000, async directory => {
  const secret = 'synthetic-app-session-secret-0123456789'
  let backendMode = 'ok', requests = [], dev
  const backend = createServer((request, response) => {
    let bytes = 0
    request.on('data', chunk => { bytes += chunk.length; if (bytes > 65536) request.destroy() })
    request.on('end', () => {
      requests.push({ path: request.url, nativeCookie: /(?:^|;\s*)__Host-brain/.test(request.headers.cookie || '') })
      const auth = request.url.startsWith('/_brain/auth/')
      const status = backendMode === 'down' ? 503 : auth ? 401 : backendMode === 'rate' ? 429 : 200
      response.writeHead(status, { 'content-type': 'application/json', 'retry-after': '1' })
      response.end(status === 200 ? '[]' : JSON.stringify({ error: {
        code: status === 429 ? 'RATE_LIMITED' : status === 503 ? 'OVERLOADED' : 'UNAUTHENTICATED',
        fix: 'Synthetic controlled failure',
      } }))
    })
  })
  try {
    await new Promise((resolve, reject) => backend.once('error', reject).listen(0, '127.0.0.1', resolve))
    const backendURL = `http://127.0.0.1:${backend.address().port}`
    const user = { id: 'synthetic-user', role: 'user', name: 'Synthetic Person', avatar: 'https://example.test/avatar.png' }
    const encoded = Buffer.from(JSON.stringify({ u: user, exp: Date.now() + 3600000 })).toString('base64url')
    const appCookie = '__Host-session=' + encoded + '.' + createHmac('sha256', secret).update(encoded).digest('base64url')
    const nativeCookie = '__Host-brain-access=synthetic-expired; __Host-brain-refresh=synthetic-expired-refresh'
    for (const session of ['app', 'native']) {
      backendMode = 'ok'
      const name = 'session-' + session
      const env = { BRAIN_BACKEND_URL: backendURL, BRAIN_BACKEND_SESSION: session, BRAIN_SECRET: secret }
      brain(['new', name, '--json'], directory, env)
      const app = join(directory, name), port = await freePort(), base = `http://127.0.0.1:${port}`
      write(app, '.brain_env', `PUBLIC_SITE_URL=http://localhost:5180\n[development]\nBRAIN_SECRET=${secret}\nBRAIN_BACKEND_URL=${backendURL}\nBRAIN_BACKEND_SESSION=${session}\n`)
      write(app, 'views/plain.brain', `<meta seo title="Session example" description="A synthetic server rendered page demonstrates signed application session identity without assuming a native account shape." keyword="example">
<script>export async function load(ctx) { return { who: ctx.user?.name ?? 'anonymous' } }</script>
<main><h1>Identity {props.who}</h1></main>\n`)
      write(app, 'views/reading.brain', `<meta seo title="Reading example" description="A synthetic server rendered page demonstrates the HTTP status retained when a backend request is refused by a declared rate policy." keyword="example">
<script>export async function load(ctx) { await ctx.backend.resource('items').list(); return { ok: true } }</script>
<main><h1>Reading result</h1></main>\n`)
      dev = own(binary, ['run', '--port', String(port), '--no-inspect'], app, env)
      await ready(base + '/@brain/ping', dev)
      const get = async (path, cookie) => {
        const response = await fetch(base + path, { headers: { cookie }, signal: AbortSignal.timeout(15000) })
        const text = await response.text()
        assert.ok(text.length <= 2 * 1024 * 1024, 'the synthetic response must remain bounded')
        const route = /<script[^>]*id=["']brain-route["'][^>]*>([\s\S]*?)<\/script>/.exec(text)?.[1]
        return { status: response.status, props: route ? JSON.parse(route).p : null }
      }
      if (session === 'app') {
        const signed = await get('/plain', appCookie + '; ' + nativeCookie)
        assert.equal(signed.status, 200)
        assert.equal(signed.props?.who, user.name, 'signed app identity does not need a native JWT shape')
        backendMode = 'down'
        requests = []
        const independent = await get('/plain', appCookie)
        assert.equal(independent.status, 200, 'a public load that only reads app identity does not depend on native auth')
        assert.equal(independent.props?.who, user.name)
        assert.equal(requests.length, 0, 'app identity must not probe the unavailable native backend')
        backendMode = 'ok'
        const invalid = await get('/plain', '__Host-session=invalid')
        assert.equal(invalid.status, 200)
        assert.equal(invalid.props?.who, 'anonymous')
        backendMode = 'rate'
        requests = []
        const limited = await get('/reading', appCookie + '; ' + nativeCookie)
        assert.equal(limited.status, 429, 'a refused backend read keeps 429 instead of becoming 500')
        const resources = requests.filter(request => request.path.startsWith('/api/'))
        assert.ok(resources.length > 0, 'control: the public load actually reached the resource API')
        assert.ok(resources.every(request => !request.nativeCookie), 'leftover native cookies never authorize an app-session resource read')
      } else {
        backendMode = 'ok'
        const invalid = await get('/plain', nativeCookie)
        assert.equal(invalid.status, 200)
        assert.equal(invalid.props?.who, 'anonymous', 'a rejected native identity may become anonymous on a public page')
        backendMode = 'down'
        const unavailable = await get('/plain', nativeCookie)
        assert.equal(unavailable.status, 503, 'an unavailable identity backend must not be treated as a signed-out visitor or 500')
      }
      await stop(dev.child)
      dev = undefined
    }
    console.log('Session boundary coverage: signed app identity, invalid identity, native 503, public read 429 and resource-cookie isolation')
  } finally {
    if (dev) await stop(dev.child)
    backend.closeAllConnections()
    if (backend.listening) await new Promise(resolve => backend.close(resolve))
  }
})
