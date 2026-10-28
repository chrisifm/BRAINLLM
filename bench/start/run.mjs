// `brain start` end to end (INTERNALS §34, QA task #120): the production supervisor against real built apps.
// Cases:
//   1a kill -9 of the child (fresh supervisor per repetition) → first 200 on /_health in < 1 s (median / max)
//   1b 12 kill -9 back to back on one supervisor → external kills are no crash loop: still serving, each < 1 s
//   2  RSS over --max-memory → `memory` log line, status lastExit.reason "memory", memoryRestarts ≥ 1, answers again
//   3  `brain reload` under constant load (50 keep-alive clients, --instances 2) → 0 failed requests, p99 latency
//   3r `brain reload` to a build whose child crashes at startup → exit 1, ok:false, old instances keep serving (0 failed)
//   4  a child that always crashes → `brain start` exits 1 and stderr explains it (time and restarts reported)
//      4a dies before ready (detected at once, crash loop inside the 60 s window, stderr explains) · 4b ready once, then every restart crashes
//   5  `brain start --install` output reviewed (paths, restart policy, env PATH, no secrets inline), temp HOME;
//      the plist's argv also runs in the plist's own environment (what launchd does); systemd unit from source
//   6  static app (no server.mjs): no Node process, _headers applied, ETag = build signature + 304, /_health build
// Usage: node bench/start/run.mjs [--reps 10] [--clients 50] [--reloads 3] [--only 1a,3]
//   BRAIN=<binary> skips `cargo build --release`; BRAIN_START_WORK=<dir> sets the scratch dir (default <tmp>/brainllm-start-bench).
// Output: bench/start/results/summary.json + summary.md (committed) · results/raw/ (logs, per-request data; ignored).
// Exits 1 on any failed case. Every supervisor started here is stopped with `brain stop`; leftovers are killed only by
// the pids this script spawned or read from `brain status --json`.
import { homedir } from 'node:os'
import { spawn, spawnSync, execFileSync } from 'node:child_process'
import { readFileSync, writeFileSync, rmSync, mkdirSync, existsSync, readdirSync, copyFileSync, chmodSync, cpSync, realpathSync } from 'node:fs'
import { join, dirname } from 'node:path'
import { tmpdir } from 'node:os'
import http from 'node:http'
import net from 'node:net'

const ROOT = new URL('../..', import.meta.url).pathname
const HERE = new URL('.', import.meta.url).pathname
const OUT = join(HERE, 'results'), RAW = join(OUT, 'raw')
const argv = process.argv.slice(2)
const arg = (k, d) => { const i = argv.indexOf(`--${k}`); return i >= 0 ? argv[i + 1] : d }
const REPS = Number(arg('reps', 10)), CLIENTS = Number(arg('clients', 50)), RELOADS = Number(arg('reloads', 3))
const ONLY = arg('only', '') ? new Set(arg('only', '').split(',')) : null
const want = (c) => !ONLY || ONLY.has(c)
const sleep = (ms) => new Promise((r) => setTimeout(r, ms))
const now = () => performance.now()
const WORK = process.env.BRAIN_START_WORK ?? join(tmpdir(), 'brainllm-start-bench')
rmSync(WORK, { recursive: true, force: true }); mkdirSync(WORK, { recursive: true })
rmSync(RAW, { recursive: true, force: true }); mkdirSync(RAW, { recursive: true })

// ---------- binary: build, fingerprint, copy (a concurrent rebuild of target/ cannot change what we test) ----------
let src = process.env.BRAIN
if (!src) {
  execFileSync('cargo', ['build', '--release', '-p', 'brain-cli'], { cwd: ROOT, stdio: 'inherit' })
  src = join(ROOT, 'target/release/brain')
}
const BRAIN = join(WORK, 'brain')
// macOS kills (SIGKILL, code signature) a binary copied over a previous copy: remove it first.
rmSync(BRAIN, { force: true }); copyFileSync(src, BRAIN); chmodSync(BRAIN, 0o755)
const SHA = execFileSync('shasum', ['-a', '256', BRAIN]).toString().slice(0, 12)
// The first exec of a freshly copied binary waits for macOS to assess it (code signature scan): 126 s measured on a
// loaded machine on 2026-10-13, longer than the 120 s timeout of every command below, which then killed `brain new`
// (exit null). Pay that cost once here, with no timeout, and record it.
const WARM_MS = (() => { const t = performance.now(); const r = spawnSync(BRAIN, ['help'], { encoding: 'utf8' }); if (r.status !== 0) { console.error(`the copied binary does not run: \`brain help\` exited ${r.status ?? r.signal}\n${r.stderr}`); process.exit(1) } return Math.round(performance.now() - t) })()
const NODE_V = execFileSync('node', ['-v']).toString().trim()
console.log(`brain ${SHA} (copied from ${src}) · node ${NODE_V} · work ${WORK} · first exec ${WARM_MS} ms`)

// ---------- helpers ----------
const brain = (cwd, args, env = {}) => {
  const r = spawnSync(BRAIN, args, { cwd, env: { ...process.env, ...env }, encoding: 'utf8', timeout: 120_000 })
  return { code: r.status, signal: r.signal, error: r.error?.code, out: r.stdout ?? '', err: r.stderr ?? '' }
}
// async variant: `brain reload` now blocks until the reload ends, and the load generator must keep running meanwhile
const brainAsync = (cwd, args) => new Promise((ok) => {
  const p = spawn(BRAIN, args, { cwd, stdio: ['ignore', 'pipe', 'pipe'] })
  let out = '', err = ''
  p.stdout.on('data', (c) => (out += c)); p.stderr.on('data', (c) => (err += c))
  p.on('close', (code) => ok({ code, out, err }))
})
// constant load: n clients in a loop on `path` → stop() resolves to [startedAt, ms, ok, statusOrError][]
function loadGen(port, n, keepAlive, path = '/') {
  const agent = keepAlive ? new http.Agent({ keepAlive: true, maxSockets: n }) : false
  const samples = []
  let on = true
  const one = () => new Promise((ok) => {
    const t = now()
    const r = http.get({ host: '127.0.0.1', port, path, agent, timeout: 15_000, headers: keepAlive ? {} : { connection: 'close' } }, (res) => {
      res.resume()
      res.on('end', () => ok([t, now() - t, res.statusCode >= 200 && res.statusCode < 300, res.statusCode]))
      res.on('error', (e) => ok([t, now() - t, false, e.code || e.message]))
    })
    r.on('timeout', () => r.destroy(new Error('timeout')))
    r.on('error', (e) => ok([t, now() - t, false, e.code || e.message]))
  })
  const workers = Array.from({ length: n }, async () => { while (on) samples.push(await one()) })
  return { samples, stop: async () => { on = false; await Promise.all(workers); if (agent) agent.destroy(); return samples } }
}
const status = (app) => { try { return JSON.parse(brain(app, ['status', '--json']).out.trim()) } catch { return null } }
// first child pid from `brain status --json` (retries while the instance list is still empty)
async function childPid(app, ms = 3000) {
  const t = now()
  while (now() - t < ms) { const p = status(app)?.instances?.[0]?.pid; if (p) return p; await sleep(10) }
  return null
}
const freePort = () => new Promise((ok) => { const s = net.createServer(); s.listen(0, '127.0.0.1', () => { const p = s.address().port; s.close(() => ok(p)) }) })
const alive = (pid) => { try { process.kill(pid, 0); return true } catch { return false } }

