// brain.json `server` (INTERNALS §11.1): the build writes { maxBodyKB, timeoutMs, rpcPerMinute } into the manifest and
// server.mjs enforces them for RPC and form bodies; BRAIN_MAX_BODY_KB / BRAIN_TIMEOUT_MS / BRAIN_RPC_LIMIT still win.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { mkdirSync, mkdtempSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

const { createServer } = await import('../server/server.mjs')

let n = 0 // the rate limiter is per IP + function hash for the whole process: one hash per app
function app(server) {
  const hash = 'h' + ++n
  const d = mkdtempSync(join(tmpdir(), 'brain-dist-limits-'))
  const w = (rel, s) => (mkdirSync(join(d, rel, '..'), { recursive: true }), writeFileSync(join(d, rel), s))
  w('_server/items.mjs', 'export async function save(ctx, data) { return data.length }\nexport async function wait() { await new Promise(r => setTimeout(r, 300)); return 1 }\n')
  w('_server/manifest.json', JSON.stringify({ v: 1, build: 'l1', routes: [], rpc: { [hash]: '_server/items.mjs' }, ...(server ? { server } : {}) }))
  return { dist: d, hash }
}

async function serve({ app: { dist, hash }, ...o }, fn) {
  const s = createServer({ dist, secret: 'test-secret-0123456789', log: () => {}, ...o }) // gitleaks:allow -- local test-only session secret
  await new Promise(r => s.listen(0, '127.0.0.1', r))
  try {
    const base = `http://127.0.0.1:${s.address().port}`
    await fn((name, body) => fetch(`${base}/_r/${hash}/${name}`, { method: 'POST', headers: { 'content-type': 'application/json', 'x-brain-rpc': '1' }, body }))
  } finally {
    s.close()
  }
}

test('server.maxBodyKB from the manifest bounds RPC bodies; the 413 names the setting', async () => {
  await serve({ app: app({ maxBodyKB: 4, timeoutMs: 10000, rpcPerMinute: 60 }) }, async call => {
    assert.equal((await call('save', JSON.stringify(['x'.repeat(3 * 1024)]))).status, 200)
    const r = await call('save', JSON.stringify(['x'.repeat(5 * 1024)]))
    assert.equal(r.status, 413)
    assert.match((await r.json()).error, /4 KB.*brain\.json server\.maxBodyKB/)
  })
})

test('server.timeoutMs and server.rpcPerMinute from the manifest apply', async () => {
  await serve({ app: app({ maxBodyKB: 1024, timeoutMs: 100, rpcPerMinute: 2 }) }, async call => {
    assert.equal((await call('wait', '[]')).status, 504) // 300 ms > 100 ms
    assert.equal((await call('save', '[[]]')).status, 200)
    assert.equal((await call('save', '[[]]')).status, 200)
    assert.equal((await call('save', '[[]]')).status, 429) // the third call to the same function within a minute
  })
})

test('environment overrides beat the manifest; the body limit is capped at 64 MB', async () => {
  await serve({ app: app({ maxBodyKB: 4, timeoutMs: 100, rpcPerMinute: 60 }), env: { BRAIN_MAX_BODY_KB: '16', BRAIN_TIMEOUT_MS: '2000' } }, async call => {
    assert.equal((await call('save', JSON.stringify(['x'.repeat(10 * 1024)]))).status, 200)
    assert.equal((await call('wait', '[]')).status, 200)
  })
  assert.throws(() => createServer({ dist: app({ maxBodyKB: 65537 }).dist, log: () => {} }), /maxBodyKB/)
})
