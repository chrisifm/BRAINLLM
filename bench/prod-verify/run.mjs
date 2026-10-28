// Production verification of content collections, <meta cache> and _loading.brain under `brain start` (QA #233, #253).
// Cases:
//   233a static blog: `brain add collection blog` + 2 extra entries → prerendered dist/blog/index.html, sitemap, llms,
//        .md twins; `brain start` serves /blog (titles), /blog/<slug> (body), /blog/tag/<tag>; same with `node dist/server.mjs`
//   233b dynamic blog: the index load() reads ctx.query.tag and brain:content → SSR at request time (no dist/blog/index.html),
//        ?tag= filters, no "cannot find module" in any log; also with `node dist/server.mjs`, and with the app's .brain/
//        moved away (dist must be self-contained)
//   253a <meta cache="2">: two requests within 2 s share one render; between 2 and 4 s the stale copy is served and a
//        refresh renders in the background; after 4 s a request renders again; a session cookie bypasses the cache
//   253b _loading.brain with a 1500 ms load(): SSR (first chunk time, loading markup in it or not) and client navigation in
//        headless Chrome (loading markup appears before the content; a fast route never shows it)
// Usage: node bench/prod-verify/run.mjs   (BRAIN=<binary> to test a given binary; default: cargo build --release into
//        $HOME/tmp/brainllm-target-qa). Work dir: BRAIN_PV_WORK (default ~/tmp/brainllm-prodverify).
// Output: results/summary.md + results/summary.json. Exits 1 on any failed case. Every process started is stopped by pid.
import { spawn, spawnSync, execFileSync } from 'node:child_process'
import { readFileSync, writeFileSync, rmSync, mkdirSync, existsSync, copyFileSync, chmodSync, renameSync, cpSync } from 'node:fs'
import { join } from 'node:path'
import { homedir } from 'node:os'
import http from 'node:http'
import net from 'node:net'
import { launch, openPage } from '../lib/cdp.mjs'

const ROOT = new URL('../..', import.meta.url).pathname
const HERE = new URL('.', import.meta.url).pathname
const OUT = join(HERE, 'results')
const HOME = homedir()
const tilde = (s) => String(s).split(HOME).join('~')
const sleep = (ms) => new Promise((r) => setTimeout(r, ms))
const now = () => performance.now()
const WORK = process.env.BRAIN_PV_WORK ?? join(HOME, 'tmp/brainllm-prodverify')
mkdirSync(WORK, { recursive: true }); mkdirSync(OUT, { recursive: true })

// ---------- binary ----------
let src = process.env.BRAIN
if (!src) {
  const target = join(HOME, 'tmp/brainllm-target-qa')
  execFileSync('cargo', ['build', '--release', '-p', 'brain-cli'], { cwd: ROOT, stdio: 'inherit', env: { ...process.env, CARGO_TARGET_DIR: target } })
  src = join(target, 'release/brain')
}
const BRAIN = join(WORK, 'brain-pv')
rmSync(BRAIN, { force: true }); copyFileSync(src, BRAIN); chmodSync(BRAIN, 0o755) // never copy over a previous copy (macOS)
const SHA = execFileSync('shasum', ['-a', '256', BRAIN]).toString().slice(0, 12)
const WARM = (() => { const t = now(); const r = spawnSync(BRAIN, ['help']); if (r.status !== 0) { console.error('copied binary does not run'); process.exit(1) } return Math.round(now() - t) })()
const NODE_V = execFileSync('node', ['-v']).toString().trim()
console.log(`brain ${SHA} · node ${NODE_V} · first exec ${WARM} ms`)

const brain = (cwd, args) => { const r = spawnSync(BRAIN, args, { cwd, encoding: 'utf8', timeout: 300_000 }); return { code: r.status, out: r.stdout ?? '', err: r.stderr ?? '' } }
const must = (cwd, args) => { const r = brain(cwd, args); if (r.code !== 0) { console.error(`setup failed: brain ${args.join(' ')} → ${r.code}\n${tilde(r.err || r.out)}`); process.exit(1) } return r }
const freePort = () => new Promise((ok) => { const s = net.createServer(); s.listen(0, '127.0.0.1', () => { const p = s.address().port; s.close(() => ok(p)) }) })
const alive = (pid) => { try { process.kill(pid, 0); return true } catch { return false } }

