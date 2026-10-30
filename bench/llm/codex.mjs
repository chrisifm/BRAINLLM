// Official Codex CLI adapter. Authentication stays in the CLI's existing store.
import { spawn } from 'node:child_process'
import { mkdtemp, open, rm } from 'node:fs/promises'
import { constants } from 'node:fs'
import { join } from 'node:path'
import { tmpdir } from 'node:os'

const OUTPUT_MAX = 2 * 1024 * 1024
const STDERR_MAX = 64 * 1024
const EFFORTS = new Set(['none', 'minimal', 'low', 'medium', 'high', 'xhigh', 'max', 'ultra'])
let cancelled = false

export function codexPins(model, effort) {
  if (typeof model !== 'string' || !/^[A-Za-z0-9][A-Za-z0-9._:/-]{0,127}$/.test(model))
    throw new Error('codex: --model must be explicit and contain a valid model identifier')
  if (!EFFORTS.has(effort)) throw new Error('codex: --effort must be explicit and supported by the selected model')
  return { model, effort }
}

// deadlineMs is injectable for local transport tests; production never exceeds 180 seconds.
export async function codexGenerate(system, prompt, { model, effort, bin = process.env.BRAIN_CODEX_BIN || 'codex', binArgs = [], deadlineMs = 180000, scratch = tmpdir() } = {}) {
  codexPins(model, effort)
  if (cancelled) throw new Error('codex: benchmark cancelled')
  if (typeof bin !== 'string' || !bin || bin.length > 4096 || /[\x00-\x1f]/.test(bin)) throw new Error('codex: invalid executable path')
  if (!Array.isArray(binArgs) || binArgs.length > 8 || binArgs.some((a) => typeof a !== 'string' || a.length > 4096 || /[\x00-\x1f]/.test(a)) || Buffer.byteLength(JSON.stringify(binArgs)) > 8192) throw new Error('codex: invalid executable argument prefix')
  if (!Number.isInteger(deadlineMs) || deadlineMs < 1 || deadlineMs > 180000) throw new Error('codex: invalid deadline')
  const input = `${system}\n\nThis is a source-generation benchmark. Use only the supplied specification and request. Do not call tools, read files, execute commands, browse, or delegate. Return the requested file content only.\n\n${prompt}`
  if (Buffer.byteLength(input) > OUTPUT_MAX) throw new Error('codex: prompt exceeds 2 MiB')
  const work = await mkdtemp(join(scratch, 'brain-codex-'))
  const last = join(work, 'last-message.txt')
  try {
    const usage = await new Promise((resolve, reject) => {
      const args = ['exec', '--model', model, '-c', `model_reasoning_effort="${effort}"`,
        '-c', 'project_doc_max_bytes=0', '-c', 'features.shell_tool=false', '-c', 'web_search="disabled"',
        '--ignore-user-config', '--ignore-rules', '--sandbox', 'read-only', '--skip-git-repo-check',
        '--ephemeral', '--json', '--output-last-message', last, '-']
      const child = spawn(bin, [...binArgs, ...args], { cwd: work, detached: process.platform !== 'win32', stdio: ['pipe', 'pipe', 'pipe'] })
      let outBytes = 0, errBytes = 0, pending = '', completed = null, failure = null, exited = false
      let escalation
      const signal = (name) => {
        if (!child.pid) return
        try { process.platform === 'win32' ? child.kill(name) : process.kill(-child.pid, name) } catch (e) { if (e.code !== 'ESRCH') failure ||= 'codex: process cleanup failed' }
      }
      const stop = (message) => {
        if (failure) return
        failure = message
        signal('SIGTERM')
        escalation = setTimeout(() => { signal('SIGKILL'); if (exited) { child.stdout.destroy(); child.stderr.destroy() } }, 150)
      }
      const timer = setTimeout(() => stop('codex: generation deadline exceeded'), deadlineMs)
      const cancel = () => { cancelled = true; stop('codex: benchmark cancelled') }
      process.on('SIGINT', cancel); process.on('SIGTERM', cancel)
      const line = (raw) => {
        if (!raw.trim() || failure) return
        let event
        try { event = JSON.parse(raw) } catch { stop('codex: malformed JSON event'); return }
        if (!event || typeof event !== 'object' || typeof event.type !== 'string') { stop('codex: invalid event'); return }
        // Never persist transcripts, reasoning, thread IDs, tool arguments or CLI stderr.
        if (event.type === 'error' || event.type === 'turn.failed') { stop('codex: generation failed'); return }
        if (event.type?.startsWith('item.') && event.item && !['agent_message', 'reasoning'].includes(event.item.type)) {
          stop('codex: tools are forbidden in source-generation runs'); return
        }
        if (event.type === 'turn.completed') {
          const u = event.usage
          if (completed || !u || !Number.isSafeInteger(u.input_tokens) || u.input_tokens < 0 || !Number.isSafeInteger(u.output_tokens) || u.output_tokens < 0 ||
              (u.cached_input_tokens !== undefined && (!Number.isSafeInteger(u.cached_input_tokens) || u.cached_input_tokens < 0 || u.cached_input_tokens > u.input_tokens))) {
            stop('codex: invalid turn usage'); return
          }
          completed = { input: u.input_tokens, output: u.output_tokens, cachedInput: u.cached_input_tokens ?? 0 }
        }
      }
      child.stdout.setEncoding('utf8')
      child.stdout.on('data', (data) => {
        outBytes += Buffer.byteLength(data)
        if (outBytes > OUTPUT_MAX) { stop('codex: stdout exceeds 2 MiB'); return }
        if (failure) return
        pending += data
        let end
        while ((end = pending.indexOf('\n')) !== -1) { line(pending.slice(0, end)); pending = pending.slice(end + 1) }
      })
      child.stderr.on('data', (data) => { errBytes += data.length; if (errBytes > STDERR_MAX) stop('codex: stderr exceeds 64 KiB') })
      child.stdin.on('error', () => { stop('codex: stdin failed') })
      child.on('error', () => { failure ||= 'codex: executable could not start' })
      child.on('exit', () => { exited = true })
      child.on('close', (code) => {
        clearTimeout(timer); clearTimeout(escalation)
        process.removeListener('SIGINT', cancel); process.removeListener('SIGTERM', cancel)
        // Also terminate owned descendants that close their inherited pipes before the parent exits.
        signal('SIGKILL')
        if (pending && !failure) line(pending)
        clearTimeout(escalation)
        if (failure || code !== 0 || !completed) reject(new Error(failure || 'codex: no successful completed turn'))
        else resolve(completed)
      })
      child.stdin.end(input)
    })
    let file
    try {
      file = await open(last, constants.O_RDONLY | constants.O_NOFOLLOW | constants.O_NONBLOCK)
      const stat = await file.stat()
      if (!stat.isFile() || stat.size > OUTPUT_MAX) throw new Error('bounds')
      // Fixed-cap read also handles a concurrent writer growing the file after stat.
      const buf = Buffer.alloc(OUTPUT_MAX + 1)
      let size = 0
      while (size < buf.length) { const { bytesRead } = await file.read(buf, size, buf.length - size, size); if (!bytesRead) break; size += bytesRead }
      if (size > OUTPUT_MAX) throw new Error('bounds')
      const text = buf.subarray(0, size).toString('utf8')
      if (!text.trim()) throw new Error('empty')
      return { text, ...usage, cost: null }
    } catch { throw new Error('codex: missing, unsafe, empty or oversized last message') }
    finally { await file?.close() }
  } finally { await rm(work, { recursive: true, force: true }) }
}
