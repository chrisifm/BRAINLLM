// `.brain_env` end-to-end gate (#140, INTERNALS §36 and the initial structure of §29).
// 1. `brain new` gives the full §29 tree with a 3-section `.brain_env`; untouched it passes doctor, builds and scores ≥ minScore.
// 2. `--env staging` and `--env production` put different PUBLIC_* values in dist; non-PUBLIC values never reach dist.
// 3. The process environment wins over the file.
// 4. An empty key in [production] fails `brain build --env production` with B0720.
// 5. No value of `.brain_env` appears in check/doctor/map JSON, the MCP tools, or the dev server log.
// 6. Re-runs bench/fixtures-pending/env/run.mjs (B0720–B0724 + no-leak).
// 7. LLM gate: asserts on the apps the autonomy harness built from bench/env/briefs-140.json.
// Every value is generated at run time (random suffixes), so no secret-shaped literal is ever committed.
// Usage: node bench/env/run.mjs [--brain target/release/brain] [--work dir] [--llm-tags env140-haiku,env140-sonnet] [--no-llm]
// Output: bench/env/results/summary.{json,md} (homedir replaced by ~), captured outputs in bench/env/results/logs/.
// Exit 1 on any failure.
import { execFileSync, spawn } from 'node:child_process'
import {
  cpSync, existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, statSync, writeFileSync,
} from 'node:fs'
import { createHash, randomBytes } from 'node:crypto'
import { createServer } from 'node:net'
import { homedir, tmpdir } from 'node:os'
import { join, relative } from 'node:path'
import { brotliDecompressSync, gunzipSync } from 'node:zlib'

const ROOT = new URL('../..', import.meta.url).pathname.replace(/\/$/, '')
const HERE = join(ROOT, 'bench/env')
const arg = (k, d) => { const i = process.argv.indexOf(`--${k}`); return i > 0 ? process.argv[i + 1] : d }
const BRAIN = arg('brain', join(ROOT, 'target/release/brain'))
const WORK = arg('work', mkdtempSync(join(tmpdir(), 'brainllm-env140-')))
const LLM_TAGS = process.argv.includes('--no-llm') ? [] : arg('llm-tags', 'env140-haiku,env140-sonnet').split(',').filter(Boolean)
const OUT = join(HERE, 'results')
const LOGS = join(OUT, 'logs')
rmSync(WORK, { recursive: true, force: true }); mkdirSync(WORK, { recursive: true })
mkdirSync(LOGS, { recursive: true })

const HOME = homedir()
const tilde = (s) => String(s).split(HOME).join('~')
const sha = (f) => createHash('sha256').update(readFileSync(f)).digest('hex').slice(0, 12)
const rnd = () => randomBytes(6).toString('hex')
const checks = []
const record = (id, name, ok, evidence, owner) => {
  checks.push({ id, name, ok, evidence: tilde(evidence), owner: ok ? null : owner })
  console.log(`${ok ? 'PASS' : 'FAIL'} ${id} ${name}${ok ? '' : ` · owner ${owner}`}\n     ${tilde(evidence)}`)
}
const saveLog = (name, text) => writeFileSync(join(LOGS, name), tilde(text))

