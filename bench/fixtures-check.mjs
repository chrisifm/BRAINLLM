// Compares `brain check --json examples/invalid` against each <case>.expected.json (code/file/line/col exact, msg/fix non-empty)
// and requires the exact documented warning boundary in the valid examples.
// Usage: node bench/fixtures-check.mjs [path to brain]
import { execFileSync } from 'node:child_process'
import { readFileSync, existsSync, readdirSync } from 'node:fs'
import { join, sep } from 'node:path'
import { fileURLToPath } from 'node:url'

const ROOT = fileURLToPath(new URL('..', import.meta.url))
const BRAIN = process.argv[2] ?? join(ROOT, 'target/debug/brain')
const run = (p) => { try { return execFileSync(BRAIN, ['check', '--json', p], { cwd: join(ROOT, 'examples') }).toString() } catch (e) { if (!e.stdout) throw e; return e.stdout.toString() } }
const got = JSON.parse(run('invalid') || '[]')
const files = readdirSync(join(ROOT, 'examples/invalid'), { recursive: true })
  .filter(f => f.endsWith('.expected.json')).map(f => `invalid/${f.split(sep).join('/')}`).sort()
if (!files.length) throw new Error('No expected diagnostic fixtures found')
let fail = 0
for (const f of files) {
  const want = JSON.parse(readFileSync(join(ROOT, 'examples', f), 'utf8'))
  const stem = f.replace('.expected.json', '')
  const brain = ['.brain', '.jsbrain', '.cssbrain'].map((x) => stem + x).find((p) => existsSync(join(ROOT, 'examples', p))) ?? `${stem}.brain`
  const mine = got.filter((d) => d.file === brain)
  const key = (d) => `${d.code}@${d.line}:${d.col}/${d.level ?? 'error'}`
  const ok = mine.length === want.length && want.every((w, i) => key(w) === key(mine[i]) && mine[i].msg && mine[i].fix)
  if (!ok) fail++
  console.log(`${ok ? 'OK  ' : 'FAIL'} ${brain.padEnd(44)} want ${want.map(key).join(',')} got ${mine.map(key).join(',') || '-'}`)
}
const valid = JSON.parse(run('.').replace(/^\s*$/, '[]')).filter((d) => !d.file.startsWith('invalid/'))
// Dashboard intentionally imports a plain JavaScript API. That one uninstrumented
// boundary must remain visible; this is an oracle, not a checker exemption.
const expectedValid = [{ file: 'views/dashboard.brain', code: 'B0912', line: 6, col: 3, level: 'warning' }]
const validOk = valid.length === expectedValid.length && expectedValid.every((want, i) =>
  Object.entries(want).every(([field, value]) => valid[i][field] === value) &&
  typeof valid[i].msg === 'string' && valid[i].msg.trim() && typeof valid[i].fix === 'string' && valid[i].fix.trim())
console.log(`valid examples: ${validOk ? 'OK' : 'FAIL'} ${valid.length} diagnostics${valid.length ? ' → ' + valid.map((d) => `${d.file} ${d.code}@${d.line}:${d.col}/${d.level ?? 'error'}`).join(', ') : ''}`)
process.exit(fail || !validOk ? 1 : 0)
