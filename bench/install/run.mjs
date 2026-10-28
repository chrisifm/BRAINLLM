#!/usr/bin/env node
// Clean-install test of the npm distribution (QA task #102, TOOLCHAIN #19, npm/README.md).
// Nothing is published and nothing touches the npm registry with a write: the tarballs from `node npm/pack.mjs` are
// installed straight from disk, with HOME, the npm cache and the npm prefix all inside a temp work dir.
// Steps:
//   1 SHA256SUMS: every listed sum matches the file inside its tarball, and every .tgz has an entry
//   2 `npm i <launcher.tgz> <platform.tgz>` → `npx brain --version` = `brain 0.1.0` (also `--version --json`)
//   3 `npx brain new demo` → `npm i -D` the same tarballs → `brain doctor` exit 0 → `npm run build` exit 0 → dist/
//     (pages, SEO scores) → `npm start -- --port P` serves it: GET / 200, /_health {ok,build} → `brain stop`
//   4 `npm i <create.tgz> <launcher.tgz> <platform.tgz>` → `npx create-brainllm app2 --no-install` → `brain check` = []
//   5 runtime package: every export resolves; `.` imports in plain Node
//   6 offline: step 2 again with an empty cache and `--offline`; the real ~ and ~/.npm get no new entries
//   7 hygiene: files, sizes, forbidden content (tests, maps, absolute home paths, secrets, .env), package.json fields
// Usage: node bench/install/run.mjs [--no-build]   (--no-build: use npm/dist as it is)
//   BRAIN_INSTALL_WORK=<dir> sets the work dir (default ~/tmp/brainllm-install; it is wiped first).
// Output: bench/install/results/summary.{json,md} (home dir written as ~). Exit 1 on any failed step.
import { spawn, spawnSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import { copyFileSync, existsSync, mkdirSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from 'node:fs'
import { homedir } from 'node:os'
import { join } from 'node:path'
import http from 'node:http'
import net from 'node:net'

const ROOT = new URL('../..', import.meta.url).pathname
const HERE = new URL('.', import.meta.url).pathname
const OUT = join(HERE, 'results')
const HOME_REAL = homedir()
const WORK = process.env.BRAIN_INSTALL_WORK ?? join(HOME_REAL, 'tmp', 'brainllm-install')
const NO_BUILD = process.argv.includes('--no-build')
const PLATFORM_KEYS = ['darwin-arm64', 'darwin-x64', 'linux-x64', 'linux-arm64', 'linux-x64-musl', 'win32-x64', 'win32-arm64']
const now = () => performance.now()
const sleep = (ms) => new Promise((r) => setTimeout(r, ms))
const tilde = (s) => String(s).split(HOME_REAL).join('~')
const T0 = now()

// ---------- isolated environment: nothing of this run lives outside WORK ----------
const TMPHOME = join(WORK, 'home'), CACHE = join(WORK, 'npm-cache'), PREFIX = join(WORK, 'npm-prefix'), TGZ = join(WORK, 'tgz')
const ENV = {
  ...Object.fromEntries(Object.entries(process.env).filter(([k]) => !/^npm_/i.test(k))),
  HOME: TMPHOME,
  npm_config_cache: CACHE,
  npm_config_prefix: PREFIX,
  npm_config_userconfig: join(TMPHOME, '.npmrc'),
  npm_config_update_notifier: 'false',
  npm_config_audit: 'false',
  npm_config_fund: 'false',
  npm_config_logs_dir: join(WORK, 'npm-logs'),
  BRAIN_NO_TELEMETRY: '1',
}

// read-only listings of the real home, before anything runs
const listDir = (d) => { try { return readdirSync(d).sort() } catch { return [] } }
const homeBefore = listDir(HOME_REAL)
const npmBefore = listDir(join(HOME_REAL, '.npm'))
const npmLogsBefore = listDir(join(HOME_REAL, '.npm', '_logs'))

function sh(cmd, args, cwd, { timeout = 600_000, env = {} } = {}) {
  const t = now()
  const r = spawnSync(cmd, args, { cwd, env: { ...ENV, ...env }, encoding: 'utf8', timeout, maxBuffer: 64 << 20 })
  return { code: r.status, out: r.stdout ?? '', err: r.stderr ?? '', ms: Math.round(now() - t), cmd: `${cmd} ${args.join(' ')}` }
}
const log = (...a) => console.log(...a)
const steps = []
function step(id, name) {
  const s = { id, name, pass: true, checks: [], numbers: {}, ms: 0, t: now() }
  steps.push(s)
  log(`\n== ${id} ${name}`)
  return s
}
function check(s, name, ok, detail = '') {
  s.checks.push({ name, ok: !!ok, detail: tilde(detail) })
  if (!ok) s.pass = false
  log(`  ${ok ? 'ok  ' : 'FAIL'} ${name}${detail ? ` — ${tilde(detail).slice(0, 400)}` : ''}`)
}
const end = (s) => { s.ms = Math.round(now() - s.t); delete s.t }
const findings = []
const finding = (owner, severity, what, fix) => findings.push({ owner, severity, what: tilde(what), fix })

// ---------- 0. build + pack (never publishes), copy the tarballs into WORK ----------
rmSync(WORK, { recursive: true, force: true })
for (const d of [TMPHOME, CACHE, PREFIX, TGZ, join(WORK, 'npm-logs')]) mkdirSync(d, { recursive: true })
let buildMs = 0
if (!NO_BUILD) {
  const t = now()
  for (const [c, a] of [['cargo', ['build', '--release', '-p', 'brain-cli']], ['node', ['npm/pack.mjs']]]) {
    const r = spawnSync(c, a, { cwd: ROOT, stdio: 'inherit' })
    if (r.status !== 0) { console.error(`${c} ${a.join(' ')} failed`); process.exit(1) }
  }
  buildMs = Math.round(now() - t)
}
const DIST = join(ROOT, 'npm', 'dist')
for (const f of readdirSync(DIST)) if (f.endsWith('.tgz') || f === 'SHA256SUMS') copyFileSync(join(DIST, f), join(TGZ, f))
const VERSION = /\[workspace\.package\][^[]*?\nversion\s*=\s*"([^"]+)"/.exec(readFileSync(join(ROOT, 'Cargo.toml'), 'utf8'))[1]
const tgz = (name) => join(TGZ, `christianirack-${name}-${VERSION}.tgz`)
const LAUNCHER = tgz('brainllm'), HOST_KEY = `${process.platform}-${process.arch}`, PLATFORM = tgz(`brainllm-${HOST_KEY}`)
const CREATE = tgz('create-brainllm'), RUNTIME = tgz('brainllm-runtime')
const NODE_V = process.version, NPM_V = sh('npm', ['-v'], WORK).out.trim()
log(`BRAINLLM ${VERSION} · node ${NODE_V} · npm ${NPM_V} · host ${HOST_KEY} · work ${tilde(WORK)}`)

const sha256 = (buf) => createHash('sha256').update(buf).digest('hex')
const untar = (file, dir) => { mkdirSync(dir, { recursive: true }); return sh('tar', ['-xzf', file, '-C', dir], WORK) }
const npmInit = (dir) => { mkdirSync(dir, { recursive: true }); writeFileSync(join(dir, 'package.json'), JSON.stringify({ name: dir.split('/').pop(), version: '1.0.0', private: true }, null, 2)) }

// ---------- 1. SHA256SUMS ----------
{
  const s = step('1', 'SHA256SUMS')
  const lines = readFileSync(join(TGZ, 'SHA256SUMS'), 'utf8').split('\n').filter(Boolean)
  s.numbers.entries = lines.length
  for (const l of lines) {
    const [sum, path] = l.split(/\s+/)
    const tgzFile = path.endsWith('.tgz') ? join(TGZ, path) : tgz(`brainllm-${path.split('/')[0]}`)
    let actual = null
    if (path.endsWith('.tgz')) actual = existsSync(tgzFile) ? sha256(readFileSync(tgzFile)) : null
    else {
      const dir = join(WORK, 'sums', path.split('/')[0])
      untar(tgzFile, dir)
      const f = join(dir, 'package', 'bin', path.split('/')[1])
      actual = existsSync(f) ? sha256(readFileSync(f)) : null
    }
    check(s, `${path} matches`, actual === sum, actual === sum ? sum.slice(0, 12) : `listed ${sum.slice(0, 12)}, actual ${actual?.slice(0, 12)}`)
  }
  const tgzs = readdirSync(TGZ).filter((f) => f.endsWith('.tgz'))
  const covered = tgzs.filter((f) => lines.some((l) => l.endsWith(`  ${f}`)))
  s.numbers.tarballs = tgzs.length
  s.numbers.tarballsCovered = covered.length
  check(s, 'every .tgz has a SHA256SUMS entry', covered.length === tgzs.length, `${covered.length}/${tgzs.length} covered; SHA256SUMS lists the native binaries (<os>-<arch>/brain), not the tarballs`)
  if (covered.length !== tgzs.length) finding('TOOLCHAIN (npm/pack.mjs)', 'medium', `SHA256SUMS covers ${lines.length} native binar${lines.length === 1 ? 'y' : 'ies'} and 0 of ${tgzs.length} .tgz files, so a downloaded tarball cannot be verified against it`, 'append `sha256  <file>.tgz` for every packed tarball after `npm pack` (or ship a second TARBALLS.SHA256SUMS)')
  end(s)
}

// ---------- 2. launcher + platform package ----------
const installLauncher = (dir, extra = []) => { npmInit(dir); return sh('npm', ['i', '--no-audit', '--no-fund', ...extra, LAUNCHER, PLATFORM], dir) }
{
  const s = step('2', 'npm i launcher + platform → npx brain --version')
  const dir = join(WORK, 's2')
  const r = installLauncher(dir)
  s.numbers.installMs = r.ms
  check(s, 'npm i exits 0', r.code === 0, r.code === 0 ? `${r.ms} ms` : r.err.slice(-600))
  const v = sh('npx', ['brain', '--version'], dir)
  s.numbers.versionMs = v.ms
  // the first run of a freshly written binary includes macOS's first-launch assessment; the second is the steady state
  s.numbers.versionWarmMs = sh('npx', ['brain', '--version'], dir).ms
  check(s, '`npx brain --version` prints `brain 0.1.0`', v.code === 0 && v.out.trim() === `brain ${VERSION}`, JSON.stringify(v.out.trim() || v.err.trim()))
  const j = sh('npx', ['brain', '--version', '--json'], dir)
  let parsed = null
  try { parsed = JSON.parse(j.out) } catch {}
  check(s, '`--version --json` is JSON with the version', j.code === 0 && parsed && JSON.stringify(parsed).includes(VERSION), j.out.trim() || j.err.trim())
  const others = PLATFORM_KEYS.filter((k) => k !== HOST_KEY && existsSync(join(dir, 'node_modules', '@christianirack', `brainllm-${k}`)))
  check(s, 'no other platform binary installed', others.length === 0, others.join(', '))
  if (/ENOTARGET|E404|404 Not Found/.test(r.err)) s.numbers.optionalDepsNote = 'npm logged 404s for the unpublished optional platforms (expected before publishing)'
  end(s)
}

// ---------- 3. brain new demo → doctor → build → start ----------
// registry traffic of the npm runs, read from their debug logs (GETs only; nothing here can write to a registry)
function registryGets(logsDir) {
  const lines = listDir(logsDir).flatMap((f) => readFileSync(join(logsDir, f), 'utf8').split('\n'))
  const gets = lines.filter((l) => /http fetch GET \d+ https?:\/\//.test(l))
  return { gets: gets.length, status: [...new Set(gets.map((l) => /GET (\d+)/.exec(l)[1]))], nonGet: lines.filter((l) => /http fetch (PUT|POST|DELETE|PATCH) /.test(l)).length }
}
const freePort = () => new Promise((ok) => { const srv = net.createServer(); srv.listen(0, '127.0.0.1', () => { const p = srv.address().port; srv.close(() => ok(p)) }) })
function get(port, path) {
  return new Promise((ok) => {
    const r = http.get({ host: '127.0.0.1', port, path, agent: false, timeout: 3000 }, (res) => {
      const parts = []
      res.on('data', (c) => parts.push(c))
      res.on('end', () => ok({ status: res.statusCode, body: Buffer.concat(parts).toString('utf8') }))
    })
    r.on('timeout', () => r.destroy(new Error('timeout')))
    r.on('error', (e) => ok({ error: e.code || e.message }))
  })
}
const alive = (pid) => { try { process.kill(pid, 0); return true } catch { return false } }
const leftovers = []
{
  const s = step('3', 'brain new demo → npm i -D → doctor → build → start')
  const base = join(WORK, 's2')
  const n = sh('npx', ['brain', 'new', 'demo'], base)
  check(s, '`npx brain new demo` exits 0', n.code === 0, n.code === 0 ? `${n.ms} ms` : n.err.slice(-400))
  const demo = join(base, 'demo')
  const pkg0 = existsSync(join(demo, 'package.json')) ? JSON.parse(readFileSync(join(demo, 'package.json'), 'utf8')) : {}
  check(s, 'demo/package.json pins @christianirack/brainllm ' + VERSION, pkg0.devDependencies?.['@christianirack/brainllm'] === VERSION, JSON.stringify(pkg0.devDependencies ?? null))
  const i = sh('npm', ['i', '-D', '--no-audit', '--no-fund', LAUNCHER, PLATFORM], demo)
  s.numbers.installMs = i.ms
  check(s, '`npm i -D` the tarballs inside demo exits 0', i.code === 0, i.code === 0 ? `${i.ms} ms` : i.err.slice(-600))
  const d = sh('npx', ['brain', 'doctor'], demo)
  s.numbers.doctorMs = d.ms
  check(s, '`npx brain doctor` exits 0', d.code === 0, d.code === 0 ? `${d.ms} ms` : (d.out + d.err).slice(-800))
  const b = sh('npm', ['run', 'build'], demo)
  s.numbers.buildMs = b.ms
  check(s, '`npm run build` exits 0', b.code === 0, b.code === 0 ? `${b.ms} ms` : (b.out + b.err).slice(-800))
  const dist = join(demo, 'dist')
  const walk = (d) => readdirSync(d, { withFileTypes: true }).flatMap((e) => (e.isDirectory() ? walk(join(d, e.name)) : [join(d, e.name)]))
  const files = existsSync(dist) ? walk(dist) : []
  const pages = files.filter((f) => f.endsWith('.html'))
  s.numbers.distFiles = files.length
  s.numbers.htmlFiles = pages.length
  s.numbers.distBytes = files.reduce((a, f) => a + statSync(f).size, 0)
  check(s, 'dist/ exists with pages', pages.length > 0, `${pages.length} .html files, ${files.length} files`)
  // build summary line ("summary N pages · … · SEO a–b") and the per-page scores of `brain seo audit --json`
  const summaryLine = (b.out + '\n' + b.err).split('\n').find((l) => /^summary /.test(l)) ?? ''
  s.numbers.buildSummary = summaryLine.trim()
  s.numbers.buildPages = Number(/(\d+) pages/.exec(summaryLine)?.[1] ?? NaN)
  s.numbers.seoRange = /SEO ([\d–-]+)/.exec(summaryLine)?.[1] ?? null
  writeFileSync(join(WORK, 'build-output.txt'), b.out + '\n--- stderr ---\n' + b.err)
  const seo = sh('npx', ['brain', 'seo', 'audit', '--json'], demo)
  let audit = null
  try { audit = JSON.parse(seo.out) } catch {}
  const scored = (audit?.pages ?? []).filter((p) => p.score != null)
  s.numbers.seoScores = Object.fromEntries(scored.map((p) => [p.route, p.score]))
  s.numbers.seoMin = scored.length ? Math.min(...scored.map((p) => p.score)) : null
  s.numbers.seoIndexable = scored.length
  check(s, 'build prints SEO scores; `brain seo audit` ok', s.numbers.seoRange && audit?.ok === true && scored.length > 0, `${summaryLine.trim()} · audit ok ${audit?.ok}, ${scored.length} indexable pages scored`)
  // npm start -- --port P (the documented prod command: package.json "start": "brain start")
  const port = await freePort()
  const t = now()
  const proc = spawn('npm', ['start', '--', '--port', String(port)], { cwd: demo, env: ENV, stdio: ['ignore', 'pipe', 'pipe'] })
  let out = '', err = '', exited = null
  proc.stdout.on('data', (c) => (out += c)); proc.stderr.on('data', (c) => (err += c))
  proc.on('exit', (c, sig) => (exited = c ?? sig))
  let health = null
  while (now() - t < 30_000 && exited === null) {
    const r = await get(port, '/_health')
    if (r.status === 200) { health = r; break }
    await sleep(50)
  }
  s.numbers.startToHealthMs = health ? Math.round(now() - t) : null
  let hj = null
  try { hj = JSON.parse(health?.body ?? '') } catch {}
  check(s, 'GET /_health → 200 {ok, build}', health && hj?.ok === true && hj.build != null, health ? health.body.slice(0, 200) : `no 200 in 30 s; exit ${exited}; ${(out + err).slice(-400)}`)
  const home = await get(port, '/')
  check(s, 'GET / → 200', home.status === 200, `status ${home.status ?? home.error}, ${home.body?.length ?? 0} bytes`)
  // stop: `brain stop`, then kill by saved pid only
  const st = sh('npx', ['brain', 'status', '--json'], demo)
  let kids = []
  try { kids = (JSON.parse(st.out).instances ?? []).map((x) => x.pid) } catch {}
  const stop = sh('npx', ['brain', 'stop'], demo)
  const ts = now()
  while (exited === null && now() - ts < 20_000) await sleep(50)
  s.numbers.stopMs = Math.round(now() - ts)
  check(s, '`brain stop` ends the supervisor', exited !== null, `stop exit ${stop.code}, npm start exit ${exited}`)
  if (exited === null) { try { process.kill(proc.pid, 'SIGKILL') } catch {} }
  for (const k of kids) if (alive(k)) { leftovers.push(k); try { process.kill(k, 'SIGKILL') } catch {} }
  const after = await get(port, '/_health')
  check(s, 'nothing listens on the port after stop', after.error != null, after.error ?? `status ${after.status}`)
  writeFileSync(join(WORK, 'start-output.txt'), out + '\n--- stderr ---\n' + err)
  end(s)
}

// ---------- 4. create-brainllm ----------
{
  const s = step('4', 'npm i create + launcher + platform → npx create-brainllm app2 --no-install')
  const dir = join(WORK, 's4')
  npmInit(dir)
  const i = sh('npm', ['i', '--no-audit', '--no-fund', CREATE, LAUNCHER, PLATFORM], dir)
  s.numbers.installMs = i.ms
  check(s, 'npm i exits 0', i.code === 0, i.code === 0 ? `${i.ms} ms` : i.err.slice(-600))
  const c = sh('npx', ['create-brainllm', 'app2', '--no-install'], dir)
  s.numbers.createMs = c.ms
  check(s, '`npx create-brainllm app2 --no-install` exits 0', c.code === 0, c.code === 0 ? `${c.ms} ms` : (c.out + c.err).slice(-600))
  const app = join(dir, 'app2')
  check(s, 'app2 has app.brain + tokens.cssbrain + package.json', ['app.brain', 'tokens.cssbrain', 'package.json'].every((f) => existsSync(join(app, f))))
  check(s, 'no node_modules in app2 (--no-install honored)', !existsSync(join(app, 'node_modules')))
  const pkg = existsSync(join(app, 'package.json')) ? JSON.parse(readFileSync(join(app, 'package.json'), 'utf8')) : {}
  check(s, `app2 pins @christianirack/brainllm ${VERSION}`, pkg.devDependencies?.['@christianirack/brainllm'] === VERSION, JSON.stringify(pkg.devDependencies ?? null))
  const k = sh('npx', ['brain', 'check', '--json'], app)
  let diags = null
  try { diags = JSON.parse(k.out) } catch {}
  s.numbers.checkMs = k.ms
  check(s, '`brain check --json` → []', k.code === 0 && Array.isArray(diags) && diags.length === 0, k.out.trim().slice(0, 300) || k.err.trim().slice(0, 300))
  end(s)
}

// ---------- 5. runtime package ----------
{
  const s = step('5', 'runtime package exports')
  const dir = join(WORK, 's5')
  npmInit(dir)
  const i = sh('npm', ['i', '--no-audit', '--no-fund', RUNTIME], dir)
  check(s, 'npm i runtime exits 0', i.code === 0, i.code === 0 ? `${i.ms} ms` : i.err.slice(-600))
  const pkgDir = join(dir, 'node_modules', '@christianirack', 'brainllm-runtime')
  const pkg = JSON.parse(readFileSync(join(pkgDir, 'package.json'), 'utf8'))
  const jsFiles = readdirSync(pkgDir, { recursive: true }).filter((f) => /\.(m?js)$/.test(f)).sort()
  const probe = `
    const name = '@christianirack/brainllm-runtime'
    const files = ${JSON.stringify(jsFiles)}
    const res = { main: null, resolved: [], unresolved: [], mainImport: null }
    try { res.main = import.meta.resolve(name) } catch (e) { res.main = 'ERR ' + e.code }
    for (const f of files) { try { import.meta.resolve(name + '/' + f); res.resolved.push(f) } catch (e) { res.unresolved.push(f + ' ' + e.code) } }
    try { const m = await import(name); res.mainImport = Object.keys(m).length } catch (e) { res.mainImport = 'ERR ' + e.message }
    console.log(JSON.stringify(res))`
  const r = sh('node', ['--input-type=module', '-e', probe], dir)
  let res = null
  try { res = JSON.parse(r.out) } catch {}
  s.numbers.exportsField = pkg.exports
  s.numbers.jsFiles = jsFiles.length
  s.numbers.resolved = res?.resolved.length
  check(s, '`.` resolves to brain.js', res && String(res.main).endsWith('/brain.js'), res?.main ?? r.err.slice(-300))
  check(s, 'every shipped .js/.mjs resolves through `./*`', res && res.unresolved.length === 0, res ? `${res.resolved.length}/${jsFiles.length}${res.unresolved.length ? ': ' + res.unresolved.join(', ') : ''}` : r.err.slice(-300))
  check(s, '`import("@christianirack/brainllm-runtime")` evaluates in Node', res && typeof res.mainImport === 'number', res ? `${res.mainImport} named exports` : r.err.slice(-300))
  s.numbers.namedExports = res?.mainImport
  end(s)
}

// ---------- 6. offline + nothing outside the temp dir ----------
{
  const s = step('6', 'offline install + no writes outside the work dir')
  const dir = join(WORK, 's6')
  const emptyCache = join(WORK, 'npm-cache-offline')
  mkdirSync(emptyCache, { recursive: true })
  npmInit(dir)
  const offLogs = join(WORK, 'npm-logs-offline')
  mkdirSync(offLogs, { recursive: true })
  const offEnv = { npm_config_cache: emptyCache, npm_config_logs_dir: offLogs }
  const r = sh('npm', ['i', '--offline', '--no-audit', '--no-fund', LAUNCHER, PLATFORM], dir, { env: offEnv })
  s.numbers.offlineInstallMs = r.ms
  check(s, '`npm i --offline` with an empty cache exits 0', r.code === 0, r.code === 0 ? `${r.ms} ms` : r.err.slice(-600))
  const v = sh('npx', ['--offline', 'brain', '--version'], dir, { env: offEnv })
  check(s, '`npx --offline brain --version` = brain ' + VERSION, v.out.trim() === `brain ${VERSION}`, v.out.trim() || v.err.trim().slice(-300))
  const off = registryGets(offLogs), online = registryGets(join(WORK, 'npm-logs'))
  s.numbers.offlineRegistryRequests = off.gets
  s.numbers.onlineRegistryGets = online.gets
  s.numbers.onlineRegistryStatus = online.status
  check(s, 'the offline run made 0 registry requests', off.gets === 0 && off.nonGet === 0, `${off.gets} GET`)
  check(s, 'no run made a registry write (PUT/POST/DELETE/PATCH)', online.nonGet === 0 && off.nonGet === 0, `${online.nonGet + off.nonGet}`)
  const homeAfter = listDir(HOME_REAL), npmAfter = listDir(join(HOME_REAL, '.npm')), logsAfter = listDir(join(HOME_REAL, '.npm', '_logs'))
  const added = (a, b) => b.filter((x) => !a.includes(x))
  const newHome = added(homeBefore, homeAfter), newNpm = added(npmBefore, npmAfter)
  // a new ~/.npm/_logs file is ours only if it mentions the work dir (other sessions may run npm meanwhile)
  const ours = added(npmLogsBefore, logsAfter).filter((f) => { try { return readFileSync(join(HOME_REAL, '.npm', '_logs', f), 'utf8').includes(WORK) } catch { return false } })
  s.numbers.newHomeEntries = newHome
  s.numbers.newDotNpmEntries = newNpm
  s.numbers.npmLogsFromThisRun = ours.length
  check(s, 'no new top-level entries in ~', newHome.length === 0, newHome.join(', '))
  check(s, 'no new entries in ~/.npm and no npm log of this run there', newNpm.length === 0 && ours.length === 0, [...newNpm, ...ours].join(', '))
  check(s, 'work dir holds the cache and logs', existsSync(join(CACHE, '_cacache')) || existsSync(join(CACHE, '_npx')), tilde(CACHE))
  end(s)
}

// ---------- 7. package hygiene ----------
const packages = []
{
  const s = step('7', 'package hygiene')
  const FORBIDDEN_PATH = /(^|\/)(test|tests|__tests__|demo|perf|bench|fixtures)(\/|$)|\.test\.|\.spec\.|(^|\/)\.env(\.|$)|\.map$|(^|\/)\.brain\/|(^|\/)\.DS_Store$|\.log$/
  const SECRET = /sk-ant-[A-Za-z0-9_-]{10,}|\bsk-[A-Za-z0-9]{32,}|ghp_[A-Za-z0-9]{30,}|gho_[A-Za-z0-9]{30,}|AKIA[A-Z0-9]{16}|-----BEGIN [A-Z ]*PRIVATE KEY-----|npm_[A-Za-z0-9]{36}/
  const homeName = HOME_REAL.split('/').pop()
  for (const f of readdirSync(TGZ).filter((x) => x.endsWith('.tgz')).sort()) {
    const dir = join(WORK, 'unpack', f.replace(/\.tgz$/, ''))
    untar(join(TGZ, f), dir)
    const root = join(dir, 'package')
    const files = readdirSync(root, { recursive: true, withFileTypes: true }).filter((e) => e.isFile()).map((e) => join(e.parentPath ?? e.path, e.name).slice(root.length + 1)).sort()
    const unpacked = files.reduce((a, x) => a + statSync(join(root, x)).size, 0)
    const pkg = JSON.parse(readFileSync(join(root, 'package.json'), 'utf8'))
    const row = { file: f, name: pkg.name, tgzBytes: statSync(join(TGZ, f)).size, unpackedBytes: unpacked, files: files.length, issues: [] }
    for (const x of files) if (FORBIDDEN_PATH.test(x)) row.issues.push(`ships ${x}`)
    for (const x of files) {
      const buf = readFileSync(join(root, x))
      const text = buf.toString('latin1')
      const abs = text.match(new RegExp(`/(Users|home)/${homeName}/[^\\s\\x00"']{0,160}`, 'g'))
      if (abs) {
        row.issues.push(`${x}: ${abs.length} absolute home path(s), e.g. ${[...new Set(abs)].slice(0, 2).join(', ')}`)
      }
      const sec = text.match(SECRET)
      if (sec) row.issues.push(`${x}: secret-like string ${sec[0].slice(0, 8)}…`)
    }
    row.fields = {
      license: pkg.license ?? null, engines: pkg.engines ?? null, bin: pkg.bin ?? null, os: pkg.os ?? null, cpu: pkg.cpu ?? null,
      optionalDependencies: pkg.optionalDependencies ? Object.keys(pkg.optionalDependencies).length : null,
      exports: pkg.exports ? Object.keys(pkg.exports) : null, repository: pkg.repository ?? null, type: pkg.type ?? null,
    }
    row.readme = files.includes('README.md')
    row.licenseFile = files.some((x) => /^(LICEN[CS]E|COPYING)(\.|$)/i.test(x))
    packages.push(row)
    log(`  ${pkg.name.padEnd(40)} ${String(row.files).padStart(3)} files  tgz ${(row.tgzBytes / 1024).toFixed(1).padStart(7)} KB  unpacked ${(unpacked / 1024).toFixed(1).padStart(7)} KB`)
  }
  const by = (n) => packages.find((p) => p.name === n)
  const L = by('@christianirack/brainllm'), P = by(`@christianirack/brainllm-${HOST_KEY}`), C = by('@christianirack/create-brainllm'), R = by('@christianirack/brainllm-runtime')
  check(s, 'every package has a README.md', packages.every((p) => p.readme), packages.filter((p) => !p.readme).map((p) => p.name).join(', '))
  check(s, 'launcher: bin.brain, engines.node, 7 optionalDependencies', L?.fields.bin?.brain && L.fields.engines?.node && L.fields.optionalDependencies === 7, JSON.stringify(L?.fields))
  const opt = L ? Object.keys(JSON.parse(readFileSync(join(WORK, 'unpack', L.file.replace(/\.tgz$/, ''), 'package', 'package.json'), 'utf8')).optionalDependencies) : []
  check(s, 'optionalDependencies name the 7 platforms at the same version', PLATFORM_KEYS.every((k) => opt.includes(`@christianirack/brainllm-${k}`)), opt.join(', '))
  check(s, `platform package: os [${process.platform}], cpu [${process.arch}], bin/brain executable`, P?.fields.os?.[0] === process.platform && P?.fields.cpu?.[0] === process.arch && (statSync(join(WORK, 'unpack', P.file.replace(/\.tgz$/, ''), 'package', 'bin', 'brain')).mode & 0o111) !== 0, JSON.stringify(P?.fields))
  check(s, 'create: bin.create-brainllm, engines.node', C?.fields.bin?.['create-brainllm'] && C.fields.engines?.node, JSON.stringify(C?.fields))
  check(s, 'runtime: type module + exports', R?.fields.type === 'module' && R.fields.exports, JSON.stringify(R?.fields))
  const secretIssues = packages.flatMap((p) => p.issues.filter((i) => /secret/.test(i)).map((i) => `${p.name}: ${i}`))
  check(s, 'no secret-like strings', secretIssues.length === 0, secretIssues.join('; '))
  const textIssues = packages.filter((p) => p.name !== P?.name).flatMap((p) => p.issues.filter((i) => !/secret/.test(i)).map((i) => `${p.name}: ${i}`))
  check(s, 'JS/text packages ship no dev files and no absolute home paths', textIssues.length === 0, textIssues.join('; '))
  const binIssues = (P?.issues ?? []).filter((i) => !/secret/.test(i))
  check(s, 'native binary embeds no absolute home paths', binIssues.length === 0, binIssues.join('; '))
  // findings with owners
  if (packages.some((p) => !p.licenseFile)) finding('Chris / TOOLCHAIN (npm/pack.mjs)', 'blocker for publishing', `no LICENSE file in any package; license field is "${L?.fields.license}" (npm shows it as not licensed for use)`, 'Chris picks the license; then pack.mjs copies LICENSE into every package (already a known pre-publish item in npm/README.md)')
  if (packages.every((p) => !p.fields.repository)) finding('TOOLCHAIN (npm/pack.mjs)', 'low', 'no `repository`/`homepage` in any package.json (npm provenance needs `repository` to match the repo)', 'add once the public repository exists (npm/README.md already lists it)')
  for (const p of packages) for (const i of p.issues) {
    if (/ships .*\.brain\//.test(i)) finding('TOOLCHAIN (npm/pack.mjs)', 'low', `${p.name} ${i}: a local tool cache picked up because pack.mjs lists untracked files (\`git ls-files --others\`)`, 'skip dot-directories (`/(^|\\/)\\./`) in the runtime copy loop, or pack only tracked files')
    else if (/ships /.test(i)) finding('TOOLCHAIN (npm/pack.mjs)', 'low', `${p.name} ${i}`, 'exclude it from the package files')
    else if (/absolute home path/.test(i) && p.name === P?.name) finding('BUILD (release profile)', 'medium', `${p.name} ${i}: build-machine paths (cargo registry sources and/or the workspace) are embedded in panic/debug locations and leak the builder's user name`, 'build releases with RUSTFLAGS `--remap-path-prefix=$HOME/.cargo/registry/src=/cargo` (stable; Cargo `trim-paths` is still unstable); CI builds would embed /home/runner/… instead')
    else if (/absolute home path/.test(i)) finding('TOOLCHAIN (npm/pack.mjs)', 'medium', `${p.name} ${i}`, 'strip absolute paths from the shipped file')
  }
  s.numbers.packages = packages.map((p) => ({ name: p.name, files: p.files, tgzKB: Math.round(p.tgzBytes / 102.4) / 10, unpackedKB: Math.round(p.unpackedBytes / 102.4) / 10 }))
  end(s)
}

// ---------- leftovers + report ----------
const totalMs = Math.round(now() - T0)
const ok = steps.every((s) => s.pass) && leftovers.length === 0
const summary = {
  task: 102, date: new Date().toISOString().slice(0, 10), version: VERSION, node: NODE_V, npm: NPM_V, host: HOST_KEY,
  work: tilde(WORK), buildAndPackMs: buildMs, totalMs, ok, leftoverPidsKilled: leftovers,
  steps: steps.map((s) => ({ id: s.id, name: s.name, pass: s.pass, ms: s.ms, numbers: s.numbers, checks: s.checks })),
  packages: packages.map(({ file, name, files, tgzBytes, unpackedBytes, readme, licenseFile, fields, issues }) => ({ file, name, files, tgzBytes, unpackedBytes, readme, licenseFile, fields, issues: issues.map(tilde) })),
  findings,
}
mkdirSync(OUT, { recursive: true })
const json = tilde(JSON.stringify(summary, null, 2))
writeFileSync(join(OUT, 'summary.json'), json + '\n')
const kb = (b) => `${(b / 1024).toFixed(1)} KB`
const num = (s) => {
  const n = s.numbers
  switch (s.id) {
    case '1': return `${n.entries} entry verified; ${n.tarballsCovered}/${n.tarballs} tarballs listed`
    case '2': return `install ${n.installMs} ms · \`npx brain --version\` first run ${n.versionMs} ms, then ${n.versionWarmMs} ms`
    case '3': return `install ${n.installMs} ms · doctor ${n.doctorMs} ms · build ${n.buildMs} ms · ${n.buildPages} pages (${n.htmlFiles} .html, ${n.distFiles} files, ${kb(n.distBytes ?? 0)}) · SEO ${n.seoRange} (${n.seoIndexable} indexable: ${Object.entries(n.seoScores ?? {}).map(([r, v]) => `${r} ${v}`).join(', ')}) · start→/_health 200 ${n.startToHealthMs} ms · stop ${n.stopMs} ms`
    case '4': return `install ${n.installMs} ms · create ${n.createMs} ms · check ${n.checkMs} ms`
    case '5': return `${n.resolved}/${n.jsFiles} files resolve · ${n.namedExports} named exports from \`.\``
    case '6': return `offline install ${n.offlineInstallMs} ms, ${n.offlineRegistryRequests} registry requests · online runs: ${n.onlineRegistryGets} registry GETs (status ${n.onlineRegistryStatus.join('/') || '—'}), 0 writes · new in ~: ${n.newHomeEntries.length} · new in ~/.npm: ${n.newDotNpmEntries.length}`
    case '7': return `${packages.length} packages`
  }
}
const md = [
  `# Clean-install test of the npm distribution (task #102)`, '',
  `Run of ${summary.date}: BRAINLLM ${VERSION}, Node ${NODE_V}, npm ${NPM_V}, ${HOST_KEY}. Work dir \`${tilde(WORK)}\` (temp HOME, npm cache and prefix). Total ${(totalMs / 1000).toFixed(1)} s${buildMs ? ` (cargo build + pack ${(buildMs / 1000).toFixed(1)} s)` : ''}. Overall: **${ok ? 'PASS' : 'FAIL'}**.`, '',
  '| Step | Result | Time | Numbers |', '|---|---|---|---|',
  ...steps.map((s) => `| ${s.id} ${s.name} | ${s.pass ? 'PASS' : 'FAIL'} | ${(s.ms / 1000).toFixed(1)} s | ${num(s)} |`), '',
  '## Failed checks', '',
  ...(steps.flatMap((s) => s.checks.filter((c) => !c.ok).map((c) => `- ${s.id} ${c.name}: ${c.detail}`)).concat(steps.every((s) => s.pass) ? ['- none'] : [])), '',
  '## Packages', '', '| Package | Files | .tgz | Unpacked | README | LICENSE |', '|---|---|---|---|---|---|',
  ...packages.map((p) => `| \`${p.name}\` | ${p.files} | ${kb(p.tgzBytes)} | ${kb(p.unpackedBytes)} | ${p.readme ? 'yes' : 'no'} | ${p.licenseFile ? 'yes' : 'no'} |`), '',
  '## Findings', '', '| Owner | Severity | Finding | Fix |', '|---|---|---|---|',
  ...(findings.length ? findings.map((f) => `| ${f.owner} | ${f.severity} | ${f.what.replace(/\|/g, '\\|')} | ${f.fix.replace(/\|/g, '\\|')} |`) : ['| — | — | none | — |']), '',
]
writeFileSync(join(OUT, 'summary.md'), tilde(md.join('\n')))
log(`\n${ok ? 'PASS' : 'FAIL'} · total ${(totalMs / 1000).toFixed(1)} s · results in ${tilde(OUT)}`)
process.exit(ok ? 0 : 1)