// one GET on a fresh connection; records the time of the first body chunk and the whole body
function get(port, path, headers = {}) {
  const t = now()
  return new Promise((ok) => {
    const r = http.get({ host: '127.0.0.1', port, path, headers, agent: false, timeout: 20_000 }, (res) => {
      const parts = [], chunks = []; let first = null, firstChunk = ''
      res.on('data', (c) => { if (first === null) { first = now() - t; firstChunk = c.toString('utf8') } parts.push(c); chunks.push({ ms: now() - t, text: c.toString('utf8') }) })
      res.on('end', () => ok({ status: res.statusCode, headers: res.headers, body: Buffer.concat(parts).toString('utf8'), firstMs: first, firstChunk, chunks, ms: now() - t }))
    })
    r.on('timeout', () => r.destroy(new Error('timeout')))
    r.on('error', (e) => ok({ status: 0, error: e.code || e.message, body: '', ms: now() - t }))
  })
}
async function waitUp(port, ms = 30_000) { const t = now(); while (now() - t < ms) { const r = await get(port, '/_health'); if (r.status === 200) return true; await sleep(50) } return false }

// processes this run started: stopped at the end by pid only
const procs = []
function startBrain(app, port) {
  const p = spawn(BRAIN, ['start', '--port', String(port)], { cwd: app, stdio: ['ignore', 'pipe', 'pipe'] })
  const s = { kind: 'brain', app, p, pid: p.pid, out: '', exit: null }
  p.stdout.on('data', (c) => (s.out += c)); p.stderr.on('data', (c) => (s.out += c)); p.on('exit', (c, sig) => (s.exit = c ?? sig))
  procs.push(s); return s
}
function startNode(dist, port) {
  const p = spawn('node', [join(dist, 'server.mjs')], { cwd: dist, env: { ...process.env, PORT: String(port), HOST: '127.0.0.1' }, stdio: ['ignore', 'pipe', 'pipe'] })
  const s = { kind: 'node', app: dist, p, pid: p.pid, out: '', exit: null }
  p.stdout.on('data', (c) => (s.out += c)); p.stderr.on('data', (c) => (s.out += c)); p.on('exit', (c, sig) => (s.exit = c ?? sig))
  procs.push(s); return s
}
async function stop(s) {
  if (s.done) return; s.done = true
  let kids = []
  if (s.kind === 'brain' && s.exit === null) {
    try { kids = (JSON.parse(brain(s.app, ['status', '--json']).out.trim()).instances ?? []).map((i) => i.pid) } catch {}
    brain(s.app, ['stop'])
  } else if (s.exit === null) s.p.kill('SIGTERM')
  const t = now(); while (s.exit === null && now() - t < 15_000) await sleep(50)
  if (s.exit === null) try { process.kill(s.pid, 'SIGKILL') } catch {}
  for (const k of kids) if (alive(k)) try { process.kill(k, 'SIGKILL') } catch {}
}
const cleanup = async () => { for (const s of procs) await stop(s) }
process.on('SIGINT', async () => { await cleanup(); process.exit(130) })
const appLog = (app) => (existsSync(join(app, 'logs/app.log')) ? readFileSync(join(app, 'logs/app.log'), 'utf8') : '')
const MODERR = /cannot find (module|package)|ERR_MODULE_NOT_FOUND|brain:content/i

const results = []
const record = (id, name, command, expected, actual, pass) => { results.push({ id, name, command: tilde(command), expected, actual: tilde(actual), pass }); console.log(`${pass ? 'PASS' : 'FAIL'} ${id} ${name} — ${tilde(actual)}`) }
const titles = (b) => ['First entry of Blog', 'Second post about caching', 'Third post about routing'].filter((t) => b.includes(t))

