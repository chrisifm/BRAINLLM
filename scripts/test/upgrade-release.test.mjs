import test from 'node:test'
import assert from 'node:assert/strict'
import { mkdtempSync, readFileSync, writeFileSync, mkdirSync, readdirSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join, relative, resolve } from 'node:path'
import { spawnSync } from 'node:child_process'
import { createHash } from 'node:crypto'

let expires = Infinity
const previous = process.env.BRAIN_PREVIOUS_BINARY
const current = process.env.BRAIN_CURRENT_BINARY
const digest = bytes => createHash('sha256').update(bytes).digest('hex')
const json = path => JSON.parse(readFileSync(path, 'utf8'))
const writeJson = (path, value) => writeFileSync(path, JSON.stringify(value, null, 2) + '\n')
function invoke(binary, args, cwd, failureExpected = false) {
  const remaining = expires - Date.now()
  if (remaining <= 0) throw Error('released-app proof deadline')
  const result = spawnSync(binary, args, { cwd, encoding: 'utf8', timeout: Math.min(120000, remaining), killSignal: 'SIGKILL', maxBuffer: 4 * 1024 * 1024, detached: process.platform !== 'win32' })
  if ((result.error || result.signal) && result.pid && process.platform !== 'win32') {
    try { process.kill(-result.pid, 'SIGKILL') } catch {}
  }
  // Do not include arguments, source bodies, lease tokens or private environment in failures.
  if (result.error || result.signal || (!failureExpected && result.status !== 0)) {
    let codes = []
    try { const output = JSON.parse(result.stdout); codes = (Array.isArray(output) ? output : output.diagnostics || []).slice(0, 12).map(d => d.code) } catch {}
    throw Error(`${args[0]} failed: status=${result.status}, signal=${result.signal || 'none'}, diagnosticCodes=${codes.join(',')}`)
  }
  return result
}
function snapshot(root, at = root, output = new Map()) {
  for (const entry of readdirSync(at, { withFileTypes: true })) {
    const path = join(at, entry.name), rel = relative(root, path)
    if (entry.name === '.brain' || entry.name === 'dist') continue
    assert.ok(!entry.isSymbolicLink(), 'owned fixture must contain regular files/directories')
    if (entry.isDirectory()) snapshot(root, path, output)
    else if (entry.isFile()) output.set(rel, digest(readFileSync(path)))
  }
  return output
}