// Runs brain and never throws: { code, out } with stdout + stderr.
function brain(args, cwd, env = {}, timeout = 300_000) {
  try {
    const out = execFileSync(BRAIN, args, { cwd, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'], timeout, env: { ...process.env, ...env } })
    return { code: 0, out }
  } catch (e) { return { code: e.status ?? -1, out: `${e.stdout ?? ''}${e.stderr ?? ''}` } }
}

// Minimal `.brain_env` reader (shared lines, [sections], # comments, optional quotes) for assertions only.
function parseEnv(text) {
  const env = { shared: {} }
  let sec = 'shared'
  for (const raw of text.split('\n')) {
    const line = raw.trim()
    if (!line || line.startsWith('#')) continue
    const s = line.match(/^\[([^\]]+)\]$/)
    if (s) { sec = s[1].trim(); env[sec] ??= {}; continue }
    const m = line.match(/^([A-Za-z_][A-Za-z0-9_]*)\s*=\s*(.*)$/)
    if (!m) continue
    let v = m[2]
    if (/^["']/.test(v)) v = v.slice(1, v.indexOf(v[0], 1) > 0 ? v.indexOf(v[0], 1) : undefined)
    else v = v.replace(/\s+#.*$/, '').trim()
    env[sec][m[1]] = v
  }
  return env
}
const resolved = (env, name) => ({ ...env.shared, ...(env[name] ?? {}) })

// Every file under dir as text (gzip/brotli siblings decompressed too), for literal substring search.
function corpus(dir) {
  const files = []
  const walk = (d) => {
    for (const e of readdirSync(d, { withFileTypes: true })) {
      const p = join(d, e.name)
      if (e.isDirectory()) walk(p)
      else {
        let buf = readFileSync(p)
        try { if (p.endsWith('.gz')) buf = gunzipSync(buf); else if (p.endsWith('.br')) buf = brotliDecompressSync(buf) } catch {}
        files.push({ path: relative(dir, p), text: buf.toString('latin1') })
      }
    }
  }
  walk(dir)
  return files
}
const hits = (files, value) => files.filter((f) => f.text.includes(value)).map((f) => f.path)

const BIN_SHA = sha(BRAIN)
console.log(`brain ${tilde(BRAIN)} sha256 ${BIN_SHA} · work ${tilde(WORK)}\n`)

// ---------------------------------------------------------------- 1. scaffold
const app1 = join(WORK, 'scaffold')
const neu = brain(['new', 'scaffold'], WORK)
saveLog('1-new.log', neu.out)
const PROMISED = [
  'app.brain', 'tokens.cssbrain', 'brain.json', '.brain_env', '.brain_env.example', '.gitignore', 'APP.md', 'AGENTS.md',
  'views/home.brain', 'views/about.brain', 'views/404.brain', 'views/403.brain', 'views/_error.brain', 'views/_loading.brain',
  'views/_offline.brain', 'views/login.brain', 'views/login.test.brain', 'views/posts/[id].brain',
  'views/account/_guard.jsbrain', 'views/account/index.brain',
  'comp/counter.brain', 'comp/counter.test.brain', 'ui/badge.brain',
  'channels/README.md', 'services/README.md', 'api/README.md', 'lib/README.md', 'server/README.md', 'content/README.md',
  'lib/posts.jsbrain', 'server/auth.jsbrain', 'channels/session.jsbrain',
  'public/favicon.svg', 'public/icon-192.png', 'public/icon-512.png', 'public/og.png',
]
const missing = PROMISED.filter((f) => !existsSync(join(app1, f)))
record('1a', `brain new: the §29 tree (${PROMISED.length} paths)`, neu.code === 0 && missing.length === 0,
  missing.length ? `missing: ${missing.join(', ')}` : `all ${PROMISED.length} paths present (exit ${neu.code})`, 'TOOLCHAIN brain-cli env/new')

{
  const text = existsSync(join(app1, '.brain_env')) ? readFileSync(join(app1, '.brain_env'), 'utf8') : ''
  const env = parseEnv(text)
  const ex = existsSync(join(app1, '.brain_env.example')) ? parseEnv(readFileSync(join(app1, '.brain_env.example'), 'utf8')) : { shared: {} }
  const gi = existsSync(join(app1, '.gitignore')) ? readFileSync(join(app1, '.gitignore'), 'utf8').split('\n').map((l) => l.trim()) : []
  const sections = ['development', 'staging', 'production'].filter((s) => env[s])
  const keysOk = ['PUBLIC_SITE_NAME', 'PUBLIC_SITE_URL'].every((k) => k in env.shared)
  const exValues = Object.values(ex).flatMap((o) => Object.values(o)).filter(Boolean)
  const exKeys = (o) => Object.entries(o).map(([s, kv]) => Object.keys(kv).map((k) => `${s}.${k}`)).flat().sort().join(',')
  const ok = sections.length === 3 && keysOk && gi.includes('.brain_env') && !gi.includes('.brain_env.example')
    && exValues.length === 0 && exKeys(ex) === exKeys(env)
  record('1b', '.brain_env: shared block + [development] [staging] [production], example in sync, gitignored', ok,
    `sections: ${sections.join(', ')} · shared keys: ${Object.keys(env.shared).join(', ')} · example values: ${exValues.length} · example keys match: ${exKeys(ex) === exKeys(env)} · .gitignore has .brain_env: ${gi.includes('.brain_env')}`,
    'TOOLCHAIN brain-cli env/new')
}

const doc1 = brain(['doctor'], app1)
saveLog('1-doctor.log', doc1.out)
record('1c', 'untouched app: brain doctor exits 0', doc1.code === 0, `exit ${doc1.code}: ${doc1.out.trim().split('\n').slice(-5).join(' | ')}`, 'TOOLCHAIN brain-cli env/new')
const bld1 = brain(['build'], app1)
saveLog('1-build.log', bld1.out)
record('1d', 'untouched app: brain build (static) succeeds', bld1.code === 0 && existsSync(join(app1, 'dist/index.html')),
  `exit ${bld1.code}: ${bld1.out.trim().split('\n').filter((l) => l.startsWith('summary')).join('')}`, 'BUILD bundle substitution')
{
  const seo = brain(['seo', 'audit', '--json'], app1)
  saveLog('1-seo.json', seo.out)
  let pages = [], min = 80
  try { const j = JSON.parse(seo.out); pages = j.pages ?? []; min = j.minScore ?? 80 } catch {}
  const scored = pages.filter((p) => p.indexable)
  const low = scored.filter((p) => !(p.score >= min))
  record('1e', `SEO audit: every indexable page ≥ ${min}`, seo.code === 0 && scored.length > 0 && low.length === 0,
    `${scored.map((p) => `${p.route} ${p.score}`).join(', ')} · noindex (not scored): ${pages.filter((p) => !p.indexable).map((p) => p.route).join(', ')}${low.length ? ` · LOW: ${low.map((p) => p.route).join(', ')}` : ''}`,
    'TOOLCHAIN brain-cli env/new')
}

// ---------------------------------------------------------------- 2–5 shared probe app
// A copy of the scaffold with unique values, a client read of PUBLIC_API_URL and a server read of DATABASE_URL.
const app2 = join(WORK, 'probe')
cpSync(app1, app2, { recursive: true })
rmSync(join(app2, 'dist'), { recursive: true, force: true })
rmSync(join(app2, '.brain'), { recursive: true, force: true })
const id = rnd()
const V = {
  siteName: `Probe${id}`,
  sharedUrl: `https://shared-${id}.example.org`,
  timeout: `timeout-${id}`,
  devApi: `http://localhost:4000/dev-${id}`,
  devPass: `devpass-${id}`,
  devSecret: randomBytes(24).toString('hex'),
  stgUrl: `https://stg-${id}.example.org`,
  stgApi: `https://stg-api-${id}.example.org/v1`,
  stgDb: `postgres://stg-db-${id}.internal/app`,
  prodUrl: `https://prod-${id}.example.org`,
  prodApi: `https://prod-api-${id}.example.org/v1`,
  prodDb: `postgres://prod-db-${id}.internal/app`,
  prodToken: `inttok-${id}${rnd()}`,
}
const ENV2 = `# probe for #140: every value is unique to this run
PUBLIC_SITE_NAME=${V.siteName}
PUBLIC_SITE_URL=${V.sharedUrl}
API_TIMEOUT_MS=${V.timeout}

[development]
PUBLIC_API_URL=${V.devApi}
DEMO_PASSWORD=${V.devPass}
BRAIN_SECRET=${V.devSecret}
DATABASE_URL=postgres://dev-db-${id}.internal/app

[staging]
PUBLIC_SITE_URL=${V.stgUrl}
PUBLIC_API_URL=${V.stgApi}
DATABASE_URL=${V.stgDb}

[production]
PUBLIC_SITE_URL=${V.prodUrl}
PUBLIC_API_URL=${V.prodApi}
DATABASE_URL=${V.prodDb}
INTERNAL_TOKEN=${V.prodToken}
`
writeFileSync(join(app2, '.brain_env'), ENV2)
{
  const about = join(app2, 'views/about.brain')
  const src = readFileSync(about, 'utf8').replace('\n<main class="page">',
    '\n<script>\n  const api = process.env.PUBLIC_API_URL\n</script>\n\n<main class="page">')
    .replace('</main>', '  <p>API base: <code>{api}</code></p>\n</main>')
  writeFileSync(about, src)
  const auth = join(app2, 'server/auth.jsbrain')
  writeFileSync(auth, readFileSync(auth, 'utf8').replace('const configured = () => Boolean(process.env.DEMO_PASSWORD)',
    'const configured = () => Boolean(process.env.DEMO_PASSWORD) && Boolean(process.env.DATABASE_URL)'))
}
const ALL_VALUES = Object.values(parseEnv(ENV2)).flatMap((o) => Object.values(o)).filter((v) => v.length >= 8)

// ---------------------------------------------------------------- 2. per-environment builds
const PRIVATE = { devPass: V.devPass, devSecret: V.devSecret, timeout: V.timeout, stgDb: V.stgDb, prodDb: V.prodDb, prodToken: V.prodToken, devDb: `dev-db-${id}` }
const builds = {}
for (const [envName, own, other] of [['staging', V.stgApi, V.prodApi], ['production', V.prodApi, V.stgApi]]) {
  const b = brain(['build', '--env', envName], app2)
  saveLog(`2-build-${envName}.log`, b.out)
  const files = b.code === 0 ? corpus(join(app2, 'dist')) : []
  const ownHits = hits(files, own), otherHits = hits(files, other)
  const clientHits = ownHits.filter((p) => !p.startsWith('_server/') && p !== 'server.mjs')
  const priv = Object.entries(PRIVATE).map(([k, v]) => [k, hits(files, v)]).filter(([, h]) => h.length)
  builds[envName] = { code: b.code, ownHits, otherHits, clientHits, priv }
  record(`2-${envName}`, `build --env ${envName}: its PUBLIC_API_URL in the client bundle, the other env's absent, no private value in dist`,
    b.code === 0 && clientHits.length > 0 && otherHits.length === 0 && priv.length === 0,
    `exit ${b.code} · own value in ${ownHits.length} file(s) (${clientHits.slice(0, 3).join(', ')}) · other env value in ${otherHits.length} · private values in dist: ${priv.length ? priv.map(([k, h]) => `${k} → ${h.slice(0, 3).join(', ')}`).join('; ') : 'none'}`,
    priv.length ? 'BUILD bundle substitution' : 'BUILD bundle substitution')
}
{
  const s = builds.staging, p = builds.production
  record('2-diff', 'staging and production bundles carry different PUBLIC_* values',
    s.clientHits.length > 0 && p.clientHits.length > 0 && s.otherHits.length === 0 && p.otherHits.length === 0,
    `staging → stg value only: ${s.clientHits.length > 0 && s.otherHits.length === 0} · production → prod value only: ${p.clientHits.length > 0 && p.otherHits.length === 0}`,
    'BUILD bundle substitution')
}
{ // PUBLIC_SITE_URL per environment drives canonical/sitemap: a second, independent PUBLIC_* signal.
  const b = brain(['build', '--env', 'staging'], app2)
  const files = b.code === 0 ? corpus(join(app2, 'dist')) : []
  const stg = hits(files, V.stgUrl).length, prod = hits(files, V.prodUrl).length
  const b2 = brain(['build', '--env', 'production'], app2)
  const files2 = b2.code === 0 ? corpus(join(app2, 'dist')) : []
  const stg2 = hits(files2, V.stgUrl).length, prod2 = hits(files2, V.prodUrl).length
  record('2-site-url', 'PUBLIC_SITE_URL follows --env (canonical, sitemap)', stg > 0 && prod === 0 && prod2 > 0 && stg2 === 0,
    `staging: stg url in ${stg} file(s), prod url in ${prod} · production: prod url in ${prod2}, stg url in ${stg2}`, 'BUILD bundle substitution')
}

// ---------------------------------------------------------------- 3. process env wins
{
  const over = `https://override-${id}.example.org/v9`
  const b = brain(['build', '--env', 'production'], app2, { PUBLIC_API_URL: over })
  saveLog('3-build-override.log', b.out)
  const files = b.code === 0 ? corpus(join(app2, 'dist')) : []
  const o = hits(files, over).filter((p) => !p.startsWith('_server/')), f = hits(files, V.prodApi)
  record('3', 'process env PUBLIC_API_URL overrides [production]', b.code === 0 && o.length > 0 && f.length === 0,
    `exit ${b.code} · override in ${o.length} client file(s) · file value in ${f.length}`, 'TOOLCHAIN brain-cli env/new')
}

// ---------------------------------------------------------------- 4. empty production key → B0720
{
  const app4 = join(WORK, 'empty-key')
  cpSync(app2, app4, { recursive: true })
  rmSync(join(app4, 'dist'), { recursive: true, force: true })
  writeFileSync(join(app4, '.brain_env'), ENV2.replace(`DATABASE_URL=${V.prodDb}`, 'DATABASE_URL='))
  const b = brain(['build', '--env', 'production'], app4)
  saveLog('4-build-empty.log', b.out)
  const leaked = ALL_VALUES.filter((v) => b.out.includes(v))
  const emptyEnvB = brain(['build', '--env', 'production'], app4, { DATABASE_URL: `postgres://host-${id}/db` })
  record('4', 'empty [production] key: build --env production fails with B0720 (host env fills it)',
    b.code !== 0 && b.out.includes('B0720') && b.out.includes('DATABASE_URL') && leaked.length === 0 && emptyEnvB.code === 0,
    `exit ${b.code} · B0720 in output: ${b.out.includes('B0720')} · names DATABASE_URL: ${b.out.includes('DATABASE_URL')} · values leaked: ${leaked.length} · same build with DATABASE_URL in the process env: exit ${emptyEnvB.code} · "${b.out.trim().split('\n').find((l) => l.includes('B0720'))?.slice(0, 160) ?? ''}"`,
    b.out.includes('B0720') ? 'TOOLCHAIN brain-cli env/new' : 'COMPILER B0720–B0724 checks')
}

// ---------------------------------------------------------------- 5. no value in any output
const leaks = []
const scan = (where, text) => {
  for (const v of ALL_VALUES) if (text.includes(v)) leaks.push({ where, value: `${v.slice(0, 6)}…` })
}
rmSync(join(app2, 'dist'), { recursive: true, force: true })
for (const [name, args] of [['check --json', ['check', '--json', '.']], ['doctor --json', ['doctor', '--json']], ['map --json', ['map', '--json']]]) {
  const r = brain(args, app2)
  saveLog(`5-${name.replace(/\W+/g, '-')}.log`, r.out)
  scan(`brain ${name}`, r.out)
  record(`5-${name.split(' ')[0]}`, `brain ${name}: no .brain_env value`, !leaks.some((l) => l.where === `brain ${name}`) && r.out.length > 0,
    `exit ${r.code} · ${r.out.length} bytes captured${name.startsWith('map') ? ` · env keys listed: ${(() => { try { return JSON.stringify(JSON.parse(r.out).env) } catch { return 'n/a' } })()}` : ''}`,
    'TOOLCHAIN brain-cli env/new')
}

// MCP over stdio: initialize, tools/list, then the tools; one request at a time.
async function mcpSession(cwd, calls) {
  const p = spawn(BRAIN, ['mcp'], { cwd, stdio: ['pipe', 'pipe', 'pipe'] })
  let buf = '', err = ''
  const pending = new Map()
  p.stdout.on('data', (d) => {
    buf += d
    let i
    while ((i = buf.indexOf('\n')) >= 0) {
      const line = buf.slice(0, i); buf = buf.slice(i + 1)
      try { const j = JSON.parse(line); if (j.id != null && pending.has(j.id)) { pending.get(j.id)(line); pending.delete(j.id) } } catch {}
    }
  })
  p.stderr.on('data', (d) => (err += d))
  let n = 0
  const req = (method, params, ms = 300_000) => new Promise((res) => {
    const rid = ++n
    const t = setTimeout(() => { pending.delete(rid); res(`TIMEOUT ${method}`) }, ms)
    pending.set(rid, (l) => { clearTimeout(t); res(l) })
    p.stdin.write(JSON.stringify({ jsonrpc: '2.0', id: rid, method, params }) + '\n')
  })
  const out = {}
  out.initialize = await req('initialize', { protocolVersion: '2025-06-18', capabilities: {}, clientInfo: { name: 'env140', version: '1' } })
  p.stdin.write(JSON.stringify({ jsonrpc: '2.0', method: 'notifications/initialized' }) + '\n')
  out['tools/list'] = await req('tools/list', {})
  for (const [label, name, a] of calls) out[label] = await req('tools/call', { name, arguments: a })
  p.stdin.end()
  await new Promise((r) => { const t = setTimeout(() => { p.kill('SIGTERM'); r() }, 5000); p.on('close', () => { clearTimeout(t); r() }) })
  out.stderr = err
  return out
}
{
  const mcp = await mcpSession(app2, [
    ['brain_check', 'brain_check', {}], ['brain_doctor', 'brain_doctor', {}], ['brain_map', 'brain_map', {}],
    ['brain_info', 'brain_info', {}], ['brain_read .brain_env', 'brain_read', { path: '.brain_env' }],
    ['brain_read .brain_env.example', 'brain_read', { path: '.brain_env.example' }],
  ])
  saveLog('5-mcp.jsonl', Object.entries(mcp).map(([k, v]) => `### ${k}\n${v}`).join('\n'))
  for (const [k, v] of Object.entries(mcp)) {
    const before = leaks.length
    scan(`mcp ${k}`, v)
    const answered = typeof v === 'string' && !v.startsWith('TIMEOUT') && (k === 'stderr' || v.includes('"result"') || v.includes('"error"'))
    if (k === 'initialize') continue
    record(`5-mcp-${k.replace(/\W+/g, '_')}`, `MCP ${k}: no .brain_env value`, leaks.length === before && answered,
      `${answered ? 'answered' : 'NO ANSWER'} · ${String(v).length} bytes${k.startsWith('brain_read') ? ` · ${String(v).slice(0, 160).replace(/\s+/g, ' ')}` : ''}`,
      'TOOLCHAIN brain-cli env/new')
  }
}

// Dev server: `brain run` on a free port for a few seconds, two requests, then kill by saved PID.
const freePort = () => new Promise((r) => { const s = createServer().listen(0, '127.0.0.1', () => { const p = s.address().port; s.close(() => r(p)) }) })
{
  const port = await freePort()
  const p = spawn(BRAIN, ['run', '--port', String(port)], { cwd: app2, stdio: ['ignore', 'pipe', 'pipe'], detached: true })
  const pid = p.pid
  let log = ''
  p.stdout.on('data', (d) => (log += d)); p.stderr.on('data', (d) => (log += d))
  const sleep = (ms) => new Promise((r) => setTimeout(r, ms))
  const statuses = [], bodies = {}
  for (let i = 0; i < 40 && !/localhost|127\.0\.0\.1|listening|ready/i.test(log); i++) await sleep(250)
  await sleep(1500)
  for (const path of ['/about', '/views/about.brain', '/', '/login', '/nope-404']) { // browser order: page, then its module
    try { const r = await fetch(`http://127.0.0.1:${port}${path}`); statuses.push(`${path} ${r.status}`); bodies[path] = await r.text() } catch (e) { statuses.push(`${path} ERR ${e.cause?.code ?? e.message}`) }
  }
  await sleep(2000)
  try { process.kill(-pid, 'SIGTERM') } catch {}
  await sleep(1000)
  let alive = true
  try { process.kill(pid, 0) } catch { alive = false }
  if (alive) { try { process.kill(-pid, 'SIGKILL') } catch {} }
  saveLog('5-run.log', log)
  const before = leaks.length
  scan('brain run log', log)
  const served = statuses.some((s) => / 200$/.test(s))
  record('5-run', 'brain run log (startup + requests): no .brain_env value', leaks.length === before && served,
    `port ${port} · ${statuses.join(', ')} · ${log.length} bytes of log · pid ${pid} stopped${alive ? ' (needed SIGKILL)' : ''}`,
    'RUNTIME server.mjs env')
  // `brain run` selects [development]: the dev bundle of /about (client-rendered in dev, so the compiled module
  // /views/about.brain) must carry its PUBLIC_API_URL as a constant; a raw `process.env` there is a ReferenceError
  // in the browser (blank view). Requested right after /about, as a browser does (order matters, see summary).
  // No private value may be in any response.
  const mod = bodies['/views/about.brain'] ?? ''
  const pages = Object.values(bodies).join('\n')
  const priv = Object.entries(PRIVATE).filter(([, v]) => pages.includes(v)).map(([k]) => k)
  record('5-run-env', 'brain run: [development] PUBLIC_* substituted in the dev module, no private value in responses',
    mod.includes(V.devApi) && !/process\.env\.PUBLIC_/.test(mod) && !pages.includes(V.prodApi) && priv.length === 0,
    `/views/about.brain has dev PUBLIC_API_URL: ${mod.includes(V.devApi)} · raw process.env.PUBLIC_ left: ${/process\.env\.PUBLIC_/.test(mod)} · prod value: ${pages.includes(V.prodApi)} · private values in responses: ${priv.join(', ') || 'none'}`,
    'TOOLCHAIN brain-cli env/new (dev server)')
}
{ // Scanner self-test: the same search must find every value in the file itself, or a 0 above means nothing.
  const found = ALL_VALUES.filter((v) => ENV2.includes(v)).length
  record('5-selftest', 'leak scanner finds every value in .brain_env itself', found === ALL_VALUES.length, `${found}/${ALL_VALUES.length}`, 'QA harness')
}
record('5-total', 'leaks found across every captured output', leaks.length === 0,
  leaks.length ? leaks.map((l) => `${l.where}: ${l.value}`).join('; ') : `0 leaks · ${ALL_VALUES.length} values searched literally`, 'TOOLCHAIN brain-cli env/new')

// ---------------------------------------------------------------- 6. pending fixtures
{
  let out = '', code = 0
  try { out = execFileSync('node', [join(ROOT, 'bench/fixtures-pending/env/run.mjs'), BRAIN], { encoding: 'utf8', timeout: 600_000 }) }
  catch (e) { out = `${e.stdout ?? ''}${e.stderr ?? ''}`; code = e.status ?? -1 }
  saveLog('6-fixtures.log', out)
  const lines = out.split('\n').filter((l) => /^(OK|MISS|LEAK)\s/.test(l))
  const okN = lines.filter((l) => l.startsWith('OK')).length
  record('6', `fixtures-pending/env: B0720–B0724 + no-leak (${okN}/5 OK)`, code === 0 && okN === 5 && lines.length === 5,
    `exit ${code} · ${lines.map((l) => l.replace(/\s+/g, ' ').slice(0, 70)).join(' | ')}`,
    'COMPILER B0720–B0724 checks')
}

// ---------------------------------------------------------------- 7. LLM gate
const URLS = { development: 'http://localhost:4000', staging: 'https://staging-api.statusboard.test', production: 'https://api.statusboard.test' }
const llm = []
for (const tag of LLM_TAGS) {
  const appDir = join(ROOT, 'bench/autonomy/results', tag, 'env-api-url')
  const rep = join(ROOT, 'bench/autonomy/results', `${tag}.json`)
  const row = { tag, notes: [] }
  if (!existsSync(join(appDir, 'app.brain'))) {
    row.ok = false; row.notes.push(`no app at ${appDir}`)
  } else {
    const envPath = join(appDir, '.brain_env')
    const env = existsSync(envPath) ? parseEnv(readFileSync(envPath, 'utf8')) : null
    const a = {}
    a.brainEnv = Boolean(env)
    // The key whose resolved value in each environment starts with the brief's URL.
    const urlKey = env && Object.keys({ ...env.shared, ...env.development, ...env.staging, ...env.production })
      .find((k) => Object.entries(URLS).every(([s, u]) => (resolved(env, s)[k] ?? '').startsWith(u)))
    a.urlKey = urlKey ?? null
    a.urlPerSection = Boolean(urlKey)
    a.urlPublic = Boolean(urlKey?.startsWith('PUBLIC_'))
    const keys = env ? new Set(Object.values(env).flatMap((o) => Object.keys(o))) : new Set()
    const tokenKeys = [...keys].filter((k) => /TOKEN/.test(k))
    a.tokenKeys = tokenKeys
    a.tokenServerOnly = tokenKeys.length > 0 && tokenKeys.every((k) => !k.startsWith('PUBLIC_'))
    a.tokenProdEmptyOrAbsent = env ? !(env.production?.STATUS_API_TOKEN) : false
    const files = (() => { try { return execFileSync('fd', ['-H', '-t', 'f', '-E', 'dist', '-E', '.brain', '.', appDir], { encoding: 'utf8' }).trim().split('\n').filter(Boolean) } catch { return [] } })()
    a.dotEnv = files.filter((f) => /(^|\/)\.env(\.|$)/.test(relative(appDir, f)))
    a.hardcoded = files.filter((f) => !/\.brain_env/.test(f) && !f.endsWith('.md'))
      .filter((f) => { try { const t = readFileSync(f, 'utf8'); return Object.values(URLS).some((u) => t.includes(u)) } catch { return false } })
      .map((f) => relative(appDir, f))
    a.tokenInClient = files.filter((f) => /\.(brain|jsbrain)$/.test(f))
      .filter((f) => { const t = readFileSync(f, 'utf8'); return /PUBLIC_[A-Z_]*TOKEN/.test(t) }).map((f) => relative(appDir, f))
    const d = brain(['doctor', '--json'], appDir)
    saveLog(`7-${tag}-doctor.json`, d.out)
    a.doctor = d.code
    let harness = null
    try { harness = JSON.parse(readFileSync(rep, 'utf8')).rows?.find((r) => r.id === 'env-api-url') ?? null } catch {}
    a.harnessDone = harness?.done ?? null
    row.metrics = harness ? { done: harness.done, views: `${harness.views}/${harness.pages}`, toolCalls: harness.toolCalls, turns: harness.turns, cost: harness.cost, s: Math.round((harness.ms ?? 0) / 1000), final: harness.final } : null
    row.assert = a
    row.ok = a.brainEnv && a.urlPerSection && a.urlPublic && a.tokenServerOnly && a.dotEnv.length === 0 && a.hardcoded.length === 0
      && a.tokenInClient.length === 0 && a.doctor === 0
    if (env) saveLog(`7-${tag}-brain_env.keys.txt`, Object.entries(env).map(([s, kv]) => `[${s}] ${Object.keys(kv).join(', ')}`).join('\n'))
  }
  llm.push(row)
  record(`7-${tag}`, `LLM gate ${tag}: URL per section in .brain_env as PUBLIC_*, token server-only, no .env, no hardcoded URL, doctor 0`, row.ok,
    row.assert ? JSON.stringify(row.assert) : row.notes.join('; '), 'TOOLCHAIN brain-cli env/new')
}

// ---------------------------------------------------------------- summary
const failed = checks.filter((c) => !c.ok)
const summary = {
  date: new Date().toISOString(), brain: tilde(BRAIN), brainSha: BIN_SHA, work: tilde(WORK),
  passed: checks.length - failed.length, failed: failed.length, leaks: leaks.length, checks, llm: JSON.parse(tilde(JSON.stringify(llm))),
}
writeFileSync(join(OUT, 'summary.json'), JSON.stringify(summary, null, 1) + '\n')
const md = [
  `# .brain_env gate (#140)`, '',
  `brain \`${tilde(BRAIN)}\` sha256 \`${BIN_SHA}\` · ${summary.date} · **${summary.passed} passed, ${summary.failed} failed, ${leaks.length} leaks**`, '',
  '| Check | Result | Evidence | Owner if failing |', '|---|---|---|---|',
  ...checks.map((c) => `| ${c.id} ${c.name} | ${c.ok ? 'PASS' : '**FAIL**'} | ${c.evidence.replace(/\|/g, '\\|')} | ${c.owner ?? ''} |`),
  '', '## LLM gate', '',
  '| Model run | Done (harness) | Views | Tool calls | Cost | Gate |', '|---|---|---|---|---|---|',
  ...llm.map((r) => `| ${r.tag} | ${r.metrics?.done ?? 'n/a'} | ${r.metrics?.views ?? 'n/a'} | ${r.metrics?.toolCalls ?? 'n/a'} | ${r.metrics?.cost != null ? `$${r.metrics.cost.toFixed(2)}` : 'n/a'} | ${r.ok ? 'PASS' : '**FAIL**'} |`),
  '',
]
writeFileSync(join(OUT, 'summary.md'), tilde(md.join('\n')))
console.log(`\n${summary.passed} passed · ${summary.failed} failed · ${leaks.length} leaks → ${tilde(join(OUT, 'summary.md'))}`)
process.exit(failed.length ? 1 : 0)
