// Recipes v1 verifier (INTERNALS §27, task #83).
// Every recipe in bench/recipes/<name>/ is a set of files an app drops in (paths relative to the app root, described by
// its recipe.json). For each recipe this script creates a fresh `brain new` app in the scratch directory, copies the
// recipe files over it and requires:
//   brain check --json  → []            (no errors, no warnings)
//   brain doctor --json → exit 0
//   brain test --json   → ≥ 1 test, all passing, no a11y findings
// It also probes `brain gen --list --json`: when the command exists, every recipe must be listed there (gap otherwise).
//
// Usage: node bench/recipes/verify.mjs [--only app-shell,crud] [--scratch ~/tmp/brainllm-recipes] [--bin target/release/brain]
// Output: a table on stdout, bench/recipes/results/verify.json; exit 1 when any recipe fails.
import { execFileSync, spawnSync } from 'node:child_process'
import { cpSync, existsSync, mkdirSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from 'node:fs'
import { homedir } from 'node:os'
import { join } from 'node:path'

const ROOT = new URL('../..', import.meta.url).pathname
const HERE = join(ROOT, 'bench/recipes')
const arg = (k, d) => { const i = process.argv.indexOf(`--${k}`); return i > 0 ? process.argv[i + 1] : d }
const ONLY = arg('only', '')
const SCRATCH = arg('scratch', join(homedir(), 'tmp/brainllm-recipes')).replace(/^~/, homedir())
const BIN_SRC = join(ROOT, arg('bin', 'target/release/brain'))
const tilde = (s) => String(s).split(homedir()).join('~')

if (!existsSync(BIN_SRC)) {
  console.error(`missing ${BIN_SRC}: run cargo build --release -p brain-cli`)
  process.exit(2)
}
mkdirSync(SCRATCH, { recursive: true })
// Freeze the binary for the whole run. macOS kills a binary copied over a previous copy: remove it first.
const BRAIN = join(SCRATCH, 'brain')
rmSync(BRAIN, { force: true })
execFileSync('cp', [BIN_SRC, BRAIN])

const recipes = readdirSync(HERE)
  .filter((d) => existsSync(join(HERE, d, 'recipe.json')))
  .filter((d) => !ONLY || ONLY.split(',').includes(d))
  .sort()

function run(args, cwd) {
  const r = spawnSync(BRAIN, args, { cwd, encoding: 'utf8', timeout: 300_000, maxBuffer: 64 << 20 })
  let json = null
  try { json = JSON.parse(r.stdout) } catch {}
  return { exit: r.status, json, out: r.stdout, err: r.stderr }
}

// Probe `brain gen` once: a missing command is a gap (TOOLCHAIN #80), not a recipe failure.
const genProbe = run(['gen', '--list', '--json'], SCRATCH)
const genExists = genProbe.exit === 0 && genProbe.json && !genProbe.json.error
const genNames = genExists ? new Set((Array.isArray(genProbe.json) ? genProbe.json : genProbe.json.recipes ?? []).map((r) => r.name ?? r.id ?? r)) : new Set()

const rows = []
for (const name of recipes) {
  const dir = join(HERE, name)
  const manifest = JSON.parse(readFileSync(join(dir, 'recipe.json'), 'utf8'))
  const app = join(SCRATCH, name)
  rmSync(app, { recursive: true, force: true })
  const created = run(['new', name, '--json'], SCRATCH)
  const row = { recipe: name, files: manifest.files.length, check: null, doctor: null, tests: null, recipeTests: null, gen: genExists ? genNames.has(name) : 'no brain gen', ok: false, problems: [] }
  if (created.exit !== 0) { row.problems.push(`brain new failed: ${created.err.slice(0, 200)}`); rows.push(row); continue }
  for (const f of manifest.files) {
    const src = join(dir, f)
    if (!existsSync(src)) { row.problems.push(`recipe.json lists a missing file: ${f}`); continue }
    mkdirSync(join(app, f, '..'), { recursive: true })
    cpSync(src, join(app, f))
  }
  // every file in the recipe folder (except its manifest and README) must be listed, so nothing ships by accident
  const walk = (d, p = '') => readdirSync(d).flatMap((e) => statSync(join(d, e)).isDirectory() ? walk(join(d, e), p + e + '/') : [p + e])
  for (const f of walk(dir)) if (!['recipe.json', 'README.md'].includes(f) && !manifest.files.includes(f)) row.problems.push(`file not listed in recipe.json: ${f}`)
  for (const f of manifest.remove ?? []) rmSync(join(app, f), { force: true })

  const c = run(['check', '--json'], app)
  row.check = Array.isArray(c.json) ? c.json.length : `exit ${c.exit}`
  if (!Array.isArray(c.json) || c.json.length) row.problems.push(`check: ${tilde(c.out.slice(0, 600))}`)

  const t = run(['test', '--json'], app)
  const files = Array.isArray(t.json) ? t.json : []
  const all = files.flatMap((f) => f.tests ?? [])
  const passed = all.filter((x) => x.ok).length
  const a11y = files.reduce((a, f) => a + (f.a11y?.length ?? 0) + (f.errors?.length ?? 0) + (f.leaks?.length ?? 0), 0)
  row.tests = `${passed}/${all.length}`
  // the recipe's own tests (the scaffold's tests run too and must keep passing)
  const own = files.filter((f) => manifest.files.includes(f.file)).flatMap((f) => f.tests ?? [])
  row.recipeTests = `${own.filter((x) => x.ok).length}/${own.length}`
  if (!own.length) row.problems.push('the recipe ships no *.test.brain')
  if (!all.length || passed !== all.length || a11y || t.exit !== 0) row.problems.push(`test (exit ${t.exit}, a11y/errors/leaks ${a11y}): ${tilde(t.out.slice(0, 800))}`)

  const d = run(['doctor', '--json'], app)
  row.doctor = d.exit
  if (d.exit !== 0) {
    // name only the failing sections and their error-level findings (the full report is in `brain doctor --json`)
    const failing = Object.entries(d.json ?? {}).filter(([, v]) => v && typeof v === 'object' && v.ok === false).map(([k, v]) => {
      const errs = [...(v.diags ?? []), ...(v.pages ?? []).flatMap((p) => (p.findings ?? []).map((f) => ({ ...f, route: p.route })))].filter((f) => (f.level ?? 'error') === 'error')
      return { section: k, errors: errs.slice(0, 6) }
    })
    row.problems.push(`doctor exit ${d.exit}: ${tilde(JSON.stringify(failing.length ? failing : d.json ?? d.out).slice(0, 1500))}`)
  }
  row.ok = row.problems.length === 0
  rows.push(row)
}

const report = { at: new Date().toISOString(), brain: tilde(BIN_SRC), scratch: tilde(SCRATCH), brainGen: genExists ? 'present' : tilde((genProbe.json?.error ?? genProbe.err ?? '').slice(0, 200)), ok: rows.filter((r) => r.ok).length, n: rows.length, rows }
mkdirSync(join(HERE, 'results'), { recursive: true })
writeFileSync(join(HERE, 'results', ONLY ? 'verify-partial.json' : 'verify.json'), JSON.stringify(report, null, 1) + '\n')

console.log('| recipe | files | check | doctor | recipe tests | all tests | brain gen | ok |\n|---|---:|---:|---:|---:|---:|---|---|')
for (const r of rows) console.log(`| ${r.recipe} | ${r.files} | ${r.check === 0 ? '[]' : r.check} | ${r.doctor} | ${r.recipeTests} | ${r.tests} | ${r.gen} | ${r.ok ? 'yes' : 'NO'} |`)
for (const r of rows) for (const p of r.problems) console.log(`\n${r.recipe}: ${p}`)
console.log(`\n${report.ok}/${report.n} recipes pass · brain gen: ${report.brainGen}`)
process.exit(report.ok === report.n && report.n > 0 ? 0 : 1)
