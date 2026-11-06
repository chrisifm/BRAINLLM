// A production-only npm install must retain the real compiler used by build/start scripts.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { mkdtempSync, mkdirSync, copyFileSync, chmodSync, writeFileSync, readFileSync, existsSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
const brain = resolve(process.env.BRAIN_BIN ?? new URL('../../target/debug/brain', import.meta.url).pathname)
const dependency = '@christianirack/brainllm'

test('production npm ci retains the compiler for app and static starters', { timeout: 120000, skip: process.platform === 'win32' }, () => {
  const root = mkdtempSync(join(tmpdir(), 'brain-production-install-'))
  const env = { ...process.env, NODE_ENV: 'production', BRAIN_NO_DELEGATE: '1', npm_config_cache: join(root, 'npm-cache'), npm_config_audit: 'false', npm_config_fund: 'false' }
  const run = (command, args, cwd) => {
    const output = spawnSync(command, args, { cwd, env, encoding: 'utf8', timeout: 30000, maxBuffer: 1024 * 1024 })
    assert.equal(output.status, 0, `${command} ${args.join(' ')}: ${output.stderr || output.stdout}; error=${output.error?.code ?? 'none'}; signal=${output.signal ?? 'none'}`)
    return output.stdout
  }
  try {
    const version = run(brain, ['--version'], root).trim().replace(/^brain /, '')
    // Package the actual local native executable, without a registry, SDK, installer script or substitute compiler.
    const local = join(root, 'compiler')
    mkdirSync(join(local, 'bin'), { recursive: true })
    copyFileSync(brain, join(local, 'bin/brain'))
    chmodSync(join(local, 'bin/brain'), 0o755)
    writeFileSync(join(local, 'package.json'), JSON.stringify({ name: dependency, version, bin: { brain: 'bin/brain' }, files: ['bin/brain'] }))
    const packed = JSON.parse(run('npm', ['pack', '--json', '--ignore-scripts', '--offline', '--pack-destination', root], local))
    const tarball = join(root, packed[0].filename)
    for (const mode of ['app', 'static']) {
      run(brain, ['new', mode, ...(mode === 'static' ? ['--static'] : []), '--json'], root)
      const app = join(root, mode), file = join(app, 'package.json'), pkg = JSON.parse(readFileSync(file, 'utf8'))
      const section = pkg.dependencies?.[dependency] ? 'dependencies' : 'devDependencies'
      assert.equal(pkg[section][dependency], version, 'the generated dependency remains exactly version-pinned')
      pkg[section][dependency] = `file:${tarball}`
      writeFileSync(file, JSON.stringify(pkg))
      run('npm', ['install', '--package-lock-only', '--ignore-scripts', '--offline'], app)
      run('npm', ['ci', '--omit=dev', '--ignore-scripts', '--offline'], app)
      assert.ok(existsSync(join(app, 'node_modules/.bin/brain')), `${mode}: production install omitted the compiler required by npm scripts`)
      run('npm', ['run', 'start', '--', '--help'], app)
      if (mode === 'static') {
        run('npm', ['run', 'build', '--', '--static', '--site', 'https://example.test', '--json'], app)
        assert.ok(readFileSync(join(app, 'dist/index.html'), 'utf8').includes('<!-- by BRAINLLM https://chrisifm.github.io/BRAINLLM/ -->'))
      }
    }
  } finally { rmSync(root, { recursive: true, force: true }) }
})
