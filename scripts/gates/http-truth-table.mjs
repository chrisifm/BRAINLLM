#!/usr/bin/env node
// HTTP truth table: what each handler return shape answers, on a real socket, directly and through every gateway.
// A row is one return shape of server/routes/<name>/handler.jsbrain; docs/backend/routes.md is the contract.
import assert from 'node:assert/strict'
import { readFileSync, rmSync } from 'node:fs'
import { join } from 'node:path'
import { binary, brain, createDatabase, freePort, gate, own, ready, stop, write } from './lib.mjs'

// case → [source of the return or throw, expected status, expected body (undefined: not compared), expected headers]
const rows = {
  plain_object: ['return { items: [1] }', 200, { items: [1] }],
  plain_array: ['return [1, 2]', 200, [1, 2]],
  plain_string: ["return 'text'", 200, 'text'],
  plain_number: ['return 7', 200, 7],
  plain_null: ['return null', 200, null],
  nothing: ['return undefined', 200, null],
  data_status_word: ["return { status: 'open' }", 200, { status: 'open' }],
  data_status_extra_key: ["return { status: 500, message: 'a field named status' }", 200, { status: 500, message: 'a field named status' }],
  empty_object: ['return {}', 200, {}],
  envelope_body_only: ['return { body: { a: 1 } }', 200, { a: 1 }],
  envelope_200: ['return { status: 200, body: { a: 1 } }', 200, { a: 1 }],
  envelope_no_body: ['return { status: 200 }', 200, null],
  envelope_201: ["return { status: 201, body: { id: 1 }, headers: { location: '/items/1', 'x-shape': 'a', 'set-cookie': 'a=1' } }", 201, { id: 1 }, { location: '/items/1', 'x-shape': 'a', 'set-cookie': null }],
  envelope_204: ['return { status: 204, body: { dropped: true } }', 204, ''],
  envelope_302: ["return { status: 302, headers: { location: '/moved' } }", 302, null, { location: '/moved' }],
  envelope_304: ['return { status: 304 }', 304, ''],
  envelope_400: ["return { status: 400, body: { reason: 'bad' } }", 400, { reason: 'bad' }],
  envelope_404: ["return { status: 404, body: { reason: 'gone' } }", 404, { reason: 'gone' }],
  envelope_422: ["return { status: 422, body: { reason: 'invalid' } }", 422, { reason: 'invalid' }],
  envelope_429: ["return { status: 429, body: { reason: 'slow' }, headers: { 'retry-after': '9' } }", 429, { reason: 'slow' }, { 'retry-after': '9' }],
  envelope_500: ['return { status: 500, body: { retry: true } }', 500, { retry: true }],
  envelope_502: ['return { status: 502, body: { retry: true } }', 502, { retry: true }],
  envelope_503: ["return { status: 503, body: { retry: true }, headers: { 'retry-after': '7' } }", 503, { retry: true }, { 'retry-after': '7' }],
  envelope_504: ['return { status: 504, body: { retry: true } }', 504, { retry: true }],
  envelope_500_no_body: ['return { status: 500 }', 500, null],
  invalid_status_zero: ['return { status: 0, body: {} }', 500, 'ROUTE_INVALID_STATUS'],
  invalid_status_101: ['return { status: 101, body: {} }', 500, 'ROUTE_INVALID_STATUS'],
  invalid_status_600: ['return { status: 600, body: {} }', 500, 'ROUTE_INVALID_STATUS'],
  invalid_status_string: ["return { status: '500', body: {} }", 500, 'ROUTE_INVALID_STATUS'],
  invalid_status_float: ['return { status: 200.5, body: {} }', 500, 'ROUTE_INVALID_STATUS'],
  file: ["return { file: { base64: 'aGk=', name: 'a.txt', type: 'text/plain' } }", 200, 'hi', { 'content-disposition': /^attachment/, 'x-content-type-options': 'nosniff' }],
  file_invalid_status: ["return { status: 404, file: { base64: 'aGk=', name: 'a.txt' } }", 500, 'ROUTE_INVALID_STATUS'],
  thrown_fields: ["throw { errors: { name: 'required' } }", 422, 'INVALID_INPUT'],
  thrown_status_409: ['throw { status: 409 }', 409, 'ROUTE_REJECTED'],
  thrown_error_status_403: ["throw Object.assign(new Error('private detail'), { status: 403 })", 403, 'ROUTE_REJECTED'],
  thrown_not_found: ['throw notFound()', 404, 'ROUTE_REJECTED'],
  thrown_gone: ['throw gone()', 410, 'ROUTE_REJECTED'],
  thrown_redirect: ["throw redirect('/moved')", 303, null, { location: '/moved' }],
  returned_redirect: ["return redirect('/moved', 307)", 307, null, { location: '/moved' }],
  thrown_status_503: ['throw { status: 503 }', 500, 'ROUTE_FAILED'],
  thrown_error: ["throw new Error('private detail')", 500, 'ROUTE_FAILED'],
  thrown_string: ["throw 'private detail'", 500, 'ROUTE_FAILED'],
}
const cases = Object.entries(rows).map(([name, [source]]) => `  if (ctx.params.name === '${name}') { ${source} }`).join('\n')
const code = value => typeof value === 'string' && /^[A-Z_]+$/.test(value)

