// Bounded child lifecycle for local maintenance tools. No shell interpolation, no global process cleanup.
import { spawn, execFileSync } from 'node:child_process'
import { openSync, writeSync, closeSync } from 'node:fs'

export function git(cwd, ...args) {
  try { return execFileSync('git', args, { cwd, encoding: 'utf8', timeout: 30_000, maxBuffer: 32 * 1024 * 1024, stdio: ['ignore', 'pipe', 'pipe'] }).trimEnd() }
  catch { throw new Error(`Git operation failed: ${args[0]}; inspect repository state locally.`) }
}

export function run(command, args, { cwd, log, timeout = 120_000, env = process.env, maxBytes = 4 * 1024 * 1024 } = {}) {
  return new Promise((resolve, reject) => {
    const fd = log ? openSync(log, 'w', 0o600) : null
    const child = spawn(command, args, { cwd, env, detached: process.platform !== 'win32', stdio: ['ignore', 'pipe', 'pipe'] })
    let bytes = 0, reason = '', escalation
    const kill = signal => {
      try { process.platform === 'win32' ? child.kill(signal) : process.kill(-child.pid, signal) } catch {}
    }
    const stop = why => {
      if (reason) return
      reason = why
      kill('SIGTERM')
      escalation = setTimeout(() => kill('SIGKILL'), 1000)
    }
    const cancel = () => stop('cancelled')
    const timer = setTimeout(() => stop('deadline exceeded'), timeout)
    process.once('SIGINT', cancel)
    process.once('SIGTERM', cancel)
    const output = chunk => {
      if (fd !== null && bytes < maxBytes) writeSync(fd, chunk.subarray(0, maxBytes - bytes))
      bytes += chunk.length // Continue draining; never retain unbounded output.
    }
    child.stdout.on('data', output)
    child.stderr.on('data', output)
    child.once('error', () => { reason = 'could not start process' })
    child.once('exit', () => kill('SIGKILL')) // Clean descendants even when the parent exits first.
    child.once('close', code => {
      clearTimeout(timer)
      clearTimeout(escalation)
      process.removeListener('SIGINT', cancel)
      process.removeListener('SIGTERM', cancel)
      if (fd !== null) closeSync(fd)
      if (reason || code !== 0) reject(new Error(`${command}: ${reason || `exit ${code}`}; inspect the private local log.`))
      else resolve({ code, outputBytes: bytes, truncated: bytes > maxBytes })
    })
  })
}