// One request on a fresh connection. → { status, headers, body, ms } or { error, ms }
function req(port, path = '/', headers = {}, timeout = 5000) {
  const t = now()
  return new Promise((ok) => {
    const r = http.get({ host: '127.0.0.1', port, path, headers, agent: false, timeout }, (res) => {
      const parts = []
      res.on('data', (c) => parts.push(c))
      res.on('end', () => ok({ status: res.statusCode, headers: res.headers, body: Buffer.concat(parts).toString('utf8'), ms: now() - t }))
      res.on('error', (e) => ok({ error: e.code || e.message, ms: now() - t }))
    })
    r.on('timeout', () => r.destroy(new Error('timeout')))
    r.on('error', (e) => ok({ error: e.code || e.message, ms: now() - t }))
  })
}
async function waitHealth(port, limitMs = 20_000) {
  const t = now()
  while (now() - t < limitMs) {
    const r = await req(port, '/_health', {}, 2000)
    if (r.status === 200) return now() - t
    await sleep(5)
  }
  return null
}
const logEvents = (app) => {
  const f = join(app, 'logs/app.log')
  if (!existsSync(f)) return []
  return readFileSync(f, 'utf8').split('\n').filter(Boolean).map((l) => { try { return JSON.parse(l) } catch { return { raw: l } } })
}
const brainEvents = (app) => logEvents(app).filter((e) => e.src === 'brain-start')
const pct = (xs, p) => { if (!xs.length) return null; const s = [...xs].sort((a, b) => a - b); return s[Math.min(s.length - 1, Math.ceil((p / 100) * s.length) - 1)] }
const max = (xs) => xs.reduce((a, b) => (b > a ? b : a), -Infinity), min = (xs) => xs.reduce((a, b) => (b < a ? b : a), Infinity)
const r1 = (x) => (x == null ? null : Math.round(x * 10) / 10)

// supervisors this run started: { app, proc, pid } — all stopped at the end
const started = []
// env: added to this process's environment, or the whole environment when `clean` (what a service manager passes)
function startApp(app, port, extra = [], env = {}, clean = false) {
  const tag = `${app.split('/').pop()}-${port}`
  const proc = spawn(BRAIN, ['start', '--port', String(port), ...extra], { cwd: app, env: clean ? env : { ...process.env, ...env }, stdio: ['ignore', 'pipe', 'pipe'] })
  const s = { app, proc, pid: proc.pid, out: '', err: '', exit: null, exitAt: null, t0: now(), tag }
  proc.stdout.on('data', (c) => (s.out += c))
  proc.stderr.on('data', (c) => (s.err += c))
  proc.on('exit', (code, sig) => { s.exit = code ?? sig; s.exitAt = now() })
  started.push(s)
  return s
}
async function stopApp(s) {
  if (s.saved) return
  s.saved = true
  const st = s.exit === null ? status(s.app) : null
  const kids = (st?.instances ?? []).map((i) => i.pid)
  if (s.exit === null) brain(s.app, ['stop'])
  const t = now()
  while (s.exit === null && now() - t < 20_000) await sleep(50)
  if (s.exit === null) { try { process.kill(s.pid, 'SIGKILL') } catch {} }
  for (const k of kids) if (alive(k)) { try { process.kill(k, 'SIGKILL') } catch {} }
  writeFileSync(join(RAW, `${s.tag}.stderr.txt`), s.err)
  const log = join(s.app, 'logs/app.log')
  if (existsSync(log)) copyFileSync(log, join(RAW, `${s.tag}.app.log`))
}
async function cleanup() { for (const s of started) await stopApp(s) }
process.on('SIGINT', async () => { await cleanup(); process.exit(130) })

