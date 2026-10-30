import { test } from 'node:test'
import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
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
      for (const keyword of ['rust', 'llm', 'frontend', 'front-end', 'backend', 'back-end', 'cli', 'framework', 'testing', 'design-system']) {
        assert.ok(pkg.keywords.includes(keyword), `${name} must expose its relevant discovery keywords`)
      }
      const [pack] = JSON.parse(execFileSync('npm', ['pack', '--dry-run', '--json'], { cwd: dir, encoding: 'utf8', timeout: 30000 }))
      const paths = pack.files.map(f => f.path)
      assert.ok(paths.includes('LICENSE'))
      assert.ok(paths.includes('THIRD_PARTY_NOTICES.txt'))
      assert.match(readFileSync(join(dir, 'THIRD_PARTY_NOTICES.txt'), 'utf8'), /Apache License/)
      assert.ok(paths.includes('README.md'))
      if (name === 'brainllm-runtime') {
        assert.ok(paths.includes('brain.js'), 'runtime package must retain its executable modules')
        assert.ok(paths.includes('brain-form.js'))
        assert.ok(!paths.some(path => path.startsWith('test/')))
        const [archive] = JSON.parse(execFileSync('npm', ['pack', '--json', '--ignore-scripts', '--pack-destination', bins], { cwd: dir, encoding: 'utf8', timeout: 30000 }))
        const tarball = join(bins, archive.filename)
        const members = execFileSync('tar', ['-tzf', tarball], { encoding: 'utf8', timeout: 10000 }).split('\n')
        assert.ok(members.includes('package/brain.js'))
        assert.ok(members.includes('package/brain-form.js'))
        assert.ok(!members.some(path => path.startsWith('package/test/')))
        const digest = bytes => createHash('sha256').update(bytes).digest('hex')
        const actual = execFileSync('tar', ['-xOf', tarball, 'package/brain.js'], { timeout: 10000, maxBuffer: 4 * 1024 * 1024 })
        assert.equal(digest(actual), digest(readFileSync(join(dir, 'brain.js'))), 'actual runtime tarball preserves the staged module bytes')
      }
      assert.ok(!paths.some(p => /(^|\/)(\.env|\.npmrc|\.brain)(\/|$)/.test(p)))
      if (name.startsWith('create-')) {
        assert.equal(pkg.dependencies['@christianirack/brainllm'], pkg.version)
        assert.equal(pkg.bin[name], 'bin/create.js')
      }
      if (name !== 'brainllm-runtime') {
        const readme = readFileSync(join(dir, 'README.md'), 'utf8')
        assert.match(readme, /The web compiler for LLMs/)
        assert.ok(readme.includes(`https://raw.githubusercontent.com/chrisifm/BRAINLLM/v${pkg.version}/docs/site/assets/logo.png`))
        assert.ok(!readme.includes('](SPEC.md)'))
        assert.ok(readme.includes(`https://github.com/chrisifm/BRAINLLM/blob/v${pkg.version}/SPEC.md`))
      }
    }
    assert.throws(() => execFileSync(process.execPath, ['npm/pack.mjs', '--dry-run', '--bins', bins, '--require-all'], { cwd: root, stdio: 'pipe', timeout: 30000 }), /Command failed/)
  } finally {
    rmSync(bins, { recursive: true, force: true })
  }
})
