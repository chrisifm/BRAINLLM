import test from 'node:test'
import assert from 'node:assert/strict'
import { execFileSync, spawnSync } from 'node:child_process'
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { createServer } from 'node:http'
import { anonymousEnvironment, options, npmCli, childSession, httpProbe } from '../smoke-registry.mjs'

test('registry smoke requires an exact version and a bounded deadline without running npm', () => {
  assert.equal(options(['--help']).help, true)
  assert.deepEqual(options(['--version', '0.4.0', '--no-dev']), { version: '0.4.0', timeout: 900000, dev: false })
  for (const args of [[], ['--version', 'latest'], ['--version', '^0.4.0'], ['--version', 'https://example.test/package'], ['--version', '0.4.0', '--timeout-ms', '0'], ['--version', '0.4.0', '--timeout-ms', '900001'], ['--version', '0.4.0', '--binary', '/local/brain']]) assert.throws(() => options(args))
  const help = execFileSync(process.execPath, ['npm/smoke-registry.mjs', '--help'], { encoding: 'utf8', timeout: 10000 })
  assert.match(help, /anonymous|anonymously/)
  assert.equal(spawnSync(process.execPath, ['npm/smoke-registry.mjs', '--version', 'latest'], { timeout: 10000, stdio: 'pipe' }).status, 2)
})
test('anonymous environment drops every credential, local binary override and inherited npm config', () => {
  const env = anonymousEnvironment('/owned/scratch', { PATH: '/usr/bin', SystemRoot: 'C:\\Windows', HOME: '/private', NPM_TOKEN: 'not-a-real-token', NODE_AUTH_TOKEN: 'not-a-real-token', NPM_CONFIG_USERCONFIG: '/private/npmrc', BRAIN_BINARY: '/local/binary', BRAIN_NO_DELEGATE: '1', OPENAI_API_KEY: 'not-a-real-key', NODE_OPTIONS: '--require /untrusted.js', GITHUB_TOKEN: 'not-a-real-token', HTTPS_PROXY: 'http://proxy.test' })
  assert.equal(env.PATH, '/usr/bin')
  assert.equal(env.NPM_CONFIG_REGISTRY, 'https://registry.npmjs.org/')
  assert.ok(env.HOME.startsWith('/owned/scratch'))
  for (const name of ['NPM_TOKEN', 'NODE_AUTH_TOKEN', 'BRAIN_BINARY', 'BRAIN_NO_DELEGATE', 'OPENAI_API_KEY', 'NODE_OPTIONS', 'GITHUB_TOKEN', 'HTTPS_PROXY']) assert.equal(env[name], undefined, name)
  assert.ok(!Object.values(env).some(value => value.includes('not-a-real-')))
})
test('npm discovery uses its installed JavaScript CLI without invoking a platform shell', () => {
  assert.match(npmCli(), /npm-cli\.js$/)
})


test('owned command deadlines terminate the child and release lifecycle bookkeeping', { timeout: 10000 }, async () => {
  const root = mkdtempSync(join(tmpdir(), 'brain-registry-lifecycle-')), active = new Set()
  let session
  try {
    session = childSession(['-e', 'setInterval(()=>{},1000)'], { cwd: root, env: anonymousEnvironment(root), log: join(root, 'phase.log'), timeout: 100, phase: 'local-deadline', active })
    await assert.rejects(session.done, error => error.detail.phase === 'local-deadline' && error.detail.reason === 'deadline')
    assert.equal(active.size, 0)
    assert.equal(session.exited, true)
  } finally { session?.stop(); await session?.done.catch(() => {}); rmSync(root, { recursive: true, force: true }) }
})
test('temporary dev HTTP probe checks an actual loopback HTML response and rejects errors', { timeout: 10000 }, async () => {
  const server = createServer((req, res) => { res.writeHead(req.url === '/' ? 200 : 503, { 'content-type': 'text/html' }); res.end('<main>Owned smoke fixture</main>') })
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve))
  try {
    const url = `http://127.0.0.1:${server.address().port}`
    assert.equal(await httpProbe(url), true)
    assert.equal(await httpProbe(url + '/unavailable'), false)
  } finally { await new Promise(resolve => server.close(resolve)) }
})
