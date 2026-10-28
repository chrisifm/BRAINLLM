// `.brain_env` fixtures (INTERNALS §36, B0720–B0724). Each case is a mini app generated in a temp dir, because an app
// has exactly one `.brain_env`. The B0723 value is assembled at run time so no secret-shaped string is ever committed
// (GitHub push protection would block it). For each case: run the command where the code must appear, check the code,
// and check that no value from `.brain_env` appears anywhere in the captured output (hard rule of §36).
// Usage: node bench/fixtures-pending/env/run.mjs [path to brain]   → exit 1 if a wired case fails or a value leaks.
import { execFileSync } from 'node:child_process'
import { mkdtempSync, writeFileSync, mkdirSync } from 'node:fs'
import { join } from 'node:path'
import { tmpdir } from 'node:os'

const ROOT = new URL('../../..', import.meta.url).pathname
const BRAIN = process.argv[2] ?? join(ROOT, 'target/debug/brain')
const base = mkdtempSync(join(tmpdir(), 'brainllm-env-'))
const secret = ['sk', 'live', '51Hx' + 'Q'.repeat(4) + 'abcdefghijklmnopqrstuv0123'].join('_')

const view = (script) => `<meta seo title="Environment probe" description="A probe page for the environment file checks of the BRAINLLM toolchain fixtures." canonical="/">
<script>
${script}
</script>
<main>
  <h1>Environment probe</h1>
  <p>{value}</p>
</main>
`
const cases = [
  { code: 'B0724', cmd: ['check', '--json', '.'], env: 'PUBLIC_SITE_NAME=Nimbus\nlower=1\n', values: ['Nimbus'],
    script: 'const value = import.meta.env.PUBLIC_SITE_NAME' },
  { code: 'B0723', cmd: ['check', '--json', '.'], env: `PUBLIC_STRIPE=${secret}\n`, values: [secret, 'sk_live'],
    script: 'const value = import.meta.env.PUBLIC_STRIPE' },
  { code: 'B0720', cmd: ['build', '--json', '--env', 'production'], env: 'PUBLIC_SITE_NAME=Nimbus\n\n[production]\nDATABASE_URL=\n', values: ['Nimbus'],
    script: 'const value = import.meta.env.PUBLIC_SITE_NAME' },
  { code: 'B0721', cmd: ['check', '--json', '.'], env: 'PUBLIC_SITE_NAME=Nimbus\n', values: ['Nimbus'],
    script: 'const name = import.meta.env.PUBLIC_SITE_NAME\n  const value = `${name} ${import.meta.env.PUBLIC_API_URL}`' },
  { code: 'B0722', cmd: ['doctor', '--json'], env: 'PUBLIC_SITE_NAME=Nimbus\nUNUSED_KEY=hidden-value-42\n', values: ['Nimbus', 'hidden-value-42'],
    script: 'const value = import.meta.env.PUBLIC_SITE_NAME' },
]

let fail = 0
for (const c of cases) {
  const app = join(base, c.code.toLowerCase())
  mkdirSync(app, { recursive: true })
  execFileSync(BRAIN, ['new', '.'], { cwd: app, stdio: 'ignore' })
  writeFileSync(join(app, '.brain_env'), c.env)
  writeFileSync(join(app, 'views/home.brain'), view(c.script))
  let out = ''
  try { out = execFileSync(BRAIN, c.cmd, { cwd: app, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'], timeout: 120_000 }) }
  catch (e) { out = `${e.stdout ?? ''}${e.stderr ?? ''}` }
  const fired = out.includes(c.code)
  const leaked = c.values.filter((v) => out.includes(v))
  const status = leaked.length ? 'LEAK' : fired ? 'OK  ' : 'MISS'
  if (leaked.length) fail++
  console.log(`${status} ${c.code} via \`brain ${c.cmd.join(' ')}\`${leaked.length ? ` · values in output: ${leaked.map((v) => v.slice(0, 6) + '…').join(', ')}` : ''}`)
}
console.log(`\napps in ${base} · MISS = the code does not reach this command yet (see README); LEAK always fails`)
process.exit(fail ? 1 : 0)
