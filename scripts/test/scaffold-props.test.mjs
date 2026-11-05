// Generated test fixtures must satisfy required top-level props in both static and ordinary applications.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { mkdtempSync, readFileSync, writeFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
const brain = process.env.BRAIN_BIN ?? new URL('../../target/debug/brain', import.meta.url).pathname
const fields = 'title:str amount:num enabled:bool items:list options:obj onPick:fn'
function cli(args, cwd) {
  const result = spawnSync(brain, args, { cwd, encoding: 'utf8', timeout: 60000, maxBuffer: 2 * 1024 * 1024 })
  assert.equal(result.status, 0, result.stdout + result.stderr)
  return JSON.parse(result.stdout)
}
test('generated required-prop tests mount real components in Chrome under static and ordinary modes', { timeout: 180000 }, () => {
  const root = mkdtempSync(join(tmpdir(), 'brain-scaffold-props-'))
  try {
    for (const mode of ['static', 'ordinary']) {
      cli(['new', mode, ...(mode === 'static' ? ['--static'] : []), '--json'], root)
      const app = join(root, mode)
      cli(['add', 'comp', 'RequiredInputs', '--props', fields, '--json'], app)
      const source = join(app, 'comp/required-inputs.brain')
      writeFileSync(source, `<meta props="${fields}">\n<section aria-label="Required inputs"><p>{title} {amount} {String(enabled)} {items.length} {Object.keys(options).length}</p><button type="button" on:click={onPick}>Choose</button></section>`)
      const test = readFileSync(join(app, 'comp/required-inputs.test.brain'), 'utf8')
      assert.match(test, /items: \[\], options: \{\}, onPick: \(\) => \{\}/)
      const report = cli(['test', 'comp/required-inputs.test.brain', '--json'], app)
      assert.ok(JSON.stringify(report).includes('renders'), JSON.stringify(report))
      writeFileSync(source, readFileSync(source, 'utf8').replace('>Choose</button>', '></button>'))
      const invalid = spawnSync(brain, ['test', 'comp/required-inputs.test.brain', '--json'], { cwd: app, encoding: 'utf8', timeout: 20000, maxBuffer: 2 * 1024 * 1024 })
      assert.notEqual(invalid.status, 0, invalid.stdout + invalid.stderr)
      assert.match(invalid.stdout, /test preflight failed/)
      assert.match(invalid.stdout, /B0406/)
      assert.match(invalid.stdout, /comp\/required-inputs\.brain/)
      cli(['add', 'comp', 'ContentPanel', '--blank', '--props', 'title:str count:num=0', '--json'], app)
      const blank = readFileSync(join(app, 'comp/content-panel.brain'), 'utf8')
      assert.ok(blank.includes('<slot />') && !blank.includes('<script>') && !blank.includes('<button') && !blank.includes('<input'))
      cli(['test', 'comp/content-panel.test.brain', '--json'], app)
      const originalConfig = readFileSync(join(app, 'brain.json'), 'utf8')
      const rejected = spawnSync(brain, ['add', 'locale', 'fr', '--blank', '--json'], { cwd: app, encoding: 'utf8', timeout: 10000 })
      assert.notEqual(rejected.status, 0)
      assert.match(rejected.stdout, /--blank is only supported/)
      assert.equal(readFileSync(join(app, 'brain.json'), 'utf8'), originalConfig)
    }
  } finally { rmSync(root, { recursive: true, force: true }) }
})
