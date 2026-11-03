// platform.access: "open" (default) lets every client in with no limit; perMinute and concurrency are opt-in; "own-host"
// also refuses other sites, foreign hosts and plain scripts on /api, /_brain and /s.
import { test, after } from 'node:test'
import assert from 'node:assert/strict'
import { createServer as httpServer } from 'node:http'
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { createServer, accessPolicy, accessRefusal, accessBudget } from '../server/server.mjs'

const policy = accessPolicy({ mode: 'own-host', origins: ['https://admin.example.com'], ips: ['203.0.113.0/24'], local: false }, 'https://shop.example.com')
const req = (headers, method = 'GET') => ({ method, headers: { host: 'shop.example.com', ...headers } })
const verdict = (headers, { path = '/api/items', client = '198.51.100.7', method } = {}) => accessRefusal(req(headers, method), path, client, policy)

test('browser evidence of the site passes', () => {
  assert.equal(verdict({ 'sec-fetch-site': 'same-origin' }), null)
  assert.equal(verdict({ origin: 'https://shop.example.com' }, { method: 'POST' }), null)
  assert.equal(verdict({ origin: 'https://admin.example.com' }), null)
  assert.equal(verdict({ referer: 'https://shop.example.com/jobs' }), null)
  assert.equal(verdict({ 'sec-fetch-site': 'none', 'sec-fetch-mode': 'navigate' }), null)
})

test('other sites, foreign hosts and scripts without evidence are refused', () => {
  assert.equal(verdict({}), 'no-browser-evidence')
  assert.equal(verdict({ origin: 'https://copycat.example' }), 'origin')
  assert.equal(verdict({ 'sec-fetch-site': 'cross-site', referer: 'https://copycat.example/' }), 'cross-site')
  assert.equal(verdict({ 'sec-fetch-site': 'none', 'sec-fetch-mode': 'cors' }), 'cross-site')
  assert.equal(verdict({ host: 'mirror.example', 'sec-fetch-site': 'same-origin' }), 'host')
})

test('credentials, listed addresses, exempt paths and provider callbacks pass', () => {
  assert.equal(verdict({ 'x-api-key': 'synthetic' }), null)
  assert.equal(verdict({ authorization: 'Bearer synthetic' }), null)
  assert.equal(verdict({}, { client: '203.0.113.9' }), null)
  assert.equal(verdict({}, { path: '/s/abc' }), null)
  assert.equal(verdict({}, { path: '/_brain/webhooks/in/payments', method: 'POST' }), null)
  assert.equal(verdict({}, { path: '/_brain/auth/apple/notifications', method: 'POST' }), null)
  assert.equal(verdict({ 'sec-fetch-site': 'cross-site', 'sec-fetch-mode': 'navigate' }, { path: '/_brain/auth/linkedin/callback' }), null)
  assert.equal(verdict({ 'sec-fetch-site': 'cross-site', 'sec-fetch-mode': 'navigate' }, { path: '/api/items' }), 'cross-site')
})

test('local mode lets loopback clients test on localhost', () => {
  const local = accessPolicy({ mode: 'own-host' }, 'https://shop.example.com')
  assert.equal(accessRefusal(req({ host: '127.0.0.1:5190' }), '/api/items', '127.0.0.1', local), null)
  assert.equal(accessRefusal(req({}), '/api/items', '198.51.100.7', local), 'no-browser-evidence')
})

test('invalid settings are refused', () => {
  assert.throws(() => accessPolicy({ origins: ['https://admin.example.com/'] }), /scheme:\/\/host/)
  assert.throws(() => accessPolicy({ hosts: ['*.example.com'] }), /lowercase host/)
  assert.throws(() => accessPolicy({ ips: ['0.0.0.0/0'] }))
  assert.throws(() => accessPolicy({ open: true }), /not a setting/)
  assert.throws(() => accessPolicy({ mode: 'closed' }), /open" or "own-host/)
  assert.throws(() => accessPolicy({ perMinute: 0 }), /whole number/)
  assert.throws(() => accessPolicy({ concurrency: 1.5 }), /whole number/)
})

test('open mode, the default, refuses nobody and budgets each client', () => {
  const open = accessPolicy({ perMinute: 3, local: false, ips: ['203.0.113.0/24'] }, 'https://shop.example.com')
  assert.equal(open.mode, 'open')
  assert.equal(accessRefusal(req({ 'user-agent': 'headless' }), '/api/items', '198.51.100.7', open), null)
  assert.equal(accessRefusal(req({ origin: 'https://copycat.example' }), '/api/items', '198.51.100.7', open), null)
  const key = 'budget-test '
  for (let i = 0; i < 3; i++) assert.equal(accessBudget('/jobs', '198.51.100.8', open, key), 0)
  assert.ok(accessBudget('/jobs', '198.51.100.8', open, key) > 0)
  assert.equal(accessBudget('/jobs', '198.51.100.9', open, key), 0) // another client has its own budget
  assert.equal(accessBudget('/jobs', '203.0.113.4', open, key), 0) // a listed address is not counted
  assert.equal(accessBudget('/_brain/webhooks/in/payments', '198.51.100.8', open, key), 0)
  const defaults = accessPolicy({}) // nothing is limited unless the app asks
  assert.equal(defaults.perMinute, null)
  assert.equal(defaults.concurrency, null)
  for (let i = 0; i < 500; i++) assert.equal(accessBudget('/jobs', '198.51.100.10', defaults, key), 0)
})