test('released 0.3.1 app adopts design and upgrades package pin without source/data loss', {
  skip: !previous || !current ? 'requires explicitly verified previous release and current compiler binaries' : false,
  timeout: 360000
}, () => {
  expires = Date.now() + 300000
  const scratch = mkdtempSync(join(tmpdir(), 'brain-upgrade-release-'))
  const app = join(scratch, 'legacy-proof')
  let token, leasePaths
  const owner = 'upgrade-release-proof'
  const cli = (args, failureExpected = false) => invoke(resolve(current), args, app, failureExpected)
  const query = args => JSON.parse(cli(args).stdout)
  const changes = (op, input, failureExpected = false) => cli(['changes', op, JSON.stringify(input), '--json'], failureExpected)
  const apply = operations => {
    const plan = JSON.parse(changes('plan', { owner, token, operations }).stdout)
    const result = JSON.parse(changes('apply', { owner, token, planId: plan.planId }).stdout)
    assert.equal(result.state, 'committed')
  }
  try {
    assert.equal(invoke(resolve(previous), ['--version'], scratch).stdout.trim(), 'brain 0.3.1')
    const currentVersion = invoke(resolve(current), ['--version'], scratch).stdout.trim()
    invoke(resolve(previous), ['new', 'legacy-proof'], scratch)
    assert.equal(json(join(app, 'package.json')).devDependencies['@christianirack/brainllm'], '0.3.1')
    const config = json(join(app, 'brain.json'))
    config.migrationSentinel = { nested: ['preserve', 1, true] }
    writeJson(join(app, 'brain.json'), config)
    const originalPackage = json(join(app, 'package.json'))
    originalPackage.migrationSentinel = { nested: ['preserve', 2, false] }
    writeJson(join(app, 'package.json'), originalPackage)
    const tokenPath = join(app, 'tokens.cssbrain')
    writeFileSync(tokenPath, readFileSync(tokenPath, 'utf8').replace('--font:', '--font: "Migration Proof Sans",'))
    writeFileSync(join(app, 'content/local-data.json'), '{"rows":[{"id":1,"label":"preserve local data"}]}\n')
    mkdirSync(join(app, 'public/fonts'))
    // Opaque asset preservation sentinel; this test makes no font-rendering claim.
    writeFileSync(join(app, 'public/fonts/custom-font.woff2'), 'wOF2 migration-owned byte sentinel')
    const before = snapshot(app)
    invoke(resolve(previous), ['check', '--json'], app)
    cli(['check', '--json'])

    const adoption = query(['design', 'init', '--request', '{}'])
    assert.equal(adoption.dryRun, true)
    assert.equal(adoption.operations.length, 1)
    leasePaths = ['design.brain.json', 'package.json']
    token = JSON.parse(changes('lease', { owner, paths: leasePaths, ttlSeconds: 900 }).stdout).token
    assert.ok(token)
    apply(adoption.operations)
    const baseline = json(join(app, 'design.brain.json'))
    baseline.projectExtension = { userOwned: true, categories: ['preserve'] }
    writeJson(join(app, 'design.brain.json'), baseline)
    const readopt = query(['design', 'init', '--request', '{}'])
    if (readopt.operations.length) apply(readopt.operations)
    assert.deepEqual(json(join(app, 'design.brain.json')).projectExtension, baseline.projectExtension)

    const upgrade = query(['upgrade', '0.4.0', '--json'])
    assert.equal(upgrade.targetVersion, '0.4.0')
    assert.equal(upgrade.installation.executed, false)
    assert.equal(upgrade.registryVerified, false)
    assert.deepEqual(upgrade.sourceMigrations, [])
    const concurrentPackage = json(join(app, 'package.json'))
    concurrentPackage.concurrentSentinel = { preserve: true }
    writeJson(join(app, 'package.json'), concurrentPackage)
    const conflict = changes('plan', { owner, token, operations: upgrade.operations }, true)
    assert.notEqual(conflict.status, 0, 'stale package revision must be rejected')
    assert.deepEqual(json(join(app, 'package.json')), concurrentPackage)
    const fresh = query(['upgrade', '0.4.0', '--json'])
    apply(fresh.operations)
    const expectedPackage = structuredClone(concurrentPackage)
    expectedPackage.devDependencies['@christianirack/brainllm'] = '0.4.0'
    assert.deepEqual(json(join(app, 'package.json')), expectedPackage)
    assert.deepEqual(json(join(app, 'brain.json')), config)
    const preserved = () => {
      const after = snapshot(app)
      for (const [file, hash] of before) {
        if (file !== 'package.json') assert.equal(after.get(file), hash, `pre-existing file changed: ${file}`)
      }
    }
    preserved()
    console.log(JSON.stringify({ phase: 'adoption-preservation', originalFilesPreserved: before.size - 1,
      tokenAndFontBytesPreserved: true, dataPreserved: true, unknownFieldsPreserved: true,
      staleCASRejected: true, targetInstalled: false, buildVerified: false }))
    cli(['check', '--json'])
    cli(['build', '--json'])
    preserved()
    // No npm install and no execution of the unpublished target package occurs.
    console.log(JSON.stringify({ previousCompiler: '0.3.1', currentCompiler: currentVersion,
      originalFilesPreserved: before.size - 1, designAdopted: true, unknownDesignFieldsPreserved: true,
      unknownConfigurationPreserved: true, tokenAndFontBytesPreserved: true, dataPreserved: true,
      staleCASRejected: true, currentCheckAndBuild: true, packagePinApplied: '0.4.0', targetInstalled: false, published: false }))
  } finally {
    try {
      if (token && leasePaths) changes('lease', { action: 'release', owner, token, paths: leasePaths }, true)
    } catch {} finally {
      rmSync(scratch, { recursive: true, force: true })
    }
  }
})
