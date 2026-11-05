// Pure helper availability in private handlers must not force browser helpers into every route.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

const brain = process.env.BRAIN_BIN ?? new URL('../../target/debug/brain', import.meta.url).pathname
test('check accepts pure helpers in both handler graphs and build loads browser helpers only where used', { timeout: 120000 }, () => {
  const root = mkdtempSync(join(tmpdir(), 'brain-server-native-'))
  const run = (args, cwd) => {
    const result = spawnSync(brain, args, { cwd, encoding: 'utf8', timeout: 90000, maxBuffer: 4 * 1024 * 1024 })
    assert.equal(result.status, 0, result.stdout + result.stderr)
    return JSON.parse(result.stdout)
  }
  try {
    run(['new', 'site', '--json'], root)
    const app = join(root, 'site')
    const put = (name, source) => { const file = join(app, name); mkdirSync(join(file, '..'), { recursive: true }); writeFileSync(file, source) }
    put('server/jobs/sample/schema.jsbrain', '<meta server>\nexport default {}\n')
    put('server/jobs/sample/handler.jsbrain', "<meta server>\nexport default async function handle(ctx) { return { values: range(0, 50), folded: text.fold('EXAMPLE') } }\n")
    put('server/lib/pure.jsbrain', "<meta server>\nexport default function values() { return { values: range(0, 50), folded: text.fold('EXAMPLE') } }\n")
    put('server/routes/sample/route.jsbrain', "<meta server>\nexport default { method: 'GET', path: '/example-handler', auth: 'public' }\n")
    put('server/routes/sample/handler.jsbrain', "<meta server>\nimport values from '../../lib/pure.jsbrain'\nexport default async function handle(ctx) { return values() }\n")
    const check = run(['check', '--json'], app)
    assert.ok(!JSON.stringify(check).includes('B0792'))
    put('views/helpers.brain', '<meta seo title="Pure helper example" description="A fixed example showing bounded native list and text helpers loaded only by the page that uses their output."><script>const choices = range(0, 3); const word = text.fold("EXAMPLE"); let count = sig(0)</script><main><h1>Pure helper example</h1><p>{word}</p>{#each choices as item (item)}<p>{item}</p>{/each}<button on:click={() => count++}>Count {count}</button></main>')
    const build = run(['build', '--site', 'https://example.test', '--json'], app)
    for (const route of ['/', '/brandbook', '/helpers']) {
      const page = build.ledger.find(page => page.route === route)
      assert.ok(page, route)
      for (const helper of ['brain-list.js', 'brain-text.js']) {
        const includes = page.assets.some(asset => asset.source === 'runtime/' + helper)
        assert.equal(includes, route === '/helpers', route + ' ' + helper)
      }
    }
    for (const kind of ['jobs', 'routes']) {
      put(`server/${kind}/sample/handler.jsbrain`, "<meta server>\nexport default async function handle(ctx) { return await api('/rows') }\n")
    }
    const refused = spawnSync(brain, ['check', '--json'], { cwd: app, encoding: 'utf8', timeout: 30000, maxBuffer: 1024 * 1024 })
    assert.equal(refused.status, 1, refused.stdout + refused.stderr)
    const diagnostics = JSON.parse(refused.stdout).filter(diagnostic => diagnostic.code === 'B0792')
    assert.equal(diagnostics.length, 2)
    for (const diagnostic of diagnostics) {
      assert.equal(diagnostic.level, 'error')
      assert.ok(diagnostic.fix.includes('browser-only'), diagnostic.fix)
      assert.ok(!diagnostic.fix.includes('<meta server>'), 'already server-only sources need a different fix')
      assert.ok(!diagnostic.msg.includes('; fix:'), 'the machine-readable fix must not be duplicated into the message')
    }
  } finally { rmSync(root, { recursive: true, force: true }) }
})
