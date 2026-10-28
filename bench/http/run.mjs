// HTTP client bench (QA #172, INTERNALS §40 and §41.4): every behaviour of the api() client reproduced in real headless
// Chrome against a fault-injection server (server.mjs), on the app in bench/http/app served two ways:
//   dev  — `brain run` behind the fault server (one origin: the server proxies the app and answers /f/* itself)
//   prod — `brain build`, dist/ served statically by the fault server
// Each scenario asserts from both the DOM (<output data-r>, rendered states) and the server's request log.
//
// A scenario is BLOCKED (not FAIL) when the feature it needs does not compile or is not wired yet: its source files
// fail `brain check` (they are left out of the working copy so the rest still runs), or `<meta http>` is emitted
// verbatim instead of being compiled. Compiler-check scenarios (C*) run `brain check` on probe files.
//
// Usage: node bench/http/run.mjs [--modes dev,prod] [--port 5198] [--only S01,S07] [--no-build]
// Env:   BRAIN (binary, default target/debug/brain) · BRAIN_CHROME (see bench/lib/cdp.mjs)
// Output: results/summary.json + summary.md (committed) · results/raw/<mode>.json (gitignored)
// Exit:  0 no FAIL (BLOCKED allowed) · 1 any scenario FAILs · 2 the bench itself could not run
import { spawn, execFileSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import { cpSync, existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { join, relative } from 'node:path'
import { launch, openPage } from '../lib/cdp.mjs'
import { createFaultServer } from './server.mjs'

const HERE = new URL('.', import.meta.url).pathname
const ROOT = new URL('../..', import.meta.url).pathname
const SRC = join(HERE, 'app')
const WORK = join(HERE, '.work')
const OUT = join(HERE, 'results')
const BRAIN = process.env.BRAIN ?? join(ROOT, 'target/debug/brain')
const argv = process.argv.slice(2)
const arg = (k, d) => { const i = argv.indexOf(`--${k}`); return i >= 0 ? argv[i + 1] : d }
const MODES = arg('modes', 'dev,prod').split(',')
const DEV_PORT = Number(arg('port', 5198))
const ONLY = arg('only', null)?.split(',')
const NO_BUILD = argv.includes('--no-build')
const sleep = (ms) => new Promise((r) => setTimeout(r, ms))
const t0 = Date.now()

// ---------- children are killed by their own handles only ----------
const children = new Set()
const cleanup = () => { for (const c of children) { try { c.kill('SIGTERM') } catch {} } children.clear() }
process.on('exit', cleanup)
process.on('SIGINT', () => (cleanup(), process.exit(130)))
process.on('SIGTERM', () => (cleanup(), process.exit(143)))
function fatal(msg) { console.error('http bench: ' + msg); cleanup(); process.exit(2) }
if (!existsSync(BRAIN)) fatal(`no binary at ${BRAIN}; run \`cargo build -p brain-cli\` first`)
const brainSha = createHash('sha256').update(readFileSync(BRAIN)).digest('hex').slice(0, 12)

const isError = (d) => !/^(warn|warning|info|hint|note)$/i.test(String(d.severity ?? d.level ?? 'error'))
const brainCheck = (dir) => {
  let s
  try { s = execFileSync(BRAIN, ['check', '--json', '.'], { cwd: dir, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] }) } catch (e) { s = e.stdout }
  return JSON.parse(s || '[]')
}

// ---------- scripted answers (server.mjs answer format) ----------
const ITEMS = (n) => [{ id: 1, name: 'a' }, { id: 3, name: 'third' }, { id: 4, name: 'd' }].slice(0, n)
const KINDS = {
  'GET /f/k404': [{ status: 404, json: { error: 'Not found' } }],
  'GET /f/k403': [{ status: 403, json: { error: 'orders.read' } }],
  'POST /f/k422': [{ status: 422, json: { error: 'Invalid input', errors: { email: 'Invalid email' } } }],
  'GET /f/k429long': [{ status: 429, retryAfter: 60 }],
  'GET /f/k503long': [{ status: 503, retryAfter: 60, json: { error: 'Maintenance' } }],
  'GET /f/typed': [{ json: { not: 'a list' } }],
  'POST /f/reset': [{ reset: true }],
  'GET /f/reset3': [{ reset: true }],
  'POST /f/slow': [{ delay: 15000 }],
  'GET /f/flaky': [{ status: 502 }, { status: 504 }, { json: { ok: 1 } }],
  'PUT /f/flaky-put': [{ status: 503 }, { json: { ok: 1 } }],
  'DELETE /f/flaky-delete': [{ reset: true }, { status: 204 }],
  'POST /f/k503': [{ status: 503 }],
  'PATCH /f/k502': [{ status: 502 }],
  'GET /f/ra429': [{ status: 429, retryAfter: 1 }, { json: { ok: 1 } }],
  'GET /f/ra503': [{ status: 503, retryAfter: 1 }, { json: { ok: 1 } }],
  'GET /f/hang': [{ hang: true }],
  'GET /f/hang2': [{ hang: true }],
}

// ---------- assertions ----------
function A() {
  const fails = []
  const ok = (cond, msg) => { if (!cond) fails.push(msg); return !!cond }
  return { ok, fails, eq: (got, want, what) => ok(JSON.stringify(got) === JSON.stringify(want), `${what}: got ${JSON.stringify(got)}, want ${JSON.stringify(want)}`) }
}
const calls = (L, key) => L.log.filter((e) => `${e.method} ${e.path.split('?')[0]}` === key)
const gaps = (cs) => cs.slice(1).map((c, i) => c.t0 - cs[i].t1)
const RT = 'RUNTIME runtime/brain-data.js'

// One entry per behaviour of §40/§41.4. `needs`: source files (a compile error there → BLOCKED); `features`: wiring
// detected at runtime (`meta-http`). `modes`: where it applies (default both).
const SCENARIOS = [
  { id: 'S01', title: 'HTTP status → ApiError kind http, status, message, x-request-id, is(403)', owner: RT, needs: ['views/kinds.brain'], plan: KINDS, view: '/kinds',
    async run(c, a) {
      const r404 = await c.act('get404'), r403 = await c.act('get403')
      const L = await c.log(), c404 = calls(L, 'GET /f/k404'), c403 = calls(L, 'GET /f/k403')
      a.eq([r404.kind, r404.status, r404.message, r404.is403], ['http', 404, 'Not found', false], '404')
      a.eq([r403.kind, r403.status, r403.is403], ['http', 403, true], '403')
      a.eq([c404.length, c403.length], [1, 1], 'requests (404/403 are never retried)')
      a.eq([r404.id, r403.id], [c404.at(-1)?.rid, c403.at(-1)?.rid], 'e.id = the response x-request-id')
      return `404 ${r404.kind}/${r404.status} id ${r404.id}; 403 is(403)=${r403.is403}; requests ${c404.length}+${c403.length}`
    } },
  { id: 'S02', title: '422 → e.errors per field', owner: RT, needs: ['views/kinds.brain'], plan: KINDS, view: '/kinds',
    async run(c, a) {
      const r = await c.act('post422'), cs = calls(await c.log(), 'POST /f/k422')
      a.eq([r.kind, r.status, r.errors], ['http', 422, { email: 'Invalid email' }], '422')
      a.eq(cs.length, 1, 'requests'); a.ok(/application\/json/.test(cs[0]?.ct), 'POST body sent as JSON')
      return `422 errors ${JSON.stringify(r.errors)}`
    } },
  { id: 'S03', title: '429/503 with Retry-After > 10 s → not retried, e.retryAfter', owner: RT, needs: ['views/kinds.brain'], plan: KINDS, view: '/kinds',
    async run(c, a) {
      const r1 = await c.act('get429long'), r2 = await c.act('get503long'), L = await c.log()
      a.eq([r1.status, r1.retryAfter, r2.status, r2.retryAfter], [429, 60, 503, 60], 'status/retryAfter')
      a.eq([calls(L, 'GET /f/k429long').length, calls(L, 'GET /f/k503long').length], [1, 1], 'requests')
      a.eq(r2.id, calls(L, 'GET /f/k503long')[0]?.rid, '503 e.id')
      return `429 retryAfter ${r1.retryAfter} · 503 retryAfter ${r2.retryAfter} id ${r2.id}; 1 request each`
    } },
  { id: 'S04', title: "response not matching `out` → kind 'schema'", owner: RT, needs: ['views/kinds.brain'], plan: KINDS, view: '/kinds',
    async run(c, a) {
      const r = await c.act('schema')
      a.eq(r.kind, 'schema', 'kind'); a.eq(calls(await c.log(), 'GET /f/typed').length, 1, 'requests')
      return `kind ${r.kind}: ${String(r.message).slice(0, 80)}`
    } },
  { id: 'S05', title: "connection reset → kind 'network'; GET retried ×2, POST never", owner: RT, needs: ['views/kinds.brain'], plan: KINDS, view: '/kinds',
    async run(c, a) {
      const p = await c.act('postReset'), g = await c.act('getReset'), L = await c.log()
      a.eq([p.kind, g.kind], ['network', 'network'], 'kinds')
      a.eq([calls(L, 'POST /f/reset').length, calls(L, 'GET /f/reset3').length], [1, 3], 'requests POST/GET')
      return `POST ${p.kind} ×${calls(L, 'POST /f/reset').length} · GET ${g.kind} ×${calls(L, 'GET /f/reset3').length}`
    } },
  { id: 'S06', title: "no answer → kind 'timeout' at the policy timeout (default 10 s)", owner: RT, needs: ['views/kinds.brain'], plan: KINDS, view: '/kinds',
    async run(c, a) {
      const T = c.feature.metaHttp ? 4000 : 10000
      const r = await c.act('timeout', T + 6000), cs = calls(await c.log(), 'POST /f/slow')
      a.eq(r.kind, 'timeout', 'kind')
      a.ok(r.ms >= T - 300 && r.ms <= T + 2000, `timed out after ${r.ms} ms, want ~${T} ms`)
      a.eq(cs.length, 1, 'requests (POST is never retried)'); a.ok(cs[0]?.aborted, 'the server saw the request aborted')
      return `kind ${r.kind} after ${r.ms} ms (policy ${T} ms${c.feature.metaHttp ? ' from <meta http>' : ', default: <meta http timeout> not wired'})`
    } },
  { id: 'S07', title: 'safe methods retried on 502/503/504/network with 300 ms then 600 ms (±25 %)', owner: RT, needs: ['views/kinds.brain'], plan: KINDS, view: '/kinds',
    async run(c, a) {
      const g = await c.act('getRetry'), p = await c.act('putRetry'), d = await c.act('deleteRetry'), L = await c.log()
      const cg = calls(L, 'GET /f/flaky'), [g1, g2] = gaps(cg)
      a.eq([g.ok, g.value], [true, { ok: 1 }], 'GET value'); a.eq(cg.map((x) => x.status), [502, 504, 200], 'GET attempts')
      a.ok(g1 >= 215 && g1 <= 520, `first backoff ${g1} ms, want 300 ±25 %`); a.ok(g2 >= 440 && g2 <= 900, `second backoff ${g2} ms, want 600 ±25 %`)
      a.eq([p.ok, calls(L, 'PUT /f/flaky-put').length], [true, 2], 'PUT 503 → retried once')
      a.eq([d.ok, calls(L, 'DELETE /f/flaky-delete').length], [true, 2], 'DELETE reset → retried once')
      return `GET 502→504→200 backoffs ${g1}/${g2} ms · PUT ×${calls(L, 'PUT /f/flaky-put').length} · DELETE ×${calls(L, 'DELETE /f/flaky-delete').length}`
    } },
  { id: 'S08', title: 'POST and PATCH are never retried', owner: RT, needs: ['views/kinds.brain'], plan: KINDS, view: '/kinds',
    async run(c, a) {
      const p = await c.act('post503'), q = await c.act('patch502'), L = await c.log()
      a.eq([p.status, q.status], [503, 502], 'statuses')
      a.eq([calls(L, 'POST /f/k503').length, calls(L, 'PATCH /f/k502').length], [1, 1], 'requests')
      return `POST 503 ×${calls(L, 'POST /f/k503').length} · PATCH 502 ×${calls(L, 'PATCH /f/k502').length}`
    } },
  { id: 'S09', title: 'Retry-After (≤ 10 s) honoured on 429 and 503', owner: RT, needs: ['views/kinds.brain'], plan: KINDS, view: '/kinds',
    async run(c, a) {
      const r1 = await c.act('ra429'), r2 = await c.act('ra503'), L = await c.log()
      const [g1] = gaps(calls(L, 'GET /f/ra429')), [g2] = gaps(calls(L, 'GET /f/ra503'))
      a.eq([r1.ok, r2.ok], [true, true], 'eventual success')
      a.eq([calls(L, 'GET /f/ra429').length, calls(L, 'GET /f/ra503').length], [2, 2], 'requests')
      a.ok(g1 >= 950 && g1 <= 1800, `429 wait ${g1} ms, want ~1000`); a.ok(g2 >= 950 && g2 <= 1800, `503 wait ${g2} ms, want ~1000`)
      return `Retry-After 1 s → waited ${g1} ms (429) and ${g2} ms (503)`
    } },
  { id: 'S10', title: ".cancel() rejects with kind 'aborted', finally runs, nothing else surfaces", owner: RT + ' · runtime/brain-guard.js', needs: ['views/kinds.brain'], plan: KINDS, view: '/kinds',
    async run(c, a) {
      await c.click('cancel', 'cancel')
      await c.until(`document.querySelector('[data-r=cancel]').textContent.trim() !== 'pending'`, 3000)
      await c.click('cancelUncaught', 'cancelUncaught'); await sleep(1500)
      const out = await c.out('cancel'), fin = await c.out('cancelFinally'), unc = await c.out('cancelUncaught'), L = await c.log()
      const h1 = calls(L, 'GET /f/hang'), h2 = calls(L, 'GET /f/hang2'), noise = await c.quiet()
      a.eq(out, 'caught aborted', "author's catch"); a.eq(fin, 'yes', "author's finally")
      a.eq(unc, 'pending', 'uncaught cancel: the handler stops at the await')
      a.eq([h1.length, h1[0]?.aborted, h2.length, h2[0]?.aborted], [1, true, 1, true], 'server saw both requests aborted')
      a.eq(noise, [], 'silent abort (no toast, report, ErrorState, {:catch}, console error)')
      return `catch "${out}", finally ${fin}; uncaught cancel silent (${noise.length} surfaces); server aborted ${h1[0]?.aborted}/${h2[0]?.aborted}`
    } },
  { id: 'S11', title: 'identical GETs in flight share one request', owner: RT, needs: ['views/dedupe.brain', 'comp/feed-card.brain'], view: '/dedupe',
    plan: { 'GET /f/feed': [{ delay: 400, json: { n: 7 } }] },
    async run(c, a) {
      const done = await c.until(`[...document.querySelectorAll('[data-card] .state')].map((e) => e.textContent).join('|') === 'n=7|n=7'`, 5000)
      const cs = calls(await c.log(), 'GET /f/feed')
      a.ok(done, 'both cards render n=7: ' + (await c.ev(`[...document.querySelectorAll('[data-card] .state')].map((e) => e.textContent).join('|')`)))
      a.eq(cs.length, 1, 'requests to /f/feed')
      return `2 components, ${cs.length} request(s)`
    } },
  { id: 'S12', title: 'max 6 per origin; a user action goes before waiting background requests', owner: RT, needs: ['views/queue.brain'], view: '/queue',
    plan: { 'GET /f/q': [{ delay: 600 }], 'POST /f/act': [{ json: { ok: 1 } }] },
    async run(c, a) {
      await c.click('burst', 'order')
      await c.until(`(document.querySelector('[data-r=order]').textContent.match(/low\\d|act/g) || []).length === 11`, 8000)
      const L = await c.log(), order = (await c.out('order')).split(',')
      const post = L.log.find((e) => e.method === 'POST'), before = L.log.filter((e) => e.method === 'GET' && e.seq < post?.seq).length
      a.ok(L.peak <= 6, `peak concurrent requests ${L.peak}, want ≤ 6`)
      a.ok(post && before <= 6, `the action arrived after ${before} background GETs, want ≤ 6 (ahead of the waiting ones)`)
      a.ok(order.indexOf('act') >= 0 && order.indexOf('act') < order.indexOf('low9'), 'action settles before the last background request: ' + order.join(','))
      return `peak ${L.peak}; action after ${before} background GETs; settle order ${order.slice(1).join(',')}`
    } },
  { id: 'S13', title: 'client navigation aborts background (prefetch) requests, keeps user ones', owner: RT, needs: ['views/nav.brain', 'views/nav-done.brain'], view: '/nav',
    plan: { 'GET /f/bg': [{ delay: 5000 }], 'GET /f/fg': [{ delay: 1500, json: { ok: 1 } }] },
    async run(c, a) {
      await c.click('start', 'started'); await sleep(300)
      await c.ev(`(window.__same = 1, document.querySelector('[data-a=go]').click(), 1)`)
      const arrived = await c.until(`!!document.querySelector('[data-probe=nav-done]')`, 6000)
      const client = await c.ev(`window.__same === 1`)
      await sleep(1800)
      const L = await c.log(), bg = calls(L, 'GET /f/bg'), fg = calls(L, 'GET /f/fg')
      a.ok(arrived, 'reached /nav-done'); a.ok(client, 'navigation was client-side (no full page load)')
      a.ok(bg.length === 3 && bg.every((e) => e.aborted && e.t1 - e.t0 < 4000), `background: ${bg.map((e) => (e.aborted ? 'aborted@' + (e.t1 - e.t0) : 'kept')).join(',')}`)
      a.ok(fg.length === 1 && !fg[0].aborted && fg[0].status === 200, `user request: ${fg.map((e) => (e.aborted ? 'aborted' : e.status)).join(',')}`)
      const noise = await c.quiet(); a.eq(noise, [], 'aborted background requests stay silent')
      return `client nav=${client}; background ${bg.filter((e) => e.aborted).length}/3 aborted; user request ${fg[0]?.aborted ? 'aborted' : fg[0]?.status}`
    } },
  { id: 'S14', title: 'offline: GET answers from cache (stale), write fails at once, nothing sent', owner: RT, needs: ['views/offline.brain'], view: '/offline',
    plan: { 'GET /f/items': [{ json: ITEMS(2) }] },
    async run(c, a) {
      a.ok(await c.until(`document.querySelector('[data-r=list]')?.textContent === 'items 2'`, 5000), 'list rendered')
      await c.offline(true)
      a.eq(await c.ev('navigator.onLine'), false, 'navigator.onLine under emulation')
      const r = await c.act('read'), w = await c.act('write')
      const L = await c.log() // read while still offline: a refresh after reconnecting is allowed
      await c.offline(false)
      a.eq([r.ok, r.stale, r.n], [true, true, 2], 'read while offline')
      a.eq(w.kind, 'offline', 'write kind'); a.ok(w.ms < 100, `write failed after ${w.ms} ms, want at once`)
      a.eq([calls(L, 'GET /f/items').length, calls(L, 'POST /f/items').length], [1, 0], 'requests reaching the server while offline (GET at mount only)')
      return `read ${JSON.stringify(r)}; GET times ${calls(L, 'GET /f/items').map((e) => e.t0).join(',')} ms; write ${w.kind} in ${w.ms} ms; server saw GET×${calls(L, 'GET /f/items').length} POST×${calls(L, 'POST /f/items').length}`
    } },
  { id: 'S15', title: 'unmount and re-run abort the resource request silently ({:catch} not rendered)', owner: RT, needs: ['views/unmount.brain', 'comp/slow-card.brain'], view: '/unmount',
    plan: { 'GET /f/slowcard': [{ hang: true }], 'GET /f/rerun': [{ hang: true }, { json: { q: 2 } }] },
    async run(c, a) {
      await c.until(`document.querySelector('[data-r=slowcard]')?.textContent === 'loading'`, 4000); await sleep(300)
      await c.click('hide', 'hidden')
      await c.click('bump', 'q'); await c.until(`document.querySelector('[data-r=q]').textContent === '2'`, 2000)
      const loaded = await c.until(`document.querySelector('[data-r=rerun]')?.textContent === 'loaded q=2'`, 4000)
      await sleep(400)
      const L = await c.log(), cs = calls(L, 'GET /f/slowcard'), rr = calls(L, 'GET /f/rerun'), noise = await c.quiet()
      a.eq([cs.length, cs[0]?.aborted], [1, true], 'unmount: one request, aborted')
      a.eq(rr.map((e) => (e.aborted ? 'aborted' : e.status)), ['aborted', 200], 're-run: first run aborted, second answered')
      a.ok(loaded, 're-run renders the second answer: ' + (await c.out('rerun')))
      a.eq(noise, [], 'silent abort (no {:catch}, toast, ErrorState, console error)')
      return `unmount aborted=${cs[0]?.aborted}; re-run ${rr.map((e) => (e.aborted ? 'aborted' : e.status)).join('→')}; error surfaces ${noise.length}`
    } },
  { id: 'S16', title: 'a write re-runs the resources of its path; refetch() forces a reload', owner: RT, needs: ['views/refetch.brain'], view: '/refetch',
    plan: { 'GET /f/items': [{ json: ITEMS(1) }, { json: ITEMS(2) }, { json: ITEMS(3) }], 'POST /f/items': [{ json: { id: 3, name: 'third' } }] },
    async run(c, a) {
      const text = `document.querySelector('[data-r=list]')?.textContent`
      a.ok(await c.until(`${text} === 'items 1'`, 5000), 'first load')
      await c.click('add', 'added'); await c.until(`document.querySelector('[data-r=added]').textContent === 'yes'`, 3000)
      const afterWrite = await c.until(`${text} === 'items 2'`, 3000)
      await c.click('refetch', 'refetched')
      const afterRefetch = await c.until(`${text} === 'items 3'`, 3000)
      const L = await c.log(), gets = calls(L, 'GET /f/items'), post = calls(L, 'POST /f/items')[0]
      a.ok(afterWrite, 'list re-fetched after the write: ' + (await c.ev(text)))
      a.ok(afterRefetch, 'list re-fetched by refetch()')
      a.eq(gets.length, 3, 'GET /f/items count'); a.ok(post && gets[1] && gets[1].t0 >= post.t1, 'second GET after the POST answered')
      return `GETs ${gets.length} (mount, after write, refetch); final "${await c.ev(text)}"`
    } },
  { id: 'S17', title: "upload (in: 'file'): multipart with .progress 0→1", owner: RT, needs: ['views/upload.brain'], view: '/upload',
    plan: { 'POST /f/files': [{ readRate: 4 * 1024 * 1024 }] },
    async run(c, a) {
      await c.click('send', 'send')
      const seen = []
      for (let t = 0; t < 15000; t += 25) {
        const v = await c.out('progress'); if (seen.at(-1) !== v) seen.push(v)
        if ((await c.out('send')) !== 'running') break
        await sleep(25)
      }
      const r = JSON.parse(await c.out('send')), nums = seen.filter((v) => v !== '').map(Number)
      const L = await c.log(), cs = calls(L, 'POST /f/files')
      a.ok(r.ok && r.value?.files?.[0]?.size === 8 * 1024 * 1024 && r.value?.fields?.name === 'big.bin', 'server received the 8 MiB file and the name field: ' + JSON.stringify(r).slice(0, 200))
      a.ok(/multipart\/form-data/.test(cs[0]?.ct), 'sent as multipart')
      a.ok(nums.some((v) => v > 0 && v < 1), `intermediate progress values: ${nums.slice(0, 8).join(',')}`)
      a.ok(nums.at(-1) === 1, 'progress ends at 1'); a.ok(nums.every((v, i) => i === 0 || v >= nums[i - 1]), 'progress never goes back')
      return `${nums.length} progress values (${nums.filter((v) => v > 0 && v < 1).length} between 0 and 1), ${(r.ms / 1000).toFixed(1)} s, ${cs[0]?.bytes} bytes`
    } },
  { id: 'S18', title: "an upload in flight can be cancelled (kind 'aborted', finally runs, silent)", owner: RT, needs: ['views/upload.brain'], view: '/upload',
    plan: { 'POST /f/files': [{ readRate: 1024 * 1024 }] },
    async run(c, a) {
      await c.click('abort', 'abort')
      await c.until(`document.querySelector('[data-r=abort]').textContent.trim() !== 'pending'`, 3000); await sleep(500)
      let cs = []
      for (let t = 0; t < 4000; t += 100) { cs = calls(await c.log(), 'POST /f/files'); if (cs[0]?.aborted) break; await sleep(100) } // the slow reader notices on its next resume
      const out = await c.out('abort'), fin = await c.out('abortFinally'), noise = await c.quiet()
      a.eq([out, fin], ['caught aborted', 'yes'], "author's catch and finally")
      a.eq([cs.length, cs[0]?.aborted], [1, true], 'server saw the upload aborted')
      a.eq(noise, [], 'silent abort')
      return `"${out}", finally ${fin}, aborted at ${cs[0]?.bytes} bytes`
    } },
  { id: 'S19', title: '401 → login?next=<current path> (from <meta http login>)', owner: RT, needs: ['views/login-gate.brain', 'views/login.brain'], features: ['meta-http'], view: '/login-gate',
    plan: { 'GET /f/me': [{ status: 401, json: { error: 'Sign in' } }] },
    async run(c, a) {
      const next = await c.ev('encodeURIComponent(location.pathname + location.search)')
      await c.click('me', 'me')
      const ok = await c.until(`/\\/login\\/?$/.test(location.pathname)`, 5000)
      const loc = await c.ev('location.pathname + location.search')
      a.ok(ok, 'redirected to /login: ' + loc); a.ok(loc.endsWith('?next=' + next), `next = ${next}: ${loc}`)
      return `→ ${loc}`
    } },
  { id: 'S20', title: '<meta http base> prefixes api() paths', owner: 'COMPILER crates/brain-codegen (<meta http>) · TOOLCHAIN', needs: ['views/policy.brain'], features: ['meta-http'], view: '/policy',
    plan: { 'GET /f/policy': [{ json: { ok: 1 } }] },
    async run(c, a) {
      const r = await c.act('ping'), cs = calls(await c.log(), 'GET /f/policy')
      a.eq(r.ok, true, 'answered'); a.ok(cs[0]?.raw.startsWith('/api/f/'), `raw path ${cs[0]?.raw}, want /api/f/policy`)
      return `raw path ${cs[0]?.raw}`
    } },
  { id: 'S21', title: '<ErrorState> renders message + action per kind', owner: 'RUNTIME runtime/ui/error-state.brain · COMPILER (kit registration)', needs: ['views/error-states.brain'], plan: KINDS, view: '/error-states',
    async run(c, a) {
      await c.click('load', 'started')
      await c.until(`!!document.querySelector('[data-r=server] .error-state')`, 8000)
      const t = await c.ev(`Object.fromEntries([...document.querySelectorAll('div[data-r]')].map((d) => [d.dataset.r, { text: d.textContent.replace(/\\s+/g, ' ').trim(), retry: !!d.querySelector('button'), link: d.querySelector('a')?.getAttribute('href') || null }]))`)
      const L = await c.log(), rid = calls(L, 'GET /f/k503long')[0]?.rid
      a.ok(/connection/i.test(t.network?.text) && t.network?.retry, 'network: connection lost + retry: ' + JSON.stringify(t.network))
      a.ok(/permission/i.test(t.forbidden?.text) && /orders\.read/.test(t.forbidden?.text), '403 names the permission: ' + JSON.stringify(t.forbidden))
      a.ok(/not found/i.test(t.missing?.text), '404: ' + JSON.stringify(t.missing))
      a.ok(/field/i.test(t.invalid?.text), '422 points to the fields: ' + JSON.stringify(t.invalid))
      a.ok(/60/.test(t.limited?.text), '429 waits Retry-After: ' + JSON.stringify(t.limited))
      a.ok(t.server?.retry && rid && t.server.text.includes(rid), '5xx generic + request id + retry: ' + JSON.stringify(t.server))
      return Object.entries(t).map(([k, v]) => `${k}: "${v.text.slice(0, 40)}"${v.retry ? ' +retry' : ''}`).join(' · ')
    } },
  { id: 'S22', title: 'online() follows the connection state', owner: 'RUNTIME runtime/brain-data.js · COMPILER (scope)', needs: ['views/online.brain'], view: '/online',
    async run(c, a) {
      const before = await c.out('online'); await c.offline(true)
      const off = await c.until(`document.querySelector('[data-r=online]').textContent === 'offline'`, 2000)
      await c.offline(false)
      const on = await c.until(`document.querySelector('[data-r=online]').textContent === 'online'`, 2000)
      a.eq(before, 'online', 'initial'); a.ok(off, 'turns offline'); a.ok(on, 'turns online again')
      return `online → ${off ? 'offline' : 'stuck'} → ${on ? 'online' : 'stuck'}`
    } },
  { id: 'S23', title: 'api/_http.jsbrain http({ headers }) adds headers to every request', owner: 'COMPILER (http in scope) · TOOLCHAIN (loading _http)', needs: ['views/headers.brain', 'api/_http.jsbrain'], view: '/headers',
    plan: { 'GET /f/headers': [{ json: { ok: 1 } }] },
    async run(c, a) {
      await c.act('ping'); const cs = calls(await c.log(), 'GET /f/headers')
      a.eq(cs[0]?.headers['x-bench'], 'from-http-headers', 'x-bench header at the server')
      return `x-bench=${cs[0]?.headers['x-bench']}`
    } },
  { id: 'S24', title: 'request feed: dev logs every api() call; prod emits none', owner: 'TOOLCHAIN crates/brain-serve', needs: ['views/kinds.brain'], plan: KINDS, view: '/kinds',
    async run(c, a) {
      await c.act('get404'); await sleep(400)
      const net = await c.ev('window.__net || []')
      if (c.mode === 'dev') {
        const line = c.devLog().split('\n').find((l) => l.includes('[net]') && l.includes('/f/k404'))
        a.ok(net.some((n) => /\/f\/k404$/.test(n.url) && n.status === 404 && n.method === 'GET'), 'brain:net event in the page: ' + JSON.stringify(net).slice(0, 160))
        a.ok(line && /404/.test(line), '`brain run` printed a [net] line: ' + (line || 'none'))
        return `page events ${net.length}; terminal: ${line?.trim() || 'none'}`
      }
      a.eq(net.length, 0, 'brain:net events in production')
      return `brain:net events in prod: ${net.length}`
    } },
]

// Compiler checks of §40.5/§41.4 on probe files (mode-independent): BLOCKED while the check does not exist.
const CHECKS = [
  { id: 'C1', title: 'B1104: unknown attribute or value in <meta http>', owner: 'COMPILER crates/brain-check',
    files: { 'app.brain': (s) => s.replace('<meta http base="/api" timeout="4s" retry="2" login="/login">', '<meta http base="/api" timeout="soon" bogus="1">') },
    match: (d) => d.code === 'B1104' },
  { id: 'C2', title: 'warning: fetch() used directly in app code (INTERNALS §40.5 names it B1103)', owner: 'COMPILER crates/brain-check',
    files: { 'views/fetchy.brain': () => `<meta seo title="Direct fetch probe" description="Probe file: app code that calls fetch() directly instead of api(), which must be flagged.">\n<script>\n  let v = sig('')\n  const go = async () => { v = await (await fetch('/f/x')).text() }\n</script>\n<main>\n  <h1>Fetch</h1>\n  <button type="button" on:click={go}>go</button>\n  <p>{v}</p>\n</main>\n` },
    match: (d) => d.file === 'views/fetchy.brain' && /fetch/.test(d.msg) },
  { id: 'C3', title: 'B1106: load() with independent awaits (waterfall)', owner: 'COMPILER crates/brain-check',
    files: { 'views/waterfall.brain': () => `<meta seo title="Waterfall probe" description="Probe file: a load function with two independent awaits, which must suggest Promise.all.">\n<script>\n  const one = async () => 1\n  const two = async () => 2\n  export async function load(ctx) {\n    const a = await one()\n    const b = await two()\n    return { a, b }\n  }\n</script>\n<main>\n  <h1>Waterfall</h1>\n  <p>{props.a} {props.b}</p>\n</main>\n` },
    match: (d) => d.code === 'B1106' },
]

// ---------- in-page helpers ----------
// __net: dev request feed · __noise: every visible error surface that appears (toast with text, <ErrorState>, a
// boundary fallback, a {:catch} branch printing "error …") · __unhandled: unhandled promise rejections
const INIT = `window.__net = []; addEventListener('brain:net', (e) => window.__net.push(e.detail));
  window.__noise = []; window.__unhandled = [];
  addEventListener('unhandledrejection', (e) => window.__unhandled.push(String((e.reason && (e.reason.kind || e.reason.message)) || e.reason)));
  const scan = () => {
    const add = (s) => { if (!window.__noise.includes(s)) window.__noise.push(s) };
    for (const t of document.querySelectorAll('.brain-toast')) if (!t.hidden && t.textContent.trim()) add('toast: ' + t.textContent.trim());
    for (const e of document.querySelectorAll('.error-state, [role=alert]')) add('alert: ' + e.textContent.trim().slice(0, 80));
    for (const e of document.querySelectorAll('[data-r]')) if (/^error /.test(e.textContent.trim())) add('catch: ' + e.dataset.r + ' ' + e.textContent.trim());
  };
  document.addEventListener('DOMContentLoaded', () => { scan(); new MutationObserver(scan).observe(document.documentElement, { subtree: true, childList: true, characterData: true, attributes: true, attributeFilter: ['hidden'] }) });`

function ctxFor(page, base, mode, srv, feature, devLog, logs) {
  const devMark = devLog().length
  const ev = (x) => page.evaluate(x)
  const out = (id) => ev(`(document.querySelector('[data-r="${id}"]')?.textContent ?? '').trim()`)
  const until = async (expr, ms = 5000) => { for (let t = 0; t <= ms; t += 50) { if (await ev(expr).catch(() => false)) return true; await sleep(50) } return false }
  // click until the handler visibly started (the page may still be hydrating); the action is idempotent until then
  const click = async (name, outId) => {
    for (let i = 0; i < 20; i++) {
      await ev(`document.querySelector('[data-a="${name}"]')?.click()`)
      if (!outId || await until(`(document.querySelector('[data-r="${outId}"]')?.textContent ?? '').trim() !== '' || !document.querySelector('[data-a="${name}"]')`, 250)) return
    }
    throw new Error(`button ${name} never started (hydration?)`)
  }
  // click, then wait for a final JSON line (or a literal value) in <output data-r=name>
  const act = async (name, ms = 12000, literal = null) => {
    await click(name, name)
    if (literal) { await until(`document.querySelector('[data-r="${name}"]').textContent.trim() === ${JSON.stringify(literal)}`, 2000); return literal }
    if (!(await until(`/^\\{/.test(document.querySelector('[data-r="${name}"]').textContent.trim())`, ms))) throw new Error(`${name}: no result after ${ms} ms (output "${await out(name)}")`)
    return JSON.parse(await out(name))
  }
  return {
    mode, base, feature, ev, out, until, click, act,
    log: () => fetch(srv + '/__ctl/log').then((r) => r.json()),
    offline: async (on) => { await page.send('Network.emulateNetworkConditions', { offline: on, latency: 0, downloadThroughput: -1, uploadThroughput: -1 }); await sleep(150) },
    errors: () => logs.filter((l) => /^\[(error|exception)\]/.test(l)),
    devLog,
    // #228: an abort must stay silent everywhere (no toast, no report, no <ErrorState>, no {:catch})
    quiet: async () => {
      const page = await ev('({ noise: window.__noise || [], unhandled: window.__unhandled || [] })').catch(() => ({ noise: [], unhandled: [] }))
      const dev = devLog().slice(devMark).split('\n').filter((l) => /error|abort/i.test(l) && !/\[net\]/.test(l))
      return [...page.noise, ...page.unhandled.map((u) => 'unhandled rejection: ' + u), ...logs.filter((l) => /^\[(error|exception)\]/.test(l)), ...dev.map((l) => 'brain run: ' + l.trim())]
    },
  }
}

async function waitFor(page, expr, ms = 10000) {
  for (let t = 0; t < ms; t += 50) { if (await page.evaluate(expr).catch(() => false)) return true; await sleep(50) }
  return false
}

// ---------- working copy: the app minus the files that do not compile ----------
function prepare() {
  rmSync(join(WORK, 'app'), { recursive: true, force: true })
  mkdirSync(WORK, { recursive: true })
  const diags = brainCheck(SRC).filter(isError)
  const bad = new Map()
  for (const d of diags) if (!bad.has(d.file)) bad.set(d.file, `${d.code} ${d.file}:${d.line} ${d.msg}`)
  cpSync(SRC, join(WORK, 'app'), { recursive: true, filter: (p) => { const r = relative(SRC, p); return !bad.has(r) && !/^(dist|\.brain)(\/|$)/.test(r) } })
  const left = brainCheck(join(WORK, 'app')).filter(isError)
  if (left.length) fatal('the working copy still has errors after leaving out the failing files:\n' + JSON.stringify(left, null, 1))
  return bad
}

function runChecks() {
  const dir = join(WORK, 'checks')
  rmSync(dir, { recursive: true, force: true })
  cpSync(join(WORK, 'app'), dir, { recursive: true, filter: (p) => !/\/(dist|\.brain)(\/|$)/.test(p) })
  for (const c of CHECKS) for (const [f, make] of Object.entries(c.files)) {
    const p = join(dir, f)
    writeFileSync(p, make(existsSync(p) ? readFileSync(p, 'utf8') : ''))
  }
  const diags = brainCheck(dir)
  return CHECKS.map((c) => {
    const hit = diags.filter(c.match)
    const status = hit.length ? 'PASS' : 'BLOCKED'
    const evidence = hit.length ? hit.map((d) => `${d.code} ${d.file}:${d.line} ${d.msg}`).join(' · ')
      : `brain check reports nothing for the probe (${diags.filter((d) => Object.keys(c.files).includes(d.file)).map((d) => d.code).join(',') || 'no diagnostic'})`
    return { id: c.id, title: c.title, owner: c.owner, modes: { static: { status, evidence } } }
  })
}

// ---------- one mode ----------
async function runMode(mode, chromePort, bad) {
  const app = join(WORK, 'app')
  let fault, devProc = null, devLog = '', buildMs = null
  if (mode === 'dev') {
    devProc = spawn(BRAIN, ['run', '--port', String(DEV_PORT)], { cwd: app, stdio: ['ignore', 'pipe', 'pipe'] })
    children.add(devProc)
    devProc.stdout.on('data', (d) => (devLog += d)); devProc.stderr.on('data', (d) => (devLog += d))
    let up = false
    for (let i = 0; i < 150 && !up; i++) { try { up = (await fetch(`http://127.0.0.1:${DEV_PORT}/kinds`)).ok } catch { await sleep(100) } }
    if (!up) fatal(`brain run did not answer on :${DEV_PORT}:\n${devLog}`)
    fault = createFaultServer({ upstream: `http://127.0.0.1:${DEV_PORT}` })
  } else {
    if (!NO_BUILD) {
      const b0 = Date.now()
      try { execFileSync(BRAIN, ['build'], { cwd: app, stdio: 'pipe' }) } catch (err) { fatal('`brain build` failed:\n' + (err.stdout || '') + (err.stderr || '')) }
      buildMs = Date.now() - b0
    }
    fault = createFaultServer({ staticDir: join(app, 'dist') })
  }
  const srv = await fault.listen(0)
  const url = (p) => srv + p + (mode === 'prod' && p !== '/' ? '/' : '')
  // wiring that brain check does not reveal: is <meta http> compiled, or emitted verbatim into the page?
  const html = await (await fetch(url('/policy'))).text()
  const verbatim = /<meta http[\s>]/.test(html)
  const feature = { metaHttp: !verbatim }
  const featureWhy = { 'meta-http': verbatim ? '`<meta http base timeout retry login>` passes `brain check` but is emitted verbatim into the page HTML (`<meta http base="/api" …>` in /policy), so no http() policy is applied (COMPILER #170 / TOOLCHAIN #171)' : null }

  const page = await openPage(chromePort, 'about:blank')
  let logs = []
  page.on((m) => {
    if (m.method === 'Runtime.consoleAPICalled' && (m.params.type === 'error' || m.params.type === 'warning')) logs.push(`[${m.params.type === 'error' ? 'error' : 'warn'}] ` + m.params.args.map((x) => x.value ?? x.description ?? '').join(' '))
    if (m.method === 'Runtime.exceptionThrown') logs.push('[exception] ' + (m.params.exceptionDetails.exception?.description || m.params.exceptionDetails.text))
  })
  await page.send('Network.enable')
  await page.send('Page.addScriptToEvaluateOnNewDocument', { source: INIT })
  const results = []
  for (const s of SCENARIOS) {
    if (ONLY && !ONLY.includes(s.id)) continue
    const blockedFile = s.needs?.find((f) => bad.has(f))
    const blockedFeature = s.features?.find((f) => featureWhy[f])
    if (blockedFile) { results.push({ id: s.id, status: 'BLOCKED', evidence: bad.get(blockedFile) }); continue }
    if (blockedFeature) { results.push({ id: s.id, status: 'BLOCKED', evidence: featureWhy[blockedFeature] }); continue }
    const s0 = Date.now()
    logs = []
    const c = ctxFor(page, srv, mode, srv, feature, () => devLog, logs)
    // each scenario: fresh server state and a fresh page load
    await fetch(srv + '/__ctl/reset', { method: 'POST' })
    if (s.plan) await fetch(srv + '/__ctl/plan', { method: 'POST', body: JSON.stringify(s.plan) })
    const a = A()
    let brief = ''
    try {
      await page.send('Page.navigate', { url: url(s.view) })
      if (!(await waitFor(page, `document.readyState === 'complete' && !!document.querySelector('main h1')`))) throw new Error(`${s.view} did not render`)
      await sleep(400)
      brief = await Promise.race([s.run(c, a), sleep(45000).then(() => { throw new Error('scenario timed out after 45 s') })])
    } catch (err) { a.fails.push('error: ' + err.message) }
    try { await c.offline(false) } catch {}
    const L = await c.log().catch(() => null)
    results.push({ id: s.id, status: a.fails.length ? 'FAIL' : 'PASS', evidence: a.fails.length ? a.fails.join('; ') + (brief ? ' — observed: ' + brief : '') : brief, ms: Date.now() - s0, serverLog: L, console: [...logs] })
    process.stdout.write(`  ${mode} ${s.id} ${a.fails.length ? 'FAIL' : 'PASS'} ${((Date.now() - s0) / 1000).toFixed(1)} s\n`)
  }
  page.close()
  await fault.close()
  if (devProc) { devProc.kill('SIGTERM'); children.delete(devProc) }
  return { results, buildMs, feature, featureWhy, devLog }
}

// ---------- main ----------
mkdirSync(join(OUT, 'raw'), { recursive: true })
const bad = prepare()
const checks = ONLY ? [] : runChecks()
const chrome = await launch()
children.add({ kill: () => chrome.close() })
const modes = {}
for (const mode of MODES) {
  if (!['dev', 'prod'].includes(mode)) fatal(`unknown mode ${mode} (dev, prod)`)
  const m0 = Date.now()
  modes[mode] = await runMode(mode, chrome.port, bad)
  modes[mode].ms = Date.now() - m0
  writeFileSync(join(OUT, 'raw', `${mode}.json`), JSON.stringify({ mode, ...modes[mode] }, null, 1))
}
chrome.close()

// ---------- report ----------
const rows = [
  ...SCENARIOS.filter((s) => !ONLY || ONLY.includes(s.id)).map((s) => ({ id: s.id, title: s.title, owner: s.owner,
    modes: Object.fromEntries(MODES.map((m) => { const r = modes[m].results.find((x) => x.id === s.id); return [m, { status: r.status, evidence: r.evidence }] })) })),
  ...checks,
]
const statuses = rows.flatMap((r) => Object.values(r.modes).map((x) => x.status))
const count = (st) => statuses.filter((x) => x === st).length
const anyFail = count('FAIL') > 0
const totalMs = Date.now() - t0
const summary = {
  bench: 'http', date: new Date().toISOString(), brainSha256: brainSha, modes: MODES, verdict: anyFail ? 'FAIL' : 'PASS',
  counts: { PASS: count('PASS'), FAIL: count('FAIL'), BLOCKED: count('BLOCKED') },
  leftOut: Object.fromEntries(bad), features: Object.fromEntries(MODES.map((m) => [m, modes[m].feature])),
  runtimeMs: { total: totalMs, ...Object.fromEntries(MODES.map((m) => [m, modes[m].ms])), build: modes.prod?.buildMs ?? null },
  rows,
}
writeFileSync(join(OUT, 'summary.json'), JSON.stringify(summary, null, 1) + '\n')
const esc = (s) => String(s).replace(/\|/g, '\\|').replace(/\n/g, ' ')
const cell = (r, m) => { const x = r.modes[m] || r.modes.static; return x ? (x.status === 'PASS' ? 'pass' : `**${x.status}**`) : '' }
const ev = (r) => { const e = Object.entries(r.modes); const bad = e.filter(([, x]) => x.status !== 'PASS'); return (bad.length ? bad : e.slice(0, 1)).map(([m, x]) => (e.length > 1 ? `${m}: ` : '') + x.evidence).join(' · ') }
const md = [
  '# HTTP client bench — results',
  '',
  `Binary sha256 \`${brainSha}\` · ${summary.date} · total ${(totalMs / 1000).toFixed(1)} s · PASS ${summary.counts.PASS} · FAIL ${summary.counts.FAIL} · BLOCKED ${summary.counts.BLOCKED} · verdict **${summary.verdict}**`,
  '',
  `| # | Scenario | ${MODES.join(' | ')} | Owner | Evidence |`,
  `|---|---|${MODES.map(() => '---').join('|')}|---|---|`,
  ...rows.map((r) => `| ${r.id} | ${esc(r.title)} | ${MODES.map((m) => cell(r, m)).join(' | ')} | ${esc(r.owner)} | ${esc(ev(r))} |`),
  '',
  Object.keys(summary.leftOut).length ? 'Left out of the working copy (do not compile): ' + Object.entries(summary.leftOut).map(([f, w]) => `\`${f}\` (${esc(w)})`).join(' · ') : 'Every source file compiles.',
  '',
  'Raw per-mode data (server request logs, console lines): `results/raw/<mode>.json` (not committed).',
  '',
].join('\n')
writeFileSync(join(OUT, 'summary.md'), md)
const pad = (s, n) => (String(s) + ' '.repeat(n)).slice(0, n)
console.log(`http bench · brain ${brainSha} · ${(totalMs / 1000).toFixed(1)} s`)
for (const r of rows) console.log(`${pad(r.id, 4)}${pad(r.title, 60)}${MODES.map((m) => pad((r.modes[m] || r.modes.static).status, 8)).join('')}${ev(r).slice(0, 160)}`)
console.log(`verdict ${summary.verdict} (PASS ${summary.counts.PASS} · FAIL ${summary.counts.FAIL} · BLOCKED ${summary.counts.BLOCKED}) → bench/http/results/summary.md`)
process.exit(anyFail ? 1 : 0)