async function verify(label, base) {
  const failures = []
  for (const [name, [, status, body, headers]] of Object.entries(rows)) {
    for (const accept of ['application/json', 'text/html']) {
      const response = await fetch(`${base}/shape/${name}`, { headers: { accept }, redirect: 'manual', signal: AbortSignal.timeout(20000) })
      const text = await response.text()
      const seen = `${label} ${name} accept=${accept}: ${response.status} ${text.slice(0, 160)}`
      if (response.status !== status) { failures.push(`${seen} (expected status ${status})`); continue }
      if (text.includes('private detail')) failures.push(`${seen} (a private message left the server)`)
      if (code(body) && accept === 'text/html') {
        // A default error is a page for a browser: the status stays and the reference is on the page.
        if (!/^<!doctype html/i.test(text) && !text.includes(body)) failures.push(`${seen} (neither its error page nor its code)`)
      } else if (code(body)) {
        let parsed = null; try { parsed = JSON.parse(text) } catch {}
        if (parsed?.error?.code !== body) failures.push(`${seen} (expected code ${body})`)
        if (status >= 500 && !parsed?.rid) failures.push(`${seen} (a 5xx without its rid)`)
      } else if (body === 'hi' || body === '') {
        if (text !== body) failures.push(`${seen} (expected the body ${JSON.stringify(body)})`)
      } else if (body !== undefined) {
        let parsed; try { parsed = JSON.parse(text) } catch { parsed = Symbol('not JSON') }
        try { assert.deepEqual(parsed, body) } catch { failures.push(`${seen} (expected the body ${JSON.stringify(body)})`) }
      }
      for (const [key, want] of Object.entries(headers || {})) {
        const got = response.headers.get(key)
        if (want instanceof RegExp ? !want.test(got || '') : got !== want) failures.push(`${seen} (header ${key} is ${got}, expected ${want})`)
      }
    }
  }
  // A body too long to be taken for a page, on a route failure, and a HEAD that keeps the status.
  const head = await fetch(`${base}/shape/envelope_503`, { method: 'HEAD', signal: AbortSignal.timeout(20000) })
  if (head.status !== 405 && head.status !== 503) failures.push(`${label} HEAD on a GET route answered ${head.status}`)
  const slow = await fetch(`${base}/slow`, { headers: { accept: 'application/json' }, signal: AbortSignal.timeout(20000) })
  const late = await slow.json().catch(() => null)
  if (slow.status !== 504 || late?.error?.code !== 'ROUTE_TIMEOUT') failures.push(`${label} slow: ${slow.status} ${JSON.stringify(late)} (expected 504 ROUTE_TIMEOUT)`)
  return failures
}