const upstream = httpServer((_, res) => { res.writeHead(200, { 'content-type': 'application/json' }); res.end('{"items":[]}') })
await new Promise(resolve => upstream.listen(0, '127.0.0.1', resolve))
const dist = mkdtempSync(join(tmpdir(), 'brain-access-'))
mkdirSync(join(dist, '_server'))
writeFileSync(join(dist, '_server/manifest.json'), JSON.stringify({ routes: [], access: { mode: 'own-host', local: false } }))
const logs = []
const server = createServer({ dist, backendURL: `http://127.0.0.1:${upstream.address().port}`, secret: 'synthetic-access-secret', log: (...entry) => logs.push(entry) })
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve))
const base = `http://127.0.0.1:${server.address().port}`
after(async () => {
  await new Promise(resolve => { server.close(resolve); server.closeAllConnections() })
  await new Promise(resolve => { upstream.close(resolve); upstream.closeAllConnections() })
  rmSync(dist, { recursive: true, force: true })
})

test('the server refuses a script, serves the page, and lists the refused source', async () => {
  const refused = await fetch(`${base}/api/items`, { headers: { 'user-agent': 'scraper/1.0' } })
  assert.equal(refused.status, 403)
  assert.equal((await refused.json()).code, 'B0752')
  const page = await fetch(`${base}/api/items`, { headers: { referer: `${base}/jobs` } })
  assert.equal(page.status, 200)
  await page.text()
  const entry = logs.find(([, message]) => message === 'access refused')
  assert.equal(entry[2].reason, 'no-browser-evidence')
  const { sources } = await (await fetch(`${base}/_access/sources`)).json()
  assert.equal(sources[0].count, 1)
  assert.equal(sources[0].userAgent, 'scraper/1.0')
  assert.equal((await fetch(`${base}/_access/sources`, { headers: { 'x-forwarded-for': '198.51.100.7' } })).status, 404)
})

test('a crawler is judged by evidence, never by its user agent', () => {
  assert.equal(verdict({ 'user-agent': 'Mozilla/5.0 (compatible; Googlebot/2.1; +http://www.google.com/bot.html)' }), 'no-browser-evidence')
  assert.equal(verdict({ 'user-agent': 'Mozilla/5.0 (compatible; Googlebot/2.1)', 'sec-fetch-site': 'same-origin' }), null)
  assert.equal(verdict({ 'user-agent': 'Googlebot' }, { path: '/s/launch' }), null)
})

test('pages, the sitemap and robots.txt never reach the check', async () => {
  for (const path of ['/robots.txt', '/sitemap.xml', '/jobs']) {
    const reply = await fetch(`${base}${path}`, { headers: { 'user-agent': 'Googlebot' } })
    assert.notEqual(reply.status, 403, path)
    assert.equal(reply.headers.get('x-brain-code'), null, path)
    await reply.text()
  }
})

test('an open server lets a script in and answers 429 past its budget, never touching the backend', async () => {
  let calls = 0
  const counting = httpServer((_, res) => { calls++; res.writeHead(200, { 'content-type': 'application/json' }); res.end('{}') })
  await new Promise(resolve => counting.listen(0, '127.0.0.1', resolve))
  const openDist = mkdtempSync(join(tmpdir(), 'brain-access-open-'))
  mkdirSync(join(openDist, '_server'))
  writeFileSync(join(openDist, '_server/manifest.json'), JSON.stringify({ routes: [], access: { local: false, perMinute: 2 } }))
  const lines = []
  const open = createServer({ dist: openDist, backendURL: `http://127.0.0.1:${counting.address().port}`, secret: 'synthetic-access-secret', log: (...entry) => lines.push(entry) })
  await new Promise(resolve => open.listen(0, '127.0.0.1', resolve))
  const at = `http://127.0.0.1:${open.address().port}`
  try {
    for (let i = 0; i < 2; i++) {
      const ok = await fetch(`${at}/api/items`, { headers: { 'user-agent': 'headless' } })
      assert.equal(ok.status, 200)
      await ok.text()
    }
    const limited = await fetch(`${at}/api/items`)
    assert.equal(limited.status, 429)
    assert.ok(+limited.headers.get('retry-after') > 0)
    await limited.text()
    assert.equal(calls, 2)
    assert.ok(lines.some(([, message]) => message === 'request limited'))
    // static and cached answers carry no budget
    const robots = await fetch(`${at}/robots.txt`)
    assert.notEqual(robots.status, 429)
    await robots.text()
  } finally {
    await new Promise(resolve => { open.close(resolve); open.closeAllConnections() })
    await new Promise(resolve => { counting.close(resolve); counting.closeAllConnections() })
    rmSync(openDist, { recursive: true, force: true })
  }
})
