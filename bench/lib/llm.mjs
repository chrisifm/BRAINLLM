// Shared LLM engines for QA benches. Same contract for both: generate(system, prompt) → { text, ms, input, output }.
// claude: Haiku 5.5 through `claude -p`, isolated (empty cwd, fixed system prompt, no tools/settings/MCP) — see phase0.md §1.
// qwen: qwen2.5-coder:7b through a local assistant script (`local-assistant.sh code`; $LOCAL_ASSISTANT, else resolved
//       from PATH), which has a 120 s curl timeout: callers must keep each request small (one file per call).
// Scratch cwd for claude: $QA_LLM_CWD, else <os tmpdir>/brainllm-phase0.
import { spawn } from 'node:child_process'
import { mkdirSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

export const CLAUDE_MODEL = 'claude-haiku-5-5'
export const QWEN_MODEL = 'qwen2.5-coder:7b'
const CWD = process.env.QA_LLM_CWD ?? join(tmpdir(), 'brainllm-phase0')
mkdirSync(CWD, { recursive: true })
const LOCAL = process.env.LOCAL_ASSISTANT ?? 'local-assistant.sh'

function claude(system, prompt) {
  return new Promise((resolve, reject) => {
    const p = spawn('claude', ['-p', '--model', CLAUDE_MODEL, '--output-format', 'json', '--system-prompt', system,
      '--tools', '', '--setting-sources', '', '--strict-mcp-config', '--no-session-persistence', prompt],
      { cwd: CWD, stdio: ['ignore', 'pipe', 'pipe'] })
    let out = '', err = ''
    p.stdout.on('data', (d) => (out += d))
    p.stderr.on('data', (d) => (err += d))
    p.on('close', (code) => {
      try {
        const j = JSON.parse(out)
        if (j.is_error) return reject(new Error(j.result || 'claude error'))
        const u = j.usage
        resolve({ text: j.result, ms: j.duration_ms, cost: j.total_cost_usd,
          input: u.input_tokens + u.cache_creation_input_tokens + u.cache_read_input_tokens,
          output: u.output_tokens - (u.output_tokens_details?.thinking_tokens ?? 0) })
      } catch { reject(new Error(`claude exit ${code}: ${(err || out).slice(0, 300)}`)) }
    })
  })
}

function qwen(system, prompt) {
  return new Promise((resolve, reject) => {
    const t0 = Date.now()
    const p = spawn(LOCAL, ['code', `${system}\n\n${prompt}`], { stdio: ['ignore', 'pipe', 'pipe'] })
    let out = ''
    p.stdout.on('data', (d) => (out += d))
    p.on('close', () => {
      if (!out.trim() || out.startsWith('ERROR')) return reject(new Error(`qwen: ${out.slice(0, 200) || 'empty'}`))
      resolve({ text: out, ms: Date.now() - t0, cost: 0, input: null, output: null })
    })
  })
}

export const engines = { claude, qwen }
export const modelOf = (engine) => (engine === 'qwen' ? QWEN_MODEL : CLAUDE_MODEL)

export function extractCode(text) {
  const m = text.match(/```(?:html|brain|svelte|css)?[^\n]*\n([\s\S]*?)```/)
  return (m ? m[1] : text).trim() + '\n'
}
