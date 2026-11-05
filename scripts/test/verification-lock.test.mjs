// Exercise real kernel locks and exec inheritance; no compiler or full verification is launched here.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { spawn, spawnSync } from 'node:child_process'
import { mkdtempSync, existsSync, rmSync, realpathSync, mkdirSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { once } from 'node:events'
const helper = new URL('../verify-lock.py', import.meta.url).pathname
const python = process.env.BRAIN_PYTHON ?? 'python3'
const supported = process.platform !== 'win32'
const run = (target, command = [process.execPath, '-e', ''], env = {}) => spawnSync(python, [helper, '--target', target, '--', ...command], { env: { ...process.env, ...env }, encoding: 'utf8', timeout: 7000, maxBuffer: 128 * 1024 })
async function exit(child) {
  if (child.exitCode !== null || child.signalCode !== null) return
  let timer
  try { await Promise.race([once(child, 'exit'), new Promise((_, reject) => { timer = setTimeout(() => reject(Error('owned command did not exit')), 7000) })]) }
  finally { clearTimeout(timer) }
}
async function start(target, descendant = false) {
  const code = descendant
    ? `const {spawn}=require('node:child_process');const child=spawn(process.execPath,['-e','console.log(process.pid);setInterval(()=>{},100)'],{stdio:[0,'pipe','ignore']});child.stdout.once('data',data=>console.log(JSON.stringify({pid:process.pid,descendant:Number(String(data).trim())})));setInterval(()=>{},100)`
    : `const {spawnSync}=require('node:child_process');const result=spawnSync(${JSON.stringify(python)},[${JSON.stringify(helper)},'--target',process.env.CARGO_TARGET_DIR,'--check-held'],{stdio:[0,'pipe','pipe']});console.log(JSON.stringify({pid:process.pid,held:result.status===0,target:process.env.CARGO_TARGET_DIR}));setInterval(()=>{},100)`
  const child = spawn(python, [helper, '--target', target, '--', process.execPath, '-e', code], { stdio: ['ignore', 'pipe', 'pipe'] })
  let output = '', errors = '', timer
  try {
    const info = await new Promise((resolve, reject) => {
      timer = setTimeout(() => reject(Error('owned command did not start: ' + errors)), 7000)
      child.on('error', reject)
      child.stderr.on('data', bytes => { errors = (errors + bytes).slice(-4096) })
      child.stdout.on('data', bytes => {
        output += bytes
        if (output.length > 4096) reject(Error('oversized readiness'))
        else if (output.includes('\n')) { try { resolve(JSON.parse(output.split('\n')[0])) } catch (e) { reject(e) } }
      })
      child.once('exit', () => reject(Error('owned command exited before readiness: ' + errors)))
    })
    assert.equal(info.pid, child.pid, 'the helper execs the command instead of creating a supervisor')
    return { child, ...info }
  } catch (error) { child.kill('SIGTERM'); await exit(child).catch(() => {}); throw error }
  finally { clearTimeout(timer) }
}
async function cleanup(owner) {
  if (!owner) return
  if (owner.descendant && !owner.descendantStopped) {
    try { process.kill(owner.descendant, 'SIGKILL') } catch (e) { if (e.code !== 'ESRCH') throw e }
    owner.descendantStopped = true
  }
  if (owner.child.exitCode === null && owner.child.signalCode === null) owner.child.kill('SIGTERM')
  await exit(owner.child).catch(() => {})
}

test('verification locks serialize one target, allow independent targets and release after errors', { skip: !supported, timeout: 20000 }, async () => {
  const root = mkdtempSync(join(tmpdir(), 'brain-verify-lock-'))
  let owner
  try {
    const target = join(root, 'target')
    owner = await start(target)
    assert.equal(owner.held, true, 'the command inherited the actual locked description')
    assert.equal(owner.target, realpathSync(target))
    const marker = join(root, 'contender-ran')
    const busy = run(target, [process.execPath, '-e', `require('node:fs').writeFileSync(${JSON.stringify(marker)},'unexpected')`], { BRAIN_VERIFY_LOCK_HELD: '1' })
    assert.equal(busy.status, 75, busy.stderr)
    assert.equal(existsSync(marker), false)
    const forged = spawnSync(python, [helper, '--target', target, '--check-held'], { encoding: 'utf8', timeout: 7000, env: { ...process.env, BRAIN_VERIFY_LOCK_HELD: '1' } })
    assert.equal(forged.status, 1, 'an environment marker is not proof of descriptor ownership')
    assert.equal(run(join(root, 'independent')).status, 0)
    owner.child.kill('SIGTERM'); await exit(owner.child)
    assert.equal(owner.child.signalCode, 'SIGTERM', 'command signal behavior is preserved')
    owner = undefined
    assert.equal(run(target, [process.execPath, '-e', 'process.exit(7)']).status, 7)
    assert.equal(run(target, ['brain-command-that-does-not-exist']).status, 2)
    assert.equal(run(target).status, 0, 'failed commands release their lock')
  } finally { await cleanup(owner); rmSync(root, { recursive: true, force: true }) }
})

test('the lock remains held by a command descendant after its leader is killed', { skip: !supported, timeout: 20000 }, async () => {
  const root = mkdtempSync(join(tmpdir(), 'brain-verify-inheritance-'))
  let owner
  try {
    const target = join(root, 'target')
    owner = await start(target, true)
    owner.child.kill('SIGKILL'); await exit(owner.child)
    assert.equal(run(target).status, 75, 'inherited stdin keeps the live descendant inside the lease')
    process.kill(owner.descendant, 'SIGKILL'); owner.descendantStopped = true
    // Wait on the observable lock, never signal a PID again after ending its owned lifetime.
    let released = false
    for (let i = 0; i < 20; i++) {
      const result = run(target)
      if (result.status === 0) { released = true; break }
      assert.equal(result.status, 75, result.stderr)
      await new Promise(r => setTimeout(r, 20))
    }
    assert.equal(released, true)
  } finally { await cleanup(owner); rmSync(root, { recursive: true, force: true }) }
})

test('verification discovers configured Cargo targets without compiling', { skip: !supported, timeout: 20000 }, () => {
  const root = mkdtempSync(join(tmpdir(), 'brain-verify-metadata-'))
  try {
    mkdirSync(join(root, 'src')); mkdirSync(join(root, '.cargo'))
    const target = join(root, 'configured target')
    writeFileSync(join(root, 'Cargo.toml'), '[package]\nname="verification_fixture"\nversion="0.0.0"\nedition="2024"\n')
    writeFileSync(join(root, 'src/lib.rs'), 'pub fn fixture() {}\n')
    writeFileSync(join(root, '.cargo/config.toml'), `[build]\ntarget-dir=${JSON.stringify(target)}\n`)
    const env = { ...process.env }; delete env.CARGO_TARGET_DIR
    const generated = spawnSync('cargo', ['generate-lockfile', '--offline'], { cwd: root, env, encoding: 'utf8', timeout: 7000, maxBuffer: 128 * 1024 })
    assert.equal(generated.status, 0, generated.stderr)
    const result = spawnSync(python, [helper, '--cargo-target', '--', process.execPath, '-e', 'console.log(process.env.CARGO_TARGET_DIR)'], { cwd: root, env, encoding: 'utf8', timeout: 7000, maxBuffer: 128 * 1024 })
    assert.equal(result.status, 0, result.stderr)
    assert.equal(result.stdout.trim(), realpathSync(target))
    assert.equal(existsSync(join(target, 'debug')), false, 'target discovery performs no compilation')
  } finally { rmSync(root, { recursive: true, force: true }) }
})
