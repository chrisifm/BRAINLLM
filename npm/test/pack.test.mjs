import { test } from 'node:test'
import assert from 'node:assert/strict'
import { execFileSync } from 'node:child_process'
import { mkdtempSync, readFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'

const root = fileURLToPath(new URL('../../', import.meta.url))
test('public packages include licensing, portable marketing and both version-pinned initializers', () => {
  const bins = mkdtempSync(join(tmpdir(), 'brainllm-pack-'))
  try {
    const out = execFileSync(process.execPath, ['npm/pack.mjs', '--dry-run', '--bins', bins], { cwd: root, encoding: 'utf8', timeout: 120000 })
    assert.match(out, /skipped \(no binary here/)
    const packages = ['brainllm', 'create-brainllm', 'create-brainll', 'brainllm-runtime']
    for (const name of packages) {
      const dir = join(root, 'npm/dist/stage', name)
      const pkg = JSON.parse(readFileSync(join(dir, 'package.json')))
      assert.equal(pkg.name, `@christianirack/${name}`)
      assert.equal(pkg.license, 'MIT')
      assert.equal(pkg.publishConfig.access, 'public')
      assert.equal(pkg.publishConfig.registry, 'https://registry.npmjs.org/')
      assert.notEqual(pkg.private, true)
      const [pack] = JSON.parse(execFileSync('npm', ['pack', '--dry-run', '--json'], { cwd: dir, encoding: 'utf8', timeout: 30000 }))
      const paths = pack.files.map(f => f.path)
      assert.ok(paths.includes('LICENSE'))
      assert.ok(paths.includes('README.md'))
      assert.ok(!paths.some(p => /(^|\/)(\.env|\.npmrc|\.brain)(\/|$)/.test(p)))
      if (name.startsWith('create-')) {
        assert.equal(pkg.dependencies['@christianirack/brainllm'], pkg.version)
        assert.equal(pkg.bin[name], 'bin/create.js')
      }
      if (name !== 'brainllm-runtime') {
        const readme = readFileSync(join(dir, 'README.md'), 'utf8')
        assert.match(readme, /The web compiler for LLMs/)
        assert.match(readme, /https:\/\/raw.githubusercontent.com\/chrisifm\/BRAINLLM\/main\/docs\/site\/assets\/logo.png/)
        assert.ok(!readme.includes('](SPEC.md)'))
      }
    }
    assert.throws(() => execFileSync(process.execPath, ['npm/pack.mjs', '--dry-run', '--bins', bins, '--require-all'], { cwd: root, stdio: 'pipe', timeout: 30000 }), /Command failed/)
  } finally {
    rmSync(bins, { recursive: true, force: true })
  }
})