// ---------- app setup ----------
const ENTRY = (title, date, tags, marker) => `---
title: ${title}
description: An entry written by the production verification bench to check listings, entry pages and tag pages under brain start.
date: ${date}
tags: [${tags}]
---
Entry body with the marker ${marker}.
`
function newBlogApp(name) {
  const app = join(WORK, name)
  rmSync(app, { recursive: true, force: true })
  must(WORK, ['new', name]); must(app, ['add', 'collection', 'blog'])
  writeFileSync(join(app, 'content/blog/second-post.md'), ENTRY('Second post about caching', '2026-10-12', 'caching, news', 'PV-SECOND'))
  writeFileSync(join(app, 'content/blog/third-post.md'), ENTRY('Third post about routing', '2026-10-11', 'news', 'PV-THIRD'))
  return app
}

try {
  // ===== #233a: static blog =====
  {
    const app = newBlogApp('pv-static')
    const b = brain(app, ['build'])
    const d = join(app, 'dist')
    const idx = existsSync(join(d, 'blog/index.html')) ? readFileSync(join(d, 'blog/index.html'), 'utf8') : ''
    const sm = existsSync(join(d, 'sitemap.xml')) ? readFileSync(join(d, 'sitemap.xml'), 'utf8') : ''
    const llms = existsSync(join(d, 'llms.txt')) ? readFileSync(join(d, 'llms.txt'), 'utf8') : ''
    const pre = { build: b.code, index: !!idx, indexTitles: titles(idx).length, sitemapEntries: ['/blog/second-post', '/blog/third-post', '/blog/hello-world'].filter((u) => sm.includes(u)).length, llms: llms.includes('Second post about caching'), md: existsSync(join(d, 'blog.md')) && existsSync(join(d, 'blog/second-post.md')) }
    record('233a-build', 'static blog is prerendered', 'brain build', 'exit 0; dist/blog/index.html with 3 titles; sitemap lists 3 entries; llms.txt lists them; blog.md and blog/second-post.md exist',
      JSON.stringify(pre), pre.build === 0 && pre.index && pre.indexTitles === 3 && pre.sitemapEntries === 3 && pre.llms && pre.md)
    for (const mode of ['brain start', 'node dist/server.mjs']) {
      const port = await freePort()
      const s = mode === 'brain start' ? startBrain(app, port) : startNode(d, port)
      const up = await waitUp(port)
      const list = await get(port, '/blog'), one = await get(port, '/blog/second-post'), tag = await get(port, '/blog/tag/news'), rss = await get(port, '/blog/rss.xml')
      await stop(s)
      const errs = (s.out + (mode === 'brain start' ? appLog(app) : '')).split('\n').filter((l) => MODERR.test(l))
      const act = { up, list: list.status, listTitles: titles(list.body).length, entry: one.status, entryBody: one.body.includes('PV-SECOND'), tag: tag.status, tagTitles: titles(tag.body).length, rss: rss.status, moduleErrors: errs.length }
      record(`233a-${mode === 'brain start' ? 'start' : 'node'}`, `static blog served by ${mode}`, `${mode} → GET /blog, /blog/second-post, /blog/tag/news, /blog/rss.xml`,
        '200 everywhere; /blog has the 3 titles; the entry has its body; the news tag lists 2; no module errors', JSON.stringify(act),
        up && list.status === 200 && act.listTitles === 3 && one.status === 200 && act.entryBody && tag.status === 200 && act.tagTitles === 2 && rss.status === 200 && !errs.length)
    }
  }

  // ===== #233b: dynamic blog index (ctx.query + brain:content) =====
  {
    const app = newBlogApp('pv-dynamic')
    writeFileSync(join(app, 'views/blog/index.brain'), `<meta seo title="Blog" description="Every entry of the blog, newest first, filtered by tag with ?tag=, with a short summary, its date and its tags.">

<script>
  import { page, tagged } from 'brain:content'
  export async function load(ctx) {
    const tag = ctx.query.tag
    if (tag) return { tag, entries: tagged('blog', tag), next: null }
    return { tag: '', ...page('blog', 1) }
  }
</script>

<main class="page">
  <h1>Blog</h1>
  {#if props.tag}<p class="filter">Filtered by {props.tag}</p>{/if}
  <ul class="entries">
    {#each props.entries as e (e.slug)}
      <li><h2><a href={e.url}>{e.title}</a></h2><p>{e.description}</p></li>
    {/each}
  </ul>
</main>

<style>
  .entries { display: grid; gap: var(--space-5); padding: 0; list-style: none; }
  h2 { margin: 0; font-size: 1.25rem; }
</style>
`)
    const b = brain(app, ['build'])
    const d = join(app, 'dist')
    const prerendered = existsSync(join(d, 'blog/index.html'))
    const mf = existsSync(join(d, '_server/manifest.json')) ? JSON.parse(readFileSync(join(d, '_server/manifest.json'), 'utf8')) : { routes: [] }
    const route = mf.routes.find((r) => r.path === '/blog')
    record('233b-build', 'dynamic blog index is SSR, not prerendered', 'brain build', 'exit 0; no dist/blog/index.html; /blog in dist/_server/manifest.json with a server module',
      JSON.stringify({ build: b.code, prerendered, route: !!route, server: route?.server ?? null, buildErr: b.code ? (b.err || b.out).slice(-400) : '' }), b.code === 0 && !prerendered && !!route?.server)
    const probe = async (port) => {
      const all = await get(port, '/blog'), caching = await get(port, '/blog?tag=caching'), news = await get(port, '/blog?tag=news'), none = await get(port, '/blog?tag=nope'), entry = await get(port, '/blog/third-post')
      return { all: [all.status, titles(all.body).length], caching: [caching.status, titles(caching.body)], news: [news.status, titles(news.body).length], none: [none.status, titles(none.body).length], entry: [entry.status, entry.body.includes('PV-THIRD')] }
    }
    const ok = (a) => a.all[0] === 200 && a.all[1] === 3 && a.caching[0] === 200 && a.caching[1].length === 1 && a.caching[1][0] === 'Second post about caching' && a.news[0] === 200 && a.news[1] === 2 && a.none[0] === 200 && a.none[1] === 0 && a.entry[0] === 200 && a.entry[1]
    const EXP = '/blog 200 with 3 titles; ?tag=caching only "Second post about caching"; ?tag=news 2; ?tag=nope 0; /blog/third-post 200 with its body; no module errors in stdout/stderr or logs/app.log'
    for (const mode of ['brain start', 'node dist/server.mjs', 'node dist/server.mjs, .brain/ moved away']) {
      const moved = mode.includes('moved')
      if (moved) renameSync(join(app, '.brain'), join(app, '.brain-moved'))
      const port = await freePort()
      const s = mode === 'brain start' ? startBrain(app, port) : startNode(d, port)
      const up = await waitUp(port)
      const a = up ? await probe(port) : null
      await stop(s)
      if (moved) renameSync(join(app, '.brain-moved'), join(app, '.brain'))
      const errs = (s.out + (mode === 'brain start' ? appLog(app) : '')).split('\n').filter((l) => MODERR.test(l))
      record(`233b-${mode === 'brain start' ? 'start' : moved ? 'portable' : 'node'}`, `dynamic blog served by ${mode}`, `${mode} → GET /blog, /blog?tag=caching|news|nope, /blog/third-post`, EXP,
        JSON.stringify({ up, ...a, moduleErrors: errs.slice(0, 2).map((l) => l.slice(0, 300)) }), up && !!a && ok(a) && !errs.length)
    }
  }

  // ===== #253: <meta cache> and _loading.brain =====
  {
    const app = join(WORK, 'pv-cache')
    rmSync(app, { recursive: true, force: true })
    must(WORK, ['new', 'pv-cache'])
    writeFileSync(join(app, 'views/clock.brain'), `<meta seo title="Clock" description="A view cached for two seconds: each render stamps the time so the bench can tell a cached copy from a fresh render.">
<meta cache="2">

<script>
  export async function load(ctx) {
    return { stamp: Date.now() + '-' + Math.random().toString(36).slice(2, 8), q: ctx.query.q || '' }
  }
</script>

<main class="page">
  <h1>Clock</h1>
  <p class="stamp">STAMP[{props.stamp}]</p>
</main>
`)
    writeFileSync(join(app, 'views/slow.brain'), `<meta seo title="Slow page" description="A view whose load waits one and a half seconds, so the loading view of the folder has time to show.">

<script>
  export async function load(ctx) {
    await new Promise((r) => setTimeout(r, 1500))
    return { q: ctx.query.q || '' }
  }
</script>

<main class="page">
  <h1>Slow page</h1>
  <p class="ready">SLOW-READY</p>
</main>
`)
    const b = brain(app, ['build'])
    const mf = existsSync(join(app, 'dist/_server/manifest.json')) ? JSON.parse(readFileSync(join(app, 'dist/_server/manifest.json'), 'utf8')) : { routes: [] }
    const clock = mf.routes.find((r) => r.path === '/clock'), slow = mf.routes.find((r) => r.path === '/slow')
    record('253-build', 'build emits route.cache and route.loading', 'brain build', 'exit 0; manifest /clock cache 2; /slow has loading (the _loading view)',
      JSON.stringify({ build: b.code, clockCache: clock?.cache ?? null, slowLoading: slow?.loading ?? null, slowStream: slow?.stream ?? null }), b.code === 0 && Number(clock?.cache) === 2 && !!slow?.loading)

    const port = await freePort()
    const s = startBrain(app, port)
    const up = await waitUp(port)
    // --- 253a cache timeline ---
    const stamp = (r) => (r.body.match(/STAMP\[([^\]<]+)/) || [])[1] ?? null
    const t0 = now(), at = () => Math.round(now() - t0)
    const tl = []
    const hit = async (label, headers = {}) => { const r = await get(port, '/clock', headers); tl.push({ label, atMs: at(), ms: Math.round(r.ms), status: r.status, stamp: stamp(r), cc: r.headers?.['cache-control'] ?? null }); return tl[tl.length - 1] }
    const until = async (ms) => { while (now() - t0 < ms) await sleep(10) }
    if (up) {
      await hit('cold')
      await until(500); await hit('0.5 s')
      await until(1500); await hit('1.5 s')
      await until(2300); await hit('2.3 s (expired, < 2N)')
      await until(2800); await hit('2.8 s (after background refresh)')
      await until(7500); await hit('7.5 s (> 2N since last render)')
      await hit('with session cookie', { cookie: '__Host-session=x' })
      await hit('with session cookie again', { cookie: '__Host-session=x' })
    }
    const S = tl.map((x) => x.stamp)
    record('253a-hit', 'two requests within N s share one render', 'brain start → GET /clock at 0, 0.5, 1.5 s', 'same stamp; cache-control public, s-maxage=2',
      JSON.stringify(tl.slice(0, 3)), up && S[0] && S[0] === S[1] && S[1] === S[2] && /s-maxage=2/.test(tl[0]?.cc ?? ''))
    record('253a-swr', 'first request after N s gets the stale copy and triggers a refresh', 'GET /clock at 2.3 s then 2.8 s',
      'at 2.3 s the old stamp (stale-while-revalidate, per server.mjs and INTERNALS §35 wording "served while a fresh one renders"); at 2.8 s a new stamp',
      JSON.stringify(tl.slice(3, 5)), up && S[3] === S[0] && S[4] && S[4] !== S[3])
    record('253a-expire', 'after 2N s a request renders again', 'GET /clock at 7.5 s', 'a new stamp, different from the refreshed one', JSON.stringify(tl[5] ?? null), up && S[5] && S[5] !== S[4])
    record('253a-session', 'a session cookie bypasses the cache', 'GET /clock twice with cookie __Host-session', 'two fresh stamps, both different from the cached one',
      JSON.stringify(tl.slice(6)), up && S[6] && S[7] && S[6] !== S[7] && S[6] !== S[5])

    // --- 253b SSR of the slow route ---
    const ssr = up ? await get(port, '/slow') : null
    const LOADING = /aria-label="Loading"/
    const ssrAct = ssr ? { status: ssr.status, firstChunkMs: Math.round(ssr.firstMs ?? -1), totalMs: Math.round(ssr.ms), firstChunkHasShell: /<head/.test(ssr.firstChunk), firstChunkHasLoading: LOADING.test(ssr.firstChunk), firstChunkHasContent: ssr.firstChunk.includes('SLOW-READY'), finalHasContent: ssr.body.includes('SLOW-READY') } : null
    record('253b-ssr-stream', 'SSR of a slow load streams the shell first', 'GET /slow (load waits 1500 ms)', 'first chunk (head + layout) well before 1500 ms; the full page has the content after ≥ 1500 ms',
      JSON.stringify(ssrAct), !!ssrAct && ssrAct.status === 200 && ssrAct.firstChunkMs < 500 && ssrAct.firstChunkHasShell && ssrAct.totalMs >= 1500 && ssrAct.finalHasContent)
    // The server writes the head, then the loading view, as separate writes before load() ends (runtime/server/server.mjs
    // `streamed`), so a client can receive them as two TCP chunks: "before load() ends" is every chunk that arrived
    // before the one carrying the content, all well under the 1500 ms load.
    const ci = ssr && ssr.chunks ? ssr.chunks.findIndex((c) => c.text.includes('SLOW-READY')) : -1
    const early = ci > 0 ? ssr.chunks.slice(0, ci) : []
    const earlyText = early.map((c) => c.text).join('')
    const loadAct = ssr ? { chunks: ssr.chunks.map((c) => Math.round(c.ms)), earlyHasLoading: LOADING.test(earlyText), earlyLastMs: early.length ? Math.round(early.at(-1).ms) : null, contentMs: ci >= 0 ? Math.round(ssr.chunks[ci].ms) : null, firstChunkHasLoading: ssrAct.firstChunkHasLoading } : null
    record('253b-ssr-loading', '_loading.brain markup during SSR', 'GET /slow, inspect the streamed chunks that arrive before the content', 'SPEC line 16: "_loading.brain shows while load runs" → the loading markup is in the streamed region before the content, sent before load() ends',
      JSON.stringify(loadAct), !!loadAct && loadAct.earlyHasLoading && loadAct.earlyLastMs !== null && loadAct.earlyLastMs < 1000)

    // --- 253b client navigation in headless Chrome ---
    let nav = null, fast = null, direct = null, chromeErr = null
    if (up) {
      const ch = await launch()
      try {
        const run = async (to) => {
          const pg = await openPage(ch.port, `http://127.0.0.1:${port}/`)
          // wait for the page and the router
          for (let i = 0; i < 200; i++) { const r = await pg.evaluate(`document.readyState === 'complete' && typeof globalThis.__brainNavigate === 'function'`).catch(() => false); if (r) break; await sleep(50) }
          const hasNav = await pg.evaluate(`typeof globalThis.__brainNavigate === 'function'`)
          const r = await pg.evaluate(`new Promise((done) => {
            const t0 = performance.now(), ev = { loadingAt: null, contentAt: null, contentWhileLoading: false }
            const check = () => {
              const l = document.querySelector('[aria-label="Loading"]'), c = document.body.textContent.includes('SLOW-READY') || /^About /.test(document.querySelector('main h1')?.textContent || '')
              if (l && ev.loadingAt === null) ev.loadingAt = Math.round(performance.now() - t0)
              if (c && ev.contentAt === null) { ev.contentAt = Math.round(performance.now() - t0); ev.contentWhileLoading = !!l }
            }
            const mo = new MutationObserver(check); mo.observe(document.documentElement, { childList: true, subtree: true, characterData: true })
            const iv = setInterval(check, 5)
            const start = location.href
            globalThis.__brainNavigate(${JSON.stringify(to)})
            setTimeout(() => { clearInterval(iv); mo.disconnect(); check(); done({ ...ev, path: location.pathname, sameDocument: true, reloaded: location.href === start ? false : null }) }, 3500)
          })`)
          const sameDoc = await pg.evaluate(`performance.getEntriesByType('navigation').length === 1 && performance.getEntriesByType('navigation')[0].name.endsWith('/')`)
          pg.close()
          return { router: hasNav, ...r, sameDocument: sameDoc }
        }
        nav = await run('/slow')
        fast = await run('/about')
        // a direct load of the streamed route: once the page has loaded, only the content is left (the inline script
        // of the last chunk removed the loading markup and its marker comments)
        const pg = await openPage(ch.port, `http://127.0.0.1:${port}/slow`)
        for (let i = 0; i < 100; i++) { const r = await pg.evaluate(`document.readyState === 'complete'`).catch(() => false); if (r) break; await sleep(50) }
        await sleep(300)
        direct = await pg.evaluate(`(() => { const w = document.createTreeWalker(document.body, NodeFilter.SHOW_COMMENT); let m = 0; while (w.nextNode()) if (/brain:loading/.test(w.currentNode.data)) m++; return { content: document.body.textContent.includes('SLOW-READY'), loadingNodes: document.querySelectorAll('[aria-label="Loading"]').length, markers: m } })()`)
        pg.close()
      } catch (e) { chromeErr = String(e && e.message || e) } finally { ch.close() }
    }
    record('253b-client-loading', 'client navigation to the slow route shows _loading first', 'headless Chrome: open /, __brainNavigate("/slow"), observe the DOM',
      'loading markup appears ~150 ms after the navigation (router threshold) and before the content (~1500 ms); same document (no reload)',
      JSON.stringify(nav ?? chromeErr), !!nav && nav.router && nav.loadingAt !== null && nav.contentAt !== null && nav.loadingAt < nav.contentAt && nav.sameDocument)
    record('253b-ssr-final', 'a direct load of the streamed route ends with only the content', 'headless Chrome: open /slow, wait for load', 'content present; no [aria-label="Loading"] node and no brain:loading marker comment left',
      JSON.stringify(direct ?? chromeErr), !!direct && direct.content && direct.loadingNodes === 0 && direct.markers === 0)
    record('253b-client-fast', 'a fast navigation never shows _loading', 'headless Chrome: open /, __brainNavigate("/about")', 'content arrives, no loading markup at any time',
      JSON.stringify(fast ?? chromeErr), !!fast && fast.contentAt !== null && fast.loadingAt === null)
    await stop(s)
    const errs = (s.out + appLog(app)).split('\n').filter((l) => /"level":"error"|cannot find/i.test(l))
    record('253-logs', 'no server errors logged during #253', 'logs/app.log + supervisor output', 'no error lines', JSON.stringify(errs.slice(0, 3).map((l) => l.slice(0, 300))), !errs.length)
  }
} finally {
  await cleanup()
}

const left = procs.filter((s) => alive(s.pid))
const pass = results.filter((r) => r.pass).length
writeFileSync(join(OUT, 'summary.json'), JSON.stringify({ date: new Date().toISOString().slice(0, 10), brain: SHA, node: NODE_V, firstExecMs: WARM, leftovers: left.length, results }, null, 2) + '\n')
const esc = (s) => String(s).replace(/\|/g, '\\|')
writeFileSync(join(OUT, 'summary.md'), `# Production verification: collections, \`<meta cache>\`, \`_loading.brain\` (QA #233, #253)

Run ${new Date().toISOString().slice(0, 10)} · binary sha256 \`${SHA}\` · Node ${NODE_V} · macOS · ${pass}/${results.length} PASS · leftover processes: ${left.length}

| Case | Check | Command | Expected | Actual | Verdict |
|---|---|---|---|---|---|
${results.map((r) => `| ${r.id} | ${esc(r.name)} | \`${esc(r.command)}\` | ${esc(r.expected)} | \`${esc(r.actual)}\` | ${r.pass ? 'PASS' : 'FAIL'} |`).join('\n')}
`)
console.log(`${pass}/${results.length} pass · leftovers ${left.length}`)
process.exit(pass === results.length && !left.length ? 0 : 1)
