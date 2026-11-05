// Generated documentation stays reproducible output; authored notes remain normal source files.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
const brain = process.env.BRAIN_BIN ?? new URL('../../target/debug/brain', import.meta.url).pathname

test('app and static starters ignore generated documentation while keeping authored notes', { timeout: 60000 }, () => {
  const root = mkdtempSync(join(tmpdir(), 'brain-docs-git-'))
  try {
    const emptyConfig = join(root, 'empty-git-config')
    writeFileSync(emptyConfig, '')
    const env = { ...process.env, GIT_CONFIG_GLOBAL: emptyConfig, GIT_CONFIG_NOSYSTEM: '1' }
    const run = (binary, args, cwd) => spawnSync(binary, args, { cwd, env, encoding: 'utf8', timeout: 20000, maxBuffer: 1024 * 1024 })
    const ok = (binary, args, cwd) => { const o = run(binary, args, cwd); assert.equal(o.status, 0, o.stderr || o.stdout); return o }
    for (const mode of ['app', 'static']) {
      ok(brain, ['new', mode, ...(mode === 'static' ? ['--static'] : []), '--json'], root)
      const app = join(root, mode)
      ok('git', ['-c', 'init.defaultBranch=main', 'init', '--quiet'], app)
      const ignore = readFileSync(join(app, '.gitignore'), 'utf8')
      ok(brain, ['docs', '--json'], app)
      mkdirSync(join(app, 'docs/app/notes'), { recursive: true })
      writeFileSync(join(app, 'docs/app/notes/product.md'), '# Authored product note\n')
      mkdirSync(join(app, 'docs/app/generated'), { recursive: true })
      writeFileSync(join(app, 'docs/app/generated/extra.md'), '# Reproducible generated section\n')
      for (const path of ['docs/app/index.md', 'docs/app/routes.md', 'docs/app/generated/extra.md']) {
        const o = run('git', ['-c', 'core.excludesFile=', 'check-ignore', '--quiet', path], app)
        assert.equal(o.status, 0, `${mode}: generated ${path} should stay out of Git`)
      }
      const note = run('git', ['-c', 'core.excludesFile=', 'check-ignore', '--quiet', 'docs/app/notes/product.md'], app)
      assert.equal(note.status, 1, `${mode}: authored notes must remain visible to Git`)
      assert.equal(readFileSync(join(app, '.gitignore'), 'utf8'), ignore, 'brain docs preserves the project ignore policy')
    }
  } finally { rmSync(root, { recursive: true, force: true }) }
})
