// Disposable test Redis only. Never reads REDIS_URL or connects to an existing instance.
import { spawn, spawnSync } from 'node:child_process'
import net from 'node:net'
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

export const redisAvailable = spawnSync('redis-server', ['--version'], { encoding: 'utf8', timeout: 5000 }).status === 0
export async function ownedRedis(t) {
  const directory = mkdtempSync(join(tmpdir(), 'brain-owned-redis-'))
  const reserved = net.createServer()
  await new Promise(ok => reserved.listen(0, '127.0.0.1', ok))
  const port = reserved.address().port
  await new Promise(ok => reserved.close(ok))
  const child = spawn('redis-server', ['--bind', '127.0.0.1', '--port', String(port), '--protected-mode', 'yes', '--save', '', '--appendonly', 'no', '--dir', directory, '--maxmemory', '8mb', '--maxmemory-policy', 'noeviction', '--requirepass', 'synthetic-cache-password'], { stdio: ['ignore', 'pipe', 'pipe'] })
  let output = ''
  const exited = new Promise(ok => { child.once('exit', (code, signal) => ok({ code, signal })); child.once('error', () => ok({ code: null, signal: null })) })
  const stop = async () => {
    if (child.pid && child.exitCode === null && child.signalCode === null) {
      const force = setTimeout(() => child.kill('SIGKILL'), 3000)
      child.kill('SIGTERM'); const status = await exited; clearTimeout(force)
      if (status.code !== 0 || status.signal !== null) throw Error('Owned Redis did not stop gracefully')
    }
    rmSync(directory, { recursive: true, force: true })
  }
  t.after(stop)
  await new Promise((ok, no) => {
    const timer = setTimeout(() => no(Error('Owned Redis did not become ready')), 5000)
    const keep = data => { output = (output + data).slice(-4096); if (output.includes('Ready to accept connections')) { clearTimeout(timer); ok() } }
    child.stdout.on('data', keep); child.stderr.on('data', keep)
    child.once('error', error => { clearTimeout(timer); no(error) })
    exited.then(() => { clearTimeout(timer); no(Error('Owned Redis exited before readiness')) })
  })
  const cli = args => {
    const result = spawnSync('redis-cli', ['-h', '127.0.0.1', '-p', String(port), '--raw', ...args], { env: { ...process.env, REDISCLI_AUTH: 'synthetic-cache-password' }, encoding: 'utf8', timeout: 5000, maxBuffer: 1024 * 1024 })
    if (result.status !== 0) throw Error('Owned redis-cli failed')
    return result.stdout.trim()
  }
  return { url: `redis://default:synthetic-cache-password@127.0.0.1:${port}/0`, cli }
}
