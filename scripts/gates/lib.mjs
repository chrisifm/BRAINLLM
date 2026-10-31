// Shared harness of the release gates run by scripts/backend-check.mjs against its disposable Postgres cluster.
// Every process started here is owned by PID, bounded by a deadline and stopped at the end of the gate.
import { spawn, spawnSync } from 'node:child_process'
import { mkdirSync, writeFileSync, mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import net from 'node:net'

export const binary = process.env.BRAIN_BIN
export const databaseURL = process.env.BRAIN_GATE_DATABASE_URL
if (!binary || !databaseURL) throw Error('Run this gate through scripts/backend-check.mjs; it needs BRAIN_BIN and its disposable database')
export const sleep = ms => new Promise(resolve => setTimeout(resolve, ms))
export const scratch = name => mkdtempSync(join(process.env.TMPDIR || tmpdir(), `brain-gate-${name}-`))
export function write(root, path, text) {
  mkdirSync(dirname(join(root, path)), { recursive: true })
  writeFileSync(join(root, path), text)
}
export function brain(args, cwd, env = {}) {
  const result = spawnSync(binary, args, { cwd, env: { ...process.env, DATABASE_URL: databaseURL, ...env }, encoding: 'utf8', timeout: 120000, maxBuffer: 16 * 1024 * 1024, stdio: ['ignore', 'pipe', 'pipe'] })
  if (result.status !== 0) throw Error(`brain ${args.join(' ')} failed (exit ${result.status}, signal ${result.signal}): ${(result.stderr || result.stdout || result.error?.message || '').slice(0, 2000)}`)
  return result.stdout
}
export async function freePort() {
  const reservation = net.createServer()
  await new Promise((ok, no) => reservation.once('error', no).listen(0, '127.0.0.1', ok))
  const { port } = reservation.address()
  await new Promise(ok => reservation.close(ok))
  return port
}
const owned = new Set()
const alive = child => child.exitCode === null && child.signalCode === null
export function own(command, args, cwd, env = {}) {
  const output = { text: '' }
  const child = spawn(command, args, { cwd, env: { ...process.env, DATABASE_URL: databaseURL, ...env }, stdio: ['ignore', 'pipe', 'pipe'], detached: true })
  const keep = chunk => { output.text = (output.text + chunk).slice(-256 * 1024) }
  child.stdout.on('data', keep); child.stderr.on('data', keep); child.once('error', () => {})
  owned.add(child)
  return { child, output }
}
export async function stop(child) {
  if (!owned.delete(child) || !alive(child)) return
  const gone = new Promise(ok => child.once('exit', ok))
  try { process.kill(-child.pid, 'SIGTERM') } catch {}
  const timer = setTimeout(() => { try { process.kill(-child.pid, 'SIGKILL') } catch {} }, 3000)
  await gone
  clearTimeout(timer)
}
export const stopAll = () => Promise.all([...owned].map(stop))
export async function ready(url, { child, output }, ms = 30000) {
  const end = Date.now() + ms
  while (Date.now() < end) {
    if (!alive(child)) throw Error(`The server for ${url} exited: ${output.text.slice(-2000)}`)
    try {
      const response = await fetch(url, { signal: AbortSignal.timeout(1000), redirect: 'manual' })
      await response.arrayBuffer()
      return
    } catch {}
    await sleep(100)
  }
  throw Error(`The server for ${url} was not ready within its bound: ${output.text.slice(-2000)}`)
}
export function createDatabase(name) {
  const url = new URL(databaseURL)
  const psql = process.env.PG_BIN ? join(process.env.PG_BIN, 'createdb') : 'createdb'
  const made = spawnSync(psql, ['-h', url.hostname, '-p', url.port, '-U', url.username, name], { encoding: 'utf8', timeout: 20000 })
  if (made.status !== 0) throw Error('createdb failed: ' + made.stderr)
  url.pathname = '/' + name
  return url.href
}
/** Runs `body` with a deadline and always stops what it started. A failure sets a nonzero exit code, loudly. */
export async function gate(name, ms, body) {
  const directory = scratch(name)
  const timer = setTimeout(async () => { console.error(`${name}: FAIL, exceeded its ${ms} ms deadline`); await stopAll(); rmSync(directory, { recursive: true, force: true }); process.exit(1) }, ms)
  try { await body(directory); console.log(`${name}: PASS`) }
  catch (error) { console.error(`${name}: FAIL\n${error.stack || error}`); process.exitCode = 1 }
  finally { clearTimeout(timer); await stopAll(); rmSync(directory, { recursive: true, force: true }) }
}
