// Typed domain inputs and record metadata occupy separate namespaces through the real CLI.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from 'node:fs'
import { join } from 'node:path'
import { tmpdir } from 'node:os'

const brain = process.env.BRAIN_BIN ?? new URL('../../target/debug/brain', import.meta.url).pathname
test('check and schema accept typed id inputs without relaxing resource columns', { timeout: 60000 }, () => {
  const root = mkdtempSync(join(tmpdir(), 'brain-typed-input-'))
  const run = (args, cwd, expected = 0) => {
    const result = spawnSync(brain, args, { cwd, encoding: 'utf8', timeout: 30000, maxBuffer: 2 * 1024 * 1024 })
    assert.equal(result.status, expected, result.stdout + result.stderr)
    return JSON.parse(result.stdout)
  }
  try {
    run(['new', 'site', '--json'], root)
    const app = join(root, 'site')
    const put = (file, source) => { mkdirSync(join(app, file, '..'), { recursive: true }); writeFileSync(join(app, file), '<meta server>\nexport default ' + source + '\n') }
    put('server/resources/widgets/schema.jsbrain', "{ title: 'str!' }")
    put('server/resources/widgets/policy.jsbrain', "{ read: ['admin'], create: ['admin'], update: ['admin'], delete: ['admin'] }")
    put('server/resources/widgets/actions.jsbrain', "{ rename: { roles: ['admin'], input: { id: 'str!' }, set: { title: '$input.id' } } }")
    put('server/operations/lookup.jsbrain', "{ roles: ['admin'], input: { id: 'str!' }, steps: [{ as: 'item', read: 'widgets', id: '$input.id' }], result: ['item'] }")
    const diagnostics = run(['check', '--json'], app)
    assert.ok(!diagnostics.some(item => item.level === 'error'))
    const schema = run(['backend', 'schema', '--json'], app)
    assert.ok(JSON.stringify(schema).includes('$input.id'))
    put('server/resources/widgets/schema.jsbrain', "{ id: 'str!', title: 'str!' }")
    const refused = run(['check', '--json'], app, 1)
    assert.ok(refused.some(item => item.level === 'error'), 'reserved resource columns must remain rejected')
  } finally { rmSync(root, { recursive: true, force: true }) }
})