await gate('http-truth-table', 8 * 60 * 1000, async directory => {
  const database = createDatabase('gate_truth')
  const env = { DATABASE_URL: database, BRAIN_MAIL_RESEND_KEY: '', GOOGLE_CLIENT_ID: '' }
  brain(['new', 'app', '--json'], directory, env)
  const app = join(directory, 'app')
  write(app, 'server/routes/shape/route.jsbrain', "<meta server>\nexport default { path: '/shape/{name}', auth: 'public', rate: '6000/min' }\n")
  write(app, 'server/routes/shape/handler.jsbrain', `<meta server>\nexport default async function handle(ctx) {\n${cases}\n  return { status: 404, body: { unknown: ctx.params.name } }\n}\n`)
  write(app, 'server/routes/slow/route.jsbrain', "<meta server>\nexport default { path: '/slow', auth: 'public', timeout: '200ms' }\n")
  write(app, 'server/routes/slow/handler.jsbrain', '<meta server>\nexport default async function handle() {\n  await new Promise(resolve => setTimeout(resolve, 5000))\n  return { late: true }\n}\n')
  brain(['backend', 'migrate', '--json'], app, env)
  const nativePort = await freePort()
  const nativeURL = `http://127.0.0.1:${nativePort}`
  // One backend per surface: every surface starts with empty per-client counters, so its rows are not rate limited.
  let native
  const backend = async () => {
    if (native) await stop(native.child)
    native = own(binary, ['backend', 'serve', '--port', String(nativePort)], app, env)
    await ready(nativeURL + '/health', native)
  }
  await backend()
  const failures = await verify('native', nativeURL)
  const gatewayEnv = { ...env, BRAIN_BACKEND_URL: nativeURL }

  // Rust development gateway.
  await backend()
  const devPort = await freePort()
  const dev = own(binary, ['run', '--port', String(devPort)], app, gatewayEnv)
  await ready(`http://127.0.0.1:${devPort}/`, dev, 60000)
  failures.push(...await verify('brain run', `http://127.0.0.1:${devPort}`))
  await stop(dev.child)

  // Generated Node gateway: a server function makes the build emit server.mjs.
  write(app, 'views/save.brain', '<meta seo keyword="save page" title="Save page" description="A page with a server function, so the build emits the production server." canonical="/save">\n<script>\n  server: async function save(ctx, value) { return value }\n  let done = sig(\'\')\n  const send = async () => { done = await save(\'x\') }\n</script>\n<main><h1>Save page</h1><p>{done}</p><button type="button" on:click={send}>Send</button></main>\n')
  brain(['build'], app, gatewayEnv)
  readFileSync(join(app, 'dist/server.mjs'))
  await backend()
  const nodePort = await freePort()
  const node = own(binary, ['start', '--port', String(nodePort), '--env', 'development'], app, gatewayEnv)
  await ready(`http://127.0.0.1:${nodePort}/`, node, 60000)
  failures.push(...await verify('brain start (Node)', `http://127.0.0.1:${nodePort}`))
  await stop(node.child)

  // Fixed Rust gateway: `brain start` selects it when dist/ has no Node server; the manifest keeps the declared routes.
  rmSync(join(app, 'dist/server.mjs'))
  await backend()
  const staticPort = await freePort()
  const fixed = own(binary, ['start', '--port', String(staticPort), '--env', 'development'], app, gatewayEnv)
  await ready(`http://127.0.0.1:${staticPort}/`, fixed, 60000)
  failures.push(...await verify('brain start (static)', `http://127.0.0.1:${staticPort}`))
  await stop(fixed.child)
  await stop(native.child)
  assert.equal(failures.length, 0, `${failures.length} truth table rows differ:\n${failures.join('\n')}`)
  console.log(`http-truth-table: ${Object.keys(rows).length} shapes on 4 surfaces`)
})
