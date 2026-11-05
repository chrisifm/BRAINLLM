// An intentional live-site link must behave consistently in ordinary and static builds.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { mkdtempSync, writeFileSync, readFileSync, rmSync } from 'node:fs'
import { join } from 'node:path'
import { tmpdir } from 'node:os'
const brain = process.env.BRAIN_BIN ?? new URL('../../target/debug/brain', import.meta.url).pathname
const cli = (args, cwd) => spawnSync(brain, args, { cwd, encoding: 'utf8', timeout: 60000, maxBuffer: 2 * 1024 * 1024 })
test('same-origin absolute external anchors build while relative and unmarked missing links fail', { timeout: 180000 }, () => {
  const root = mkdtempSync(join(tmpdir(), 'brain-external-navigation-'))
  try {
    for (const mode of ['static', 'ordinary']) {
      const created = cli(['new', mode, ...(mode === 'static' ? ['--static'] : []), '--json'], root)
      assert.equal(created.status, 0, created.stdout + created.stderr)
      const app = join(root, mode), view = join(app, 'views/about.brain')
      const page = anchor => '<meta seo title="About this project" description="Read the project reference and visit its separate public site for current content, independent from this generated artifact." canonical="/about"><main><h1>About this project</h1>' + anchor + '</main>'
      writeFileSync(view, page('<a href="https://example.test/live-only/" rel="noopener EXTERNAL">Live site</a>'))
      const built = cli(['build', '--site', 'https://example.test', '--json'], app)
      assert.equal(built.status, 0, built.stdout + built.stderr)
      assert.ok(readFileSync(join(app, 'dist/about/index.html'), 'utf8').includes('https://example.test/live-only/'))
      for (const anchor of ['<a href="https://example.test/missing/">Missing</a>', '<a href="/missing/" rel="external">Missing</a>']) {
        writeFileSync(view, page(anchor))
        const rejected = cli(['build', '--site', 'https://example.test', '--json'], app)
        assert.notEqual(rejected.status, 0, rejected.stdout + rejected.stderr)
        assert.match(rejected.stdout + rejected.stderr, /B1819/)
      }
    }
  } finally { rmSync(root, { recursive: true, force: true }) }
})