// ---------- apps ----------
console.log('building apps…')
// Every setup command must succeed: a failed `brain new` used to surface later as an unrelated ENOENT.
const must = (cwd, args) => {
  const r = brain(cwd, args)
  if (r.code !== 0) {
    console.error(`setup failed: \`brain ${args.join(' ')}\` in ${cwd} exited ${r.code ?? `by signal ${r.signal}`}${r.error ? ` (${r.error})` : ''}\n${(r.err || r.out).trim()}`)
    process.exit(1)
  }
  return r
}
must(WORK, ['new', 'srv']); must(WORK, ['new', 'stat'])
const SRV = join(WORK, 'srv'), STAT = join(WORK, 'stat')
// a `server:` function makes the build emit dist/server.mjs (INTERNALS §11.1, SPEC "Server and secrets").
// The bench writes the whole view itself instead of patching the scaffold's about.brain, so a change to the scaffold
// text (or a scaffold without that file) cannot break the setup.
{
  mkdirSync(join(SRV, 'views'), { recursive: true })
  writeFileSync(join(SRV, 'views/about.brain'), `<meta seo title="About srv" description="Server probe of the brain start bench: one server function called from a button." canonical="/about">
<script>
  let n = sig(0)
  server: async function ping(ctx, data) {
    return { ok: true, n: Number(data && data.n) || 0 }
  }
  const go = async () => {
    const r = await ping({ n: 1 })
    n = r.n
  }
</script>

<main class="page">
  <h1>About srv</h1>
  <button type="button" on:click={go}>Ping {n}</button>
  <p>This view exists so that the build emits a server: it calls one server function. Back to the <a href="/">home page</a>.</p>
</main>

<style>
  .page { max-width: 68ch; margin: 0 auto; padding: var(--space-6) var(--space-4); display: grid; gap: var(--space-4); }
</style>
`)
  // a secret that must never be inlined in the install output
  writeFileSync(join(SRV, '.brain_env'), (existsSync(join(SRV, '.brain_env')) ? readFileSync(join(SRV, '.brain_env'), 'utf8') : '') + '\nSECRET_TOKEN=sk-qa-should-not-leak-1234\n')
}
// the static app must not need a server: since #65 the scaffold ships guarded views (/403, _guard, login, account),
// server modules and `load`/`server:` code. Remove all of it (and the links and layout that point to it).
function makeStatic(app) {
  const gone = []
  const rm = (rel) => { if (existsSync(join(app, rel))) { rmSync(join(app, rel), { recursive: true, force: true }); gone.push(rel) } }
  for (const rel of ['views/403.brain', 'views/account', 'views/login.brain', 'views/login.test.brain', 'views/posts', 'views/_layout.brain', 'lib/posts.jsbrain', 'channels/session.jsbrain']) rm(rel)
  if (existsSync(join(app, 'server'))) for (const f of readdirSync(join(app, 'server'))) if (f.endsWith('.jsbrain')) rm(`server/${f}`)
  // any other view that needs the server: a guard, `load`, a `server:` function or a `<meta server>` import
  for (const f of readdirSync(join(app, 'views'), { recursive: true })) {
    const rel = `views/${f}`, abs = join(app, rel)
    if (!existsSync(abs) || !/\.(brain|jsbrain)$/.test(f)) continue
    if (/^_guard\./.test(f.split('/').pop()) || /\bserver:\s*async|\bexport\s+(async\s+)?function\s+load\b|\bload\s*\(/.test(readFileSync(abs, 'utf8'))) rm(rel)
  }
  // drop nav links to routes that no longer exist
  const shell = join(app, 'app.brain'), routes = new Set(['/', '/about'])
  if (existsSync(shell)) writeFileSync(shell, readFileSync(shell, 'utf8').split('\n').filter((l) => { const m = /<a [^>]*href="(\/[^"]*)"/.exec(l); return !m || routes.has(m[1]) || !/^\s*<a /.test(l) }).join('\n'))
  return gone
}
const statRemoved = makeStatic(STAT)
writeFileSync(join(RAW, 'stat-removed.txt'), statRemoved.join('\n') + '\n')
const sigOf = (out) => (/^build (\S+)$/m.exec(out) || [])[1] ?? null
const bSrv = brain(SRV, ['build']), bStat = brain(STAT, ['build'])
writeFileSync(join(RAW, 'build-srv.txt'), bSrv.out + bSrv.err); writeFileSync(join(RAW, 'build-stat.txt'), bStat.out + bStat.err)
const srvBad = bSrv.code !== 0 ? `server app build exited ${bSrv.code}: ${(bSrv.out + bSrv.err).trim().split('\n').slice(-5).join(' | ')}` : !existsSync(join(SRV, 'dist/server.mjs')) ? 'server app build emitted no dist/server.mjs' : null
const statBad = bStat.code !== 0 ? `static app build exited ${bStat.code}: ${(bStat.out + bStat.err).trim().split('\n').slice(-5).join(' | ')}` : existsSync(join(STAT, 'dist/server.mjs')) ? `static app build emitted dist/server.mjs (removed: ${statRemoved.join(', ')})` : null
const SIG_SRV = sigOf(bSrv.out), SIG_STAT = sigOf(bStat.out)
// fresh copy of the server app per case: logs and run dirs never mix
const cloneSrv = (name) => { const d = join(WORK, name); cpSync(SRV, d, { recursive: true }); rmSync(join(d, 'logs'), { recursive: true, force: true }); rmSync(join(d, '.brain/run'), { recursive: true, force: true }); return d }

const cases = []
const record = (c) => { cases.push(c); console.log(`[${c.pass === null ? 'INFO' : c.pass ? 'PASS' : 'FAIL'}] ${c.id} ${c.name}: ${c.numbers}`); for (const f of c.failures ?? []) console.log('   - ' + f) }
// a failed precondition fails only the cases that need that app; the others still run
const NEEDS = { srv: ['1a', '1b', '2', '3', '3b', '3p', '3bp', '3r', '4a', '4b', '5'], stat: ['6'] }
const blocked = new Set()
for (const [app, bad] of [['srv', srvBad], ['stat', statBad]]) {
  if (!bad) continue
  for (const id of NEEDS[app]) if (want(id)) { blocked.add(id); record({ id, name: `precondition: ${app} app`, pass: false, numbers: 'not run', failures: [bad], owner: 'QA bench / scaffold' }) }
}
const run = (id) => want(id) && !blocked.has(id)

try {
  // ---------- 1a: kill -9, fresh supervisor per repetition ----------
  if (run('1a')) {
    const times = [], failures = [], reps = []
    for (let i = 0; i < REPS; i++) {
      const app = cloneSrv(`kill-${i}`), port = await freePort()
      const s = startApp(app, port)
      if ((await waitHealth(port)) == null) { failures.push(`rep ${i}: never up; stderr: ${s.err.slice(-300)}`); await stopApp(s); continue }
      const pid = await childPid(app)
      if (!pid) { failures.push(`rep ${i}: no child pid in brain status`); await stopApp(s); continue }
      process.kill(pid, 'SIGKILL')
      const t = now()
      // first 200 after the kill; connections opened meanwhile queue on the supervisor's socket
      let ms = null
      while (now() - t < 15_000) {
        const r = await req(port, '/_health', {}, 3000)
        if (r.status === 200) { ms = now() - t; break }
        await sleep(2)
      }
      const after = status(app)
      const newPid = after?.instances?.[0]?.pid
      reps.push({ rep: i, ms: r1(ms), oldPid: pid, newPid, restarts: after?.restarts, lastExit: after?.lastExit })
      if (ms == null) failures.push(`rep ${i}: no 200 within 15 s after kill -9 of ${pid}`)
      else { times.push(ms); if (ms >= 1000) failures.push(`rep ${i}: back in ${r1(ms)} ms (≥ 1000)`) }
      if (newPid === pid) failures.push(`rep ${i}: pid did not change`)
      if (after?.restarts !== 1) failures.push(`rep ${i}: status restarts=${after?.restarts}, expected 1`)
      await stopApp(s)
      if (s.exit !== 0) failures.push(`rep ${i}: brain stop → supervisor exit ${s.exit}`)
      rmSync(app, { recursive: true, force: true })
    }
    writeFileSync(join(RAW, '1a-kill.json'), JSON.stringify(reps, null, 1))
    record({ id: '1a', name: 'kill -9 of the child → back in < 1 s (fresh supervisor per rep)', pass: failures.length === 0 && times.length === REPS,
      numbers: `n=${times.length} median ${r1(pct(times, 50))} ms · max ${r1(max(times))} ms · min ${r1(min(times))} ms`,
      data: { reps, medianMs: r1(pct(times, 50)), maxMs: r1(max(times)) }, failures, owner: 'TOOLCHAIN' })
  }

  // ---------- 1b: kill -9 back to back on one supervisor ----------
  // external kills of a ready child are not crashes: no crash loop, no backoff growth
  if (run('1b')) {
    const app = cloneSrv('kill-burst'), port = await freePort(), failures = [], KILLS = 12
    const s = startApp(app, port)
    const kills = []
    if ((await waitHealth(port)) == null) failures.push('never up')
    else {
      let prev = null
      for (let i = 0; i < KILLS && s.exit === null; i++) {
        // the pid of the current (ready) child, never the one just killed
        let pid = null
        const tp = now()
        while (now() - tp < 3000) { pid = await childPid(app); if (pid && pid !== prev) break; await sleep(10) }
        if (!pid || pid === prev) { failures.push(`kill ${i + 1}: no new child pid in brain status`); break }
        process.kill(pid, 'SIGKILL'); prev = pid
        const t = now(); let ms = null
        while (now() - t < 40_000 && s.exit === null) {
          const r = await req(port, '/_health', {}, 35_000)
          if (r.status === 200) { ms = now() - t; break }
          await sleep(2)
        }
        kills.push({ kill: i + 1, pid, ms: r1(ms), supervisorExit: s.exit })
        if (ms == null) failures.push(`kill ${i + 1}: no 200 (supervisor exit ${s.exit})`)
        else if (ms >= 1000) failures.push(`kill ${i + 1}: back in ${r1(ms)} ms (≥ 1000)`)
      }
    }
    await sleep(300)
    const exitCode = s.exit, st = status(app)
    const crashLoop = brainEvents(app).some((e) => e.event === 'crash-loop')
    const finalHealth = (await req(port, '/_health')).status
    if (exitCode !== null) failures.push(`supervisor exited ${exitCode}${crashLoop ? ' (crash loop declared)' : ''}`)
    if (kills.length < KILLS) failures.push(`only ${kills.length} of ${KILLS} kills done`)
    if (finalHealth !== 200) failures.push(`after the kills /_health → ${finalHealth}`)
    if (st?.lastExit && !/killed from outside/.test(st.lastExit.reason ?? '')) failures.push(`status lastExit.reason "${st?.lastExit?.reason}", expected "killed from outside…"`)
    await stopApp(s)
    const times = kills.map((k) => k.ms).filter((x) => x != null)
    writeFileSync(join(RAW, '1b-kill-burst.json'), JSON.stringify({ kills, exitCode, crashLoop, lastExit: st?.lastExit, restarts: st?.restarts }, null, 1))
    record({ id: '1b', name: `${KILLS} kill -9 back to back on one supervisor → no crash loop, each < 1 s`, pass: failures.length === 0,
      numbers: `recovery per kill (ms): ${kills.map((k) => k.ms ?? '—').join(', ')} · median ${r1(pct(times, 50))} · max ${r1(max(times))} · supervisor ${exitCode === null ? 'still serving' : 'exited ' + exitCode} · lastExit "${st?.lastExit?.reason}" · restarts ${st?.restarts}`,
      data: { kills, exitCode, crashLoop, lastExit: st?.lastExit, restarts: st?.restarts }, failures, owner: 'TOOLCHAIN' })
  }

  // ---------- 2: RSS over --max-memory ----------
  if (run('2')) {
    const app = cloneSrv('mem'), port = await freePort(), failures = []
    const s = startApp(app, port, ['--max-memory', '1M', '--rss-ms', '200', '--health-ms', '600000'])
    let d = {}
    if ((await waitHealth(port)) == null) failures.push('never up')
    else {
      const pid1 = await childPid(app)
      const t = now(); let memEv = null, pid2 = null, backMs = null
      while (now() - t < 15_000) {
        memEv ??= brainEvents(app).find((e) => e.event === 'memory') ?? null
        const p = status(app)?.instances?.[0]?.pid
        if (memEv && p && p !== pid1) { pid2 = p; break }
        await sleep(20)
      }
      const tAfter = now()
      if (pid2) {
        while (now() - tAfter < 10_000) {
          const r = await req(port, '/_health', {}, 3000)
          if (r.status === 200) { backMs = now() - t; break }
          await sleep(5)
        }
      }
      const st = status(app)
      const exitEv = brainEvents(app).find((e) => e.event === 'exit')
      d = { pid1, pid2, memoryEvent: memEv, exitEvent: exitEv ?? null, msToFirst200AfterDetectStart: r1(backMs), statusLastExit: st?.lastExit, rssKB: st?.instances?.map((i) => i.rssKB) }
      if (!memEv) failures.push('no `memory` event in logs/app.log')
      if (!pid2) failures.push(`child pid never changed (still ${pid1})`)
      if (backMs == null) failures.push('no 200 on /_health after the memory restart')
      d.memoryRestarts = st?.memoryRestarts
      if (st?.lastExit?.reason !== 'memory') failures.push(`status lastExit.reason "${st?.lastExit?.reason}" (code ${st?.lastExit?.code}), expected "memory"`)
      if (!(st?.memoryRestarts >= 1)) failures.push(`status memoryRestarts ${st?.memoryRestarts}, expected ≥ 1`)
      // every child stays over a 1M limit for 15 s: memory restarts are planned, so no crash loop
      while (s.exit === null && now() - s.t0 < 15_000) await sleep(100)
      const st2 = status(app)
      d.afterSustainedOverLimit = { seconds: 15, supervisorExit: s.exit, memoryRestarts: st2?.memoryRestarts, memoryEvents: brainEvents(app).filter((e) => e.event === 'memory').length, crashLoop: brainEvents(app).some((e) => e.event === 'crash-loop') }
      if (s.exit !== null) failures.push(`held over the limit, the supervisor exited ${s.exit}${d.afterSustainedOverLimit.crashLoop ? ' (crash loop)' : ''}`)
    }
    await stopApp(s)
    record({ id: '2', name: 'RSS over --max-memory 1M (--rss-ms 200) → clean restart', pass: failures.length === 0,
      numbers: d.memoryEvent ? `log ${JSON.stringify(d.memoryEvent)} · pid ${d.pid1} → ${d.pid2} · first 200 ${d.msToFirst200AfterDetectStart} ms after watch start · lastExit "${d.statusLastExit?.reason}" · memoryRestarts ${d.memoryRestarts} (${d.afterSustainedOverLimit?.memoryRestarts} after 15 s over the limit, supervisor ${d.afterSustainedOverLimit?.supervisorExit === null ? 'still up' : 'exited'})` : '—',
      data: d, failures, owner: 'TOOLCHAIN' })
  }

  // ---------- 3: reload under load ----------
  // 3p/3bp (info): the same with a candidate drain in dist/server.mjs, as evidence for the RUNTIME fix: stop accepting
  // with net.Server#close (http.Server#close on Node ≥ 19 also destroys idle and just-accepted connections) and answer
  // every request that still arrives on an old connection with `Connection: close`.
  const PATCH_FROM = '    s.close(() => process.exit(0))\n    s.closeIdleConnections()\n'
  const PATCH_TO = "    s.prependListener('request', (q, r) => { r.shouldKeepAlive = false })\n    __qaNet.Server.prototype.close.call(s, () => process.exit(0))\n"
  for (const [id, keepAlive, patch] of [['3', true, false], ['3b', false, false], ['3p', true, true], ['3bp', false, true]]) {
    if (!run(id) || (patch && !ONLY)) continue // 3p/3bp only on request: RUNTIME merged the drain fix
    const app = cloneSrv(`reload-${id}`)
    if (patch) {
      const f = join(app, 'dist/server.mjs'), src = readFileSync(f, 'utf8')
      if (!src.includes(PATCH_FROM)) { record({ id, name: 'drain patch', pass: null, numbers: 'skipped: drain code changed', failures: [] }); continue }
      writeFileSync(f, "import * as __qaNet from 'node:net'\n" + src.replace(PATCH_FROM, PATCH_TO))
    }
    const port = await freePort(), failures = []
    const s = startApp(app, port, ['--instances', '2', '--health-ms', '600000'])
    let d = {}, d0 = []
    if ((await waitHealth(port)) == null) failures.push('never up')
    else {
      const agent = keepAlive ? new http.Agent({ keepAlive: true, maxSockets: CLIENTS }) : false
      const samples = [] // [startedAt, ms, ok, detail]
      let loading = true
      const one = () => new Promise((ok) => {
        const t = now()
        const r = http.get({ host: '127.0.0.1', port, path: '/', agent, timeout: 15_000, headers: keepAlive ? {} : { connection: 'close' } }, (res) => {
          res.resume()
          res.on('end', () => ok([t, now() - t, res.statusCode >= 200 && res.statusCode < 300, res.statusCode]))
          res.on('error', (e) => ok([t, now() - t, false, e.code || e.message]))
        })
        r.on('timeout', () => r.destroy(new Error('timeout')))
        r.on('error', (e) => ok([t, now() - t, false, e.code || e.message]))
      })
      const client = async () => { while (loading) samples.push(await one()) }
      const workers = Array.from({ length: CLIENTS }, client)
      await sleep(1000)
      const windows = []
      for (let k = 0; k < RELOADS; k++) {
        const pidsBefore = (status(app)?.instances ?? []).map((i) => i.pid)
        const nReloaded = brainEvents(app).filter((e) => e.event === 'reloaded' || e.event === 'reload-failed').length
        const t0 = now()
        const out = await brainAsync(app, ['reload'])
        let t1 = null, outcome = null
        while (now() - t0 < 45_000) {
          const evs = brainEvents(app).filter((e) => e.event === 'reloaded' || e.event === 'reload-failed')
          if (evs.length > nReloaded) { outcome = evs.at(-1).event; break }
          await sleep(10)
        }
        // window ends when the old children have exited (drained) as well
        while (now() - t0 < 60_000 && pidsBefore.some(alive)) await sleep(10)
        t1 = now()
        const pidsAfter = (status(app)?.instances ?? []).map((i) => i.pid)
        windows.push({ reload: k + 1, cli: out.out.trim(), cliExit: out.code, outcome, ms: r1(t1 - t0), pidsBefore, pidsAfter, t0, t1 })
        if (outcome !== 'reloaded') failures.push(`reload ${k + 1}: outcome ${outcome}`)
        if (out.code !== 0 || !/"reload":"done"/.test(out.out)) failures.push(`reload ${k + 1}: CLI exit ${out.code} ${out.out.trim()}`)
        if (pidsAfter.some((p) => pidsBefore.includes(p)) || pidsAfter.length !== 2) failures.push(`reload ${k + 1}: pids ${pidsBefore} → ${pidsAfter}`)
        await sleep(1000)
      }
      loading = false
      await Promise.all(workers)
      if (agent) agent.destroy()
      const inWin = (x) => windows.some((w) => x[0] + x[1] >= w.t0 && x[0] <= w.t1)
      const fails = samples.filter((x) => !x[2]), during = samples.filter(inWin)
      d0 = windows.map((w) => { const f = fails.filter((x) => x[0] + x[1] >= w.t0 && x[0] <= w.t1); return f.length ? { reload: w.reload, failed: f.length, firstMsAfterReload: r1(Math.min(...f.map((x) => x[0] + x[1])) - w.t0), lastMsAfterReload: r1(Math.max(...f.map((x) => x[0] + x[1])) - w.t0) } : { reload: w.reload, failed: 0 } })
      const errKinds = {}
      for (const f of fails) errKinds[f[3]] = (errKinds[f[3]] ?? 0) + 1
      const lat = samples.map((x) => x[1]), latDuring = during.map((x) => x[1])
      d = {
        clients: CLIENTS, reloads: windows.map(({ t0, t1, ...w }) => w), total: samples.length, failed: fails.length, failedKinds: errKinds,
        duringReload: { requests: during.length, failed: during.filter((x) => !x[2]).length, p50: r1(pct(latDuring, 50)), p99: r1(pct(latDuring, 99)), max: r1(max(latDuring)) },
        failuresPerReload: d0,
        overall: { p50: r1(pct(lat, 50)), p99: r1(pct(lat, 99)), max: r1(max(lat)) },
      }
      writeFileSync(join(RAW, `${id}-requests.json`), JSON.stringify(samples.map((x) => [r1(x[0]), r1(x[1]), x[2], x[3]])))
      if (fails.length) failures.push(`${fails.length} failed requests of ${samples.length}: ${JSON.stringify(errKinds)}`)
    }
    await stopApp(s)
    record({ id, name: `brain reload ×${RELOADS} under load (${CLIENTS} ${keepAlive ? 'keep-alive' : 'Connection: close'} clients, --instances 2)${patch ? ' — candidate drain patch (info)' : ''}`, pass: patch ? null : failures.length === 0,
      numbers: d.total ? `${d.total} requests, ${d.failed} failed · during reloads: ${d.duringReload.requests} req, p99 ${d.duringReload.p99} ms (p50 ${d.duringReload.p50}, max ${d.duringReload.max}) · overall p99 ${d.overall.p99} ms` : '—',
      data: { keepAlive, patch, ...d }, failures: patch ? [] : failures, owner: 'RUNTIME (server.mjs drain)' })
  }

  // ---------- 3r: reload to a build whose child crashes at startup, under load ----------
  if (run('3r')) {
    const app = cloneSrv('reload-fail'), port = await freePort(), failures = []
    const s = startApp(app, port, ['--instances', '2', '--health-ms', '600000'])
    let d = {}
    if ((await waitHealth(port)) == null) failures.push('never up')
    else {
      const pidsBefore = (status(app)?.instances ?? []).map((i) => i.pid)
      const lg = loadGen(port, CLIENTS, true)
      await sleep(1000)
      const f = join(app, 'dist/server.mjs'), good = readFileSync(f, 'utf8')
      writeFileSync(f, "throw new Error('qa: new build crashes at startup')\n")
      const t0 = now()
      const out = await brainAsync(app, ['reload'])
      const t1 = now()
      await sleep(1000)
      const samples = await lg.stop()
      writeFileSync(f, good)
      const pidsAfter = (status(app)?.instances ?? []).map((i) => i.pid)
      const health = await req(port, '/_health')
      const fails = samples.filter((x) => !x[2]), during = samples.filter((x) => x[0] + x[1] >= t0 && x[0] <= t1)
      const kinds = {}
      for (const x of fails) kinds[x[3]] = (kinds[x[3]] ?? 0) + 1
      let reply = null; try { reply = JSON.parse(out.out.trim()) } catch {}
      d = { cliExit: out.code, cli: out.out.trim(), reloadMs: r1(t1 - t0), pidsBefore, pidsAfter, total: samples.length, failed: fails.length, failedKinds: kinds,
        duringReload: { requests: during.length, failed: during.filter((x) => !x[2]).length, p99: r1(pct(during.map((x) => x[1]), 99)) },
        events: brainEvents(app).filter((e) => /reload/.test(e.event)) }
      writeFileSync(join(RAW, '3r-requests.json'), JSON.stringify(samples.map((x) => [r1(x[0]), r1(x[1]), x[2], x[3]])))
      if (out.code !== 1) failures.push(`brain reload exit ${out.code}, expected 1`)
      if (reply?.ok !== false) failures.push(`reply ${out.out.trim()}, expected ok:false`)
      if (JSON.stringify(pidsAfter) !== JSON.stringify(pidsBefore)) failures.push(`instances changed ${pidsBefore} → ${pidsAfter}`)
      if (health.status !== 200) failures.push(`/_health after the failed reload → ${health.status}`)
      if (fails.length) failures.push(`${fails.length} failed requests of ${samples.length}: ${JSON.stringify(kinds)}`)
      if (s.exit !== null) failures.push(`supervisor exited ${s.exit}`)
    }
    await stopApp(s)
    record({ id: '3r', name: `brain reload to a build that crashes at startup, under load (${CLIENTS} keep-alive clients)`, pass: failures.length === 0,
      numbers: d.total ? `reload exit ${d.cliExit} after ${d.reloadMs} ms, ${d.cli} · instances ${d.pidsBefore} kept · ${d.total} requests, ${d.failed} failed (${d.duringReload.requests} during the reload, p99 ${d.duringReload.p99} ms)` : '—',
      data: d, failures, owner: 'TOOLCHAIN' })
  }

  // ---------- 4a: throws at startup, always ----------
  if (run('4a')) {
    const app = cloneSrv('crash-start'), port = await freePort(), failures = []
    writeFileSync(join(app, 'dist/server.mjs'), "throw new Error('qa: boom at startup')\n")
    const s = startApp(app, port)
    while (s.exit === null && now() - s.t0 < 120_000) await sleep(50)
    const ms = s.exit === null ? null : s.exitAt - s.t0
    const evs = brainEvents(app), restarts = evs.filter((e) => e.event === 'restart').length
    const logText = existsSync(join(app, 'logs/app.log')) ? readFileSync(join(app, 'logs/app.log'), 'utf8') : ''
    const d = { exitCode: s.exit, seconds: ms && r1(ms / 1000), restarts, events: evs.map((e) => e.event), stderr: s.err.trim().slice(0, 2000), logHasError: logText.includes('qa: boom at startup') }
    const firstExit = evs.find((e) => e.event === 'exit')
    d.firstExitReason = firstExit?.reason ?? null
    if (s.exit !== 1) failures.push(`exit code ${s.exit}, expected 1`)
    if (!d.logHasError) failures.push('logs/app.log does not contain the child error')
    if (!/crash loop|died before ready/i.test(s.err)) failures.push(`stderr does not explain the failure (stderr: ${JSON.stringify(s.err.trim().slice(0, 300))})`)
    if (!s.err.includes('qa: boom at startup')) failures.push('stderr does not include the last log lines (the child error)')
    if (!/died before ready/.test(d.firstExitReason ?? '')) failures.push(`first exit reason "${d.firstExitReason}", expected "died before ready…"`)
    // QA decision (2026-10-13): §34 asks for detection + crash loop + explanation, not a deadline; exponential backoff
    // inside the 60 s crash-loop window is acceptable (it tolerates transient startup failures).
    if (ms == null || ms >= 65_000) failures.push(`gave up after ${ms == null ? '—' : r1(ms / 1000)} s, expected inside the 60 s crash-loop window`)
    if (restarts === 0) failures.push('no restart attempted (the death before ready was not detected)')
    if (!evs.some((e) => e.event === 'crash-loop')) failures.push('no crash-loop event')
    await stopApp(s)
    record({ id: '4a', name: 'child dies before ready, always → crash loop, exit 1 within the 60 s window, stderr explains', pass: failures.length === 0,
      numbers: `exit ${s.exit} after ${d.seconds} s · ${restarts} restarts · events ${JSON.stringify(d.events)}`, data: d, failures, owner: 'TOOLCHAIN' })
  }

  // ---------- 4b: ready once, then every restart crashes (crash-loop path, default backoff) ----------
  if (run('4b')) {
    const app = cloneSrv('crash-loop'), port = await freePort(), failures = []
    // the real server, with a prologue: the first boot serves for 500 ms then exits 3; every later boot throws
    const real = readFileSync(join(app, 'dist/server.mjs'), 'utf8')
    writeFileSync(join(app, 'dist/server.mjs'), `import * as __qaFs from 'node:fs'
if (__qaFs.existsSync('qa-booted')) throw new Error('qa: crash on restart')
__qaFs.writeFileSync('qa-booted', '1')
setTimeout(() => { console.error('qa: first child dies'); process.exit(3) }, 500)
` + real)
    const s = startApp(app, port)
    const up = await waitHealth(port)
    const tUp = now()
    while (s.exit === null && now() - s.t0 < 150_000) await sleep(50)
    const evs = brainEvents(app), restarts = evs.filter((e) => e.event === 'restart').length
    const d = { exitCode: s.exit, secondsFromStart: s.exitAt && r1((s.exitAt - s.t0) / 1000), secondsFromReady: s.exitAt && up != null && r1((s.exitAt - tUp) / 1000), restarts, exits: evs.filter((e) => e.event === 'exit').length, crashLoopEvent: evs.find((e) => e.event === 'crash-loop') ?? null, stderrHead: s.err.trim().split('\n').slice(0, 3), stderrHasChildError: s.err.includes('qa: crash on restart') }
    if (up == null) failures.push('first child never served')
    if (s.exit !== 1) failures.push(`exit code ${s.exit}, expected 1`)
    if (!d.crashLoopEvent) failures.push('no crash-loop event in logs/app.log')
    if (!/crash loop/.test(s.err)) failures.push('stderr does not say "crash loop"')
    if (!d.stderrHasChildError) failures.push('stderr (last 50 log lines) does not show the child error')
    await stopApp(s)
    record({ id: '4b', name: 'ready once, then every restart crashes → crash loop, exit 1', pass: failures.length === 0,
      numbers: `exit ${s.exit} after ${d.secondsFromStart} s (${d.secondsFromReady} s after ready) · ${restarts} restarts / ${d.exits} exits`, data: d, failures, owner: 'TOOLCHAIN' })
  }

  // ---------- 5: --install output (temp HOME; it must write only inside the app) ----------
  if (run('5')) {
    const app = cloneSrv('install'), failures = [], home = join(WORK, 'home')
    mkdirSync(home, { recursive: true })
    const r = brain(app, ['start', '--install', '--port', '3999', '--instances', '2', '--max-memory', '300M'], { HOME: home })
    const file = (/^wrote (.+)$/m.exec(r.out) || [])[1]
    const body = file && existsSync(file) ? readFileSync(file, 'utf8') : ''
    const homeFiles = readdirSync(home, { recursive: true })
    const exe = (/<key>ProgramArguments<\/key>\s*<array>\s*<string>([^<]+)<\/string>/.exec(body) || [])[1]
    const nodeDir = dirname(execFileSync('which', ['node']).toString().trim())
    const launchdPath = ['/usr/bin', '/bin', '/usr/sbin', '/sbin']
    // EnvironmentVariables of the plist (launchd gives the agent exactly these, plus HOME/USER/…)
    const plistEnv = {}
    const envDict = (/<key>EnvironmentVariables<\/key>\s*<dict>([\s\S]*?)<\/dict>/.exec(body) || [])[1] ?? ''
    for (const m of envDict.matchAll(/<key>([^<]+)<\/key><string>([^<]*)<\/string>/g)) plistEnv[m[1]] = m[2].replace(/&amp;/g, '&').replace(/&lt;/g, '<').replace(/&gt;/g, '>').replace(/&quot;/g, '"')
    const d = { exit: r.code, stdout: r.out.trim(), plistEnv, file, homeFiles, body, exe, exeIsWorkCopy: exe === BRAIN, nodeDir, launchdDefaultPath: launchdPath.join(':') }
    const checks = {
      'exit 0': r.code === 0,
      // macOS: /var is a symlink to /private/var, so compare canonical paths.
      'file written inside the app (.brain/run/)': !!file && realpathSync(file).startsWith(realpathSync(join(app, '.brain/run'))),
      'nothing written under HOME': homeFiles.length === 0,
      'absolute brain path that exists': !!exe && exe.startsWith('/') && existsSync(exe),
      'runs `start` with the given flags': /<string>start<\/string>\s*<string>--port<\/string>\s*<string>3999<\/string>\s*<string>--instances<\/string>\s*<string>2<\/string>/.test(body),
      'WorkingDirectory is the app root': (() => { const m = body.match(/<key>WorkingDirectory<\/key><string>([^<]*)<\/string>/); return !!m && realpathSync(m[1]) === realpathSync(app) })(),
      'KeepAlive true (restart policy)': /<key>KeepAlive<\/key><true\/>/.test(body),
      'RunAtLoad true (boot)': /<key>RunAtLoad<\/key><true\/>/.test(body),
      'no secret from .brain_env inlined': !body.includes('sk-qa-should-not-leak') && !/SECRET_TOKEN/.test(body),
      'EnvironmentVariables PATH with node\'s directory first': plistEnv.PATH?.split(':')[0] === nodeDir,
      'prints the one command to enable it, no sudo': /launchctl load -w/.test(r.out) && !/sudo/.test(r.out),
    }
    // what launchd would actually run: the plist's argv (another port) with only the plist's environment + HOME/USER
    {
      const port = await freePort()
      const env = { PATH: launchdPath.join(':'), ...plistEnv, HOME: home, USER: process.env.USER ?? '' }
      const sv = startApp(app, port, ['--instances', '2', '--max-memory', '300M'], env, true)
      const up = await waitHealth(port, 8000)
      await sleep(200)
      d.underLaunchdEnv = { env, up: up != null, supervisorExit: sv.exit, events: brainEvents(app).map((e) => e.event + (e.error ? ': ' + e.error : '')) }
      await stopApp(sv)
      checks['server app comes up with only the plist\'s environment (launchd)'] = up != null
    }
    // systemd: the Linux branch is compiled out on macOS, so it is reviewed in the source the binary was built from
    {
      const rs = readFileSync(join(ROOT, 'crates/brain-cli/src/start.rs'), 'utf8')
      const unit = (/let unit = format!\(\s*"([^"\\]*(?:\\.[^"\\]*)*)"/.exec(rs) || [])[1] ?? ''
      d.systemdTemplate = unit.replace(/\\n/g, '\n')
      checks['systemd (source): User= line, never root'] = /\{user_line\}/.test(unit) && /User=\{u\}/.test(rs) && /u != "root"/.test(rs)
      checks['systemd (source): Environment="PATH=…" quoted'] = /\{env\}/.test(unit) && /Environment=\{\}\\n", sd_quote\(&format!\("PATH=\{path\}"\)\)/.test(rs)
      checks['systemd (source): quoted ExecStart and WorkingDirectory'] = /WorkingDirectory=\{\}/.test(unit) && /sd_quote\(&root_s\)/.test(rs) && /argv\.iter\(\)\.map\(\|a\| sd_quote\(a\)\)/.test(rs)
      checks['systemd (source): Restart=always, ExecReload SIGHUP, TimeoutStopSec 15'] = /Restart=always/.test(unit) && /ExecReload=\/bin\/kill -HUP \$MAINPID/.test(unit) && /TimeoutStopSec=15/.test(unit)
    }
    for (const [k, ok] of Object.entries(checks)) if (!ok) failures.push(k)
    d.checks = checks
    writeFileSync(join(RAW, '5-install.plist'), body)
    record({ id: '5', name: '--install reviewed (macOS plist run for real; systemd unit from source)', pass: failures.length === 0,
      numbers: `${Object.values(checks).filter(Boolean).length}/${Object.keys(checks).length} checks`, data: d, failures, owner: 'TOOLCHAIN' })
  }

  // ---------- 6: static app, served from Rust ----------
  if (run('6')) {
    const port = await freePort(), failures = []
    const s = startApp(STAT, port)
    let d = {}
    if ((await waitHealth(port)) == null) failures.push('never up')
    else {
      const home = await req(port, '/'), notMod = await req(port, '/', { 'if-none-match': home.headers.etag ?? '' })
      const health = await req(port, '/_health'), hdrFile = await req(port, '/_headers')
      for (let i = 0; i < 20; i++) await req(port, i % 2 ? '/about' : '/')
      // process tree of the supervisor: no node anywhere below it
      const ps = execFileSync('ps', ['-A', '-o', 'pid=,ppid=,comm=']).toString().trim().split('\n').map((l) => l.trim().split(/\s+/, 3))
      const desc = [], q = [String(s.pid)]
      while (q.length) { const p = q.shift(); for (const [pid, ppid, comm] of ps) if (ppid === p) { desc.push({ pid: +pid, comm }); q.push(pid) } }
      const st = status(STAT)
      // _headers: every header of the rules that match `/` must be on the response
      const rules = readFileSync(join(STAT, 'dist/_headers'), 'utf8')
      const expected = {}
      let cur = null
      for (const line of rules.split('\n')) {
        if (!line.trim() || line.trim().startsWith('#')) continue
        if (!/^\s/.test(line)) { cur = line.trim(); continue }
        const m = /^\s+([^:]+):\s*(.*)$/.exec(line)
        if (m && (cur === '/*' || cur === '/')) expected[m[1].toLowerCase()] = m[2].trim()
      }
      const missing = Object.entries(expected).filter(([k, v]) => k !== 'content-security-policy' ? home.headers[k] !== v : !home.headers[k])
      let healthJson = null; try { healthJson = JSON.parse(health.body) } catch {}
      d = {
        signaturePrinted: SIG_STAT, etag: home.headers.etag, status304: notMod.status, healthBody: health.body, descendants: desc,
        expectedHeaders: expected, missingHeaders: missing.map(([k, v]) => ({ header: k, expected: v, got: home.headers[k] ?? null })),
        headersFileStatus: hdrFile.status, statusMode: st?.mode, statusInstances: st?.instances,
      }
      if (desc.some((x) => /node/i.test(x.comm))) failures.push(`node process under the supervisor: ${JSON.stringify(desc)}`)
      if (st?.mode !== 'static') failures.push(`status mode ${st?.mode}`)
      if (home.status !== 200) failures.push(`GET / → ${home.status}`)
      if (missing.length) failures.push(`_headers not applied: ${JSON.stringify(d.missingHeaders)}`)
      if (home.headers.etag !== `"${SIG_STAT}"`) failures.push(`ETag ${home.headers.etag} ≠ "${SIG_STAT}" (signature printed by brain build)`)
      if (notMod.status !== 304) failures.push(`If-None-Match → ${notMod.status}, expected 304`)
      if (notMod.body.length) failures.push('304 has a body')
      if (!(healthJson && healthJson.ok === true && healthJson.build === SIG_STAT)) failures.push(`/_health → ${health.status} ${health.body}`)
      if (hdrFile.status !== 404) failures.push(`/_headers served (${hdrFile.status})`)
    }
    await stopApp(s)
    if (s.exit !== 0) failures.push(`brain stop → exit ${s.exit}`)
    record({ id: '6', name: 'static app served without Node', pass: failures.length === 0,
      numbers: d.etag ? `ETag ${d.etag} = build ${SIG_STAT} · 304 ${d.status304} · /_health ${d.healthBody} · ${d.descendants.length} child processes · ${Object.keys(d.expectedHeaders).length} _headers checked` : '—',
      data: d, failures, owner: 'TOOLCHAIN' })
  }
} finally {
  await cleanup()
}

// ---------- leftovers: nothing we started may still run ----------
const leftovers = started.filter((s) => alive(s.pid)).map((s) => s.pid)
const summary = { date: new Date().toISOString(), brainSha: SHA, firstExecMs: WARM_MS, brainSource: src.replace(homedir(), '~'), node: NODE_V, platform: `${process.platform} ${process.arch}`, work: WORK.replace(homedir(), '~'), signatures: { srv: SIG_SRV, stat: SIG_STAT }, leftovers, cases }
// Committed output: never leak this machine's home directory (PATH, temp dirs).
writeFileSync(join(OUT, 'summary.json'), JSON.stringify(summary, null, 2).replaceAll(homedir(), '~') + '\n')
const md = [`# brain start QA (#120) — ${summary.date.slice(0, 10)}`, '', `binary sha256 \`${SHA}\` · node ${NODE_V} · ${summary.platform}`, '',
  '| Case | Result | Numbers |', '|---|---|---|',
  ...cases.map((c) => `| ${c.id} ${c.name} | ${c.pass === null ? 'info' : c.pass ? 'PASS' : '**FAIL**'} | ${c.numbers.replace(/\|/g, '\\|')} |`), '',
  ...cases.filter((c) => c.failures?.length).flatMap((c) => [`## ${c.id} failures (owner ${c.owner})`, ...c.failures.map((f) => `- ${f}`), '']),
  `Leftover processes: ${leftovers.length ? leftovers.join(', ') : 'none'}`, ''].join('\n')
writeFileSync(join(OUT, 'summary.md'), md.replaceAll(homedir(), '~'))
console.log(`\n${md}`)
process.exit(cases.some((c) => c.pass === false) || leftovers.length ? 1 : 0)
