// A route parameter never hides a real file: a dynamic view at the root ranks after public files and prerendered pages.
import { test, after } from 'node:test'
import assert from 'node:assert/strict'
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { createServer } from '../server/server.mjs'

const dist = mkdtempSync(join(tmpdir(), 'brain-dynamic-root-'))
mkdirSync(join(dist, '_server'))
writeFileSync(join(dist, 'favicon.svg'), '<svg xmlns="http://www.w3.org/2000/svg"/>')
writeFileSync(join(dist, 'login.html'), '<!doctype html><title>Login</title>')
writeFileSync(join(dist, 'code.html'), '<!doctype html><title>Code</title><!--brain:ssr-->')
writeFileSync(join(dist, '_server/code.mjs'), `export const seen = globalThis.__seen = []; export async function load(ctx) { seen.push(ctx.params.code); return {} }; export default function () {}`)
const route = { path: '/:code', rx: '^/(?<code>[^/]+)/?$', pattern: '^/(?<code>[^/]+)/?$', page: 'code.html', server: '_server/code.mjs', view: '_server/code.mjs' }
writeFileSync(join(dist, '_server/manifest.json'), JSON.stringify({ routes: [route] }))
const server = createServer({ dist, secret: 'synthetic-root-secret', log: () => {} })
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve))
const base = `http://127.0.0.1:${server.address().port}`
after(async () => {
  await new Promise(resolve => { server.close(resolve); server.closeAllConnections() })
  rmSync(dist, { recursive: true, force: true })
})

test('public files and prerendered pages answer before a root parameter', async () => {
  const icon = await fetch(`${base}/favicon.svg`)
  assert.equal(icon.status, 200)
  assert.match(icon.headers.get('content-type'), /svg/)
  const login = await fetch(`${base}/login`)
  assert.equal(login.status, 200)
  assert.match(await login.text(), /<title>Login/)
  const ico = await fetch(`${base}/favicon.ico`, { redirect: 'manual' })
  assert.equal(ico.status, 301)
  assert.deepEqual(globalThis.__seen ?? [], [], 'the view load never ran for a real file')
})

test('a path without a file still reaches the parameter route', async () => {
  await fetch(`${base}/abc1234`)
  assert.deepEqual(globalThis.__seen, ['abc1234'])
})
