// Claude Code PostToolUse adapter. Read tool JSON, never execute tool-provided command text.
import { readFileSync, existsSync, realpathSync } from 'node:fs'
import { resolve, relative, sep, join } from 'node:path'
import { spawnSync } from 'node:child_process'

const emit = text => process.stdout.write(JSON.stringify({ hookSpecificOutput: { hookEventName: 'PostToolUse', additionalContext: text } }) + '\n')
let input
try {
  const raw = readFileSync(0, 'utf8')
  if (raw.length > 1024 * 1024) throw new Error('input limit')
  input = JSON.parse(raw)
} catch { process.exit(0) }
if (!['Write', 'Edit', 'MultiEdit'].includes(input.tool_name)) process.exit(0)
const root = realpathSync(process.env.CLAUDE_PROJECT_DIR || process.cwd())
const path = input.tool_input?.file_path
if (typeof path !== 'string' || !/\.(?:brain|jsbrain|cssbrain)$/.test(path)) process.exit(0)
let target
try {
  target = realpathSync(resolve(input.cwd || root, path))
  const rel = relative(root, target)
  if (rel === '..' || rel.startsWith('..' + sep) || resolve(root, rel) !== target) process.exit(0)
} catch { process.exit(0) }
// The hook does not download binaries or delegate to an unrelated global installation.
const local = join(root, 'node_modules', '.bin', process.platform === 'win32' ? 'brain.cmd' : 'brain')
if (!existsSync(local)) {
  emit('BRAINLLM edit feedback unavailable: install the pinned project dependency with npm install, or run brain check --json manually for a source installation.')
  process.exit(0)
}
// Whole-project check: a type, module or token edit can break its consumers. Tests are intentionally excluded.
// On Windows use the package's Node entry point rather than invoking a .cmd through a shell.
let command = local, args = ['check', '--json']
if (process.platform === 'win32') {
  const pkg = JSON.parse(readFileSync(join(root, 'node_modules', '@christianirack', 'brainllm', 'package.json'), 'utf8'))
  const entry = typeof pkg.bin === 'string' ? pkg.bin : pkg.bin?.brain
  if (!entry) { emit('BRAINLLM edit feedback unavailable: package has no brain entry point.'); process.exit(0) }
  command = process.execPath
  args = [join(root, 'node_modules', '@christianirack', 'brainllm', entry), ...args]
}
const result = spawnSync(command, args, { cwd: root, encoding: 'utf8', timeout: 15000, maxBuffer: 256 * 1024, windowsHide: true, shell: false })
if (result.error) { emit('BRAINLLM edit feedback failed or exceeded 15 seconds / 256 KB; run brain check --json manually.'); process.exit(0) }
let diagnostics
try { diagnostics = JSON.parse(result.stdout) } catch {
  emit('BRAINLLM check returned no valid diagnostic JSON; run brain check --json manually.')
  process.exit(0)
}
// Compiler diagnostics contain source snippets; report only check output, never environment values or tool input.
let report = JSON.stringify(diagnostics)
if (report.length > 24000) report = report.slice(0, 24000) + '\n[truncated; run brain check --json for all findings]'
emit('BRAINLLM edit feedback (check only, no tests). Exit ' + result.status + ':\n' + report)
