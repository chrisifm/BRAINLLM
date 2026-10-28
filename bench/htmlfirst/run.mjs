// QA #157 (INTERNALS §38 "HTML first"): every scaffold and example page with JavaScript disabled, then with it on.
//
//   node bench/htmlfirst/run.mjs                 # cargo build --release -p brain-cli, then every check
//   BRAIN=/path/to/brain node bench/htmlfirst/run.mjs
//
// Apps (in BRAIN_HTMLFIRST_WORK, default ~/tmp/brainllm-htmlfirst):
//   scaffold  `brain new scaf` + the probe files of bench/htmlfirst/probe (a form() with a `server:` submit, a
//             `<meta client>` component), served by `brain start --env development` (the demo login lives there)
//   examples  a copy of examples/ without invalid/, served by `brain start`
// The browser talks to a counting proxy in front of `brain start`, so POSTs are counted at the server side.
// Raw HTTP checks (defences) go straight to `brain start`. Exit 1 on any failed check.
// Processes: every supervisor is stopped with `brain stop`, then killed only by the pids this script spawned or read
// from `brain status --json`; Chrome is closed by its own handle. Nothing is left running.
import { homedir } from 'node:os'
import { spawn, spawnSync } from 'node:child_process'
import { readFileSync, writeFileSync, rmSync, mkdirSync, existsSync, copyFileSync, cpSync, readdirSync } from 'node:fs'
import { join } from 'node:path'
import { createHash } from 'node:crypto'
import http from 'node:http'
import net from 'node:net'
import { launch, openPage } from '../lib/cdp.mjs'

const ROOT = new URL('../..', import.meta.url).pathname
const HERE = new URL('.', import.meta.url).pathname
const WORK = process.env.BRAIN_HTMLFIRST_WORK || join(homedir(), 'tmp/brainllm-htmlfirst')
const RES = join(HERE, 'results')
const sleep = (ms) => new Promise((r) => setTimeout(r, ms))
const now = () => performance.now()
const tilde = (s) => String(s).split(homedir()).join('~')
mkdirSync(WORK, { recursive: true })
mkdirSync(RES, { recursive: true })

// ---------- binary ----------
let BRAIN = process.env.BRAIN
if (!BRAIN) {
  const b = spawnSync('cargo', ['build', '--release', '-p', 'brain-cli'], { cwd: ROOT, stdio: 'inherit' })
  if (b.status !== 0) { console.error('cargo build failed'); process.exit(1) }
  BRAIN = join(ROOT, 'target/release/brain')
}
mkdirSync(join(WORK, 'bin'), { recursive: true })
// macOS kills (SIGKILL, code signature) a binary copied over a previous copy: remove it first.
rmSync(join(WORK, 'bin/brain'), { force: true }); copyFileSync(BRAIN, join(WORK, 'bin/brain'))
BRAIN = join(WORK, 'bin/brain')
spawnSync('chmod', ['+x', BRAIN])
const SHA = createHash('sha256').update(readFileSync(BRAIN)).digest('hex').slice(0, 12)
console.log(`binary ${tilde(BRAIN)} sha256 ${SHA}`)

const brain = (cwd, args) => {
  const r = spawnSync(BRAIN, args, { cwd, encoding: 'utf8', timeout: 180_000 })
  return { code: r.status, out: r.stdout ?? '', err: r.stderr ?? '' }
}
const freePort = () => new Promise((ok) => { const s = net.createServer(); s.listen(0, '127.0.0.1', () => { const p = s.address().port; s.close(() => ok(p)) }) })
const alive = (pid) => { try { process.kill(pid, 0); return true } catch { return false } }

// ---------- results ----------
const rows = [] // { app, target, check, mode, pass, evidence, owner }
function rec(app, target, check, mode, pass, evidence, owner = '') {
  rows.push({ app, target, check, mode, pass: !!pass, evidence: tilde(evidence), owner: pass ? '' : owner })
  console.log(`${pass ? 'PASS' : 'FAIL'} [${app}] ${target} · ${check} · ${mode} — ${tilde(evidence)}${pass ? '' : ' (owner ' + owner + ')'}`)
}

// ---------- apps ----------
const started = []
function startApp(app, port, extra = []) {
  const proc = spawn(BRAIN, ['start', '--port', String(port), ...extra], { cwd: app, stdio: ['ignore', 'pipe', 'pipe'] })
  const s = { app, proc, pid: proc.pid, err: '', exit: null }
  proc.stderr.on('data', (c) => (s.err += c))
  proc.stdout.on('data', () => {})
  proc.on('exit', (code, sig) => { s.exit = code ?? sig })
  started.push(s)
  return s
}
async function stopApp(s) {
  if (s.done) return
  s.done = true
  let kids = []
  try { kids = (JSON.parse(brain(s.app, ['status', '--json']).out.trim()).instances ?? []).map((i) => i.pid) } catch {}
  if (s.exit === null) brain(s.app, ['stop'])
  const t = now()
  while (s.exit === null && now() - t < 20_000) await sleep(50)
  if (s.exit === null) { try { process.kill(s.pid, 'SIGKILL') } catch {} }
  await sleep(100)
  for (const k of kids) if (alive(k)) { try { process.kill(k, 'SIGKILL') } catch {} }
  return [s.pid, ...kids].filter(alive)
}
async function waitHealth(port, ms = 20_000) {
  const t = now()
  while (now() - t < ms) {
    const r = await raw(port, 'GET', '/_health')
    if (r.status === 200) return true
    await sleep(100)
  }
  return false
}
// one raw HTTP request → { status, headers, body }
function raw(port, method, path, headers = {}, body = null) {
  return new Promise((ok) => {
    const r = http.request({ host: '127.0.0.1', port, method, path, headers, agent: false, timeout: 10_000 }, (res) => {
      const parts = []
      res.on('data', (c) => parts.push(c))
      res.on('end', () => ok({ status: res.statusCode, headers: res.headers, body: Buffer.concat(parts).toString('utf8') }))
    })
    r.on('error', (e) => ok({ status: 0, error: e.message, body: '' }))
    r.on('timeout', () => r.destroy(new Error('timeout')))
    if (body != null) r.write(body)
    r.end()
  })
}
// counting proxy: the browser's origin; Host is forwarded unchanged so the same-origin checks see the proxy's host
function proxy(target) {
  const count = { posts: [], all: 0 }
  const srv = http.createServer((req, res) => {
    count.all++
    if (req.method === 'POST') count.posts.push(req.url)
    const up = http.request({ host: '127.0.0.1', port: target, method: req.method, path: req.url, headers: req.headers }, (r) => {
      res.writeHead(r.statusCode, r.headers)
      r.pipe(res)
    })
    up.on('error', () => { res.writeHead(502); res.end() })
    req.pipe(up)
  })
  return new Promise((ok) => srv.listen(0, '127.0.0.1', () => ok({ port: srv.address().port, count, close: () => new Promise((r) => { srv.closeAllConnections?.(); srv.close(r) }) })))
}

// scaffold: brain new + probe files
const SCAF = join(WORK, 'scaf')
rmSync(SCAF, { recursive: true, force: true })
const made = brain(WORK, ['new', 'scaf'])
if (made.code !== 0) { console.error('brain new failed', made.err); process.exit(1) }
for (const d of ['views', 'comp']) for (const f of readdirSync(join(HERE, 'probe', d))) copyFileSync(join(HERE, 'probe', d, f), join(SCAF, d, f))
const scafBuild = brain(SCAF, ['build'])
rec('scaffold', 'build', '`brain build` succeeds', 'build', scafBuild.code === 0, `exit ${scafBuild.code}; ${(scafBuild.out + scafBuild.err).match(/summary.*$/m)?.[0] ?? scafBuild.err.trim().split('\n').pop()}`, 'BUILD')
// test credentials: the scaffold's own [development] DEMO_PASSWORD (a test value from the project files; never printed)
const PASSWORD = (readFileSync(join(SCAF, '.brain_env'), 'utf8').match(/^DEMO_PASSWORD=(.+)$/m) || [])[1]?.trim()

// examples: copy without invalid/, dist/ and .brain/
const EX = join(WORK, 'ex')
rmSync(EX, { recursive: true, force: true })
cpSync(join(ROOT, 'examples'), EX, { recursive: true, filter: (src) => !/\/examples\/(invalid|dist|\.brain)(\/|$)/.test(src) })
let exBuild = brain(EX, ['build'])
rec('examples', 'build', '`brain build` succeeds as shipped', 'build', exBuild.code === 0, `exit ${exBuild.code}; ${(exBuild.out + exBuild.err).match(/^error:.*$/m)?.[0] ?? 'ok'}`, 'BUILD (default og image template uses tokens the app does not define)')
let exWorkaround = ''
if (exBuild.code !== 0 && /og image/.test(exBuild.err + exBuild.out)) {
  // workaround to test the pages anyway: turn the generated social image off in brain.json of the copy
  const bj = existsSync(join(EX, 'brain.json')) ? JSON.parse(readFileSync(join(EX, 'brain.json'), 'utf8')) : { name: 'examples' }
  bj.seo = { ...(bj.seo || {}), og: 'off' }
  writeFileSync(join(EX, 'brain.json'), JSON.stringify(bj, null, 2))
  exWorkaround = 'brain.json seo.og = "off" in the copy'
  exBuild = brain(EX, ['build'])
}

const apps = []

// ---------- browser ----------
async function page(port, js) {
  const p = await openPage(port, 'about:blank')
  const waiters = new Set()
  const st = { docs: [], loads: 0 }
  p.on((m) => {
    if (m.method === 'Page.loadEventFired') st.loads++
    if (m.method === 'Network.responseReceived' && m.params.type === 'Document') st.docs.push({ url: m.params.response.url, status: m.params.response.status })
    if (m.method === 'Network.requestWillBeSent' && m.params.redirectResponse && m.params.type === 'Document') st.docs.push({ url: m.params.redirectResponse.url, status: m.params.redirectResponse.status, location: m.params.request.url })
    // a dirty form() asks before leaving (beforeunload): accept, the run navigates on purpose
    if (m.method === 'Page.javascriptDialogOpening') { st.dialogs = (st.dialogs || 0) + 1; p.send('Page.handleJavaScriptDialog', { accept: true }).catch(() => {}) }
    for (const w of waiters) if (w.f(m)) { waiters.delete(w); w.r(m) }
  })
  const once = (f, ms = 10_000) => new Promise((r, j) => { const w = { f, r }; waiters.add(w); setTimeout(() => { if (waiters.delete(w)) j(new Error('timeout waiting for event')) }, ms) })
  await p.send('Network.enable')
  await p.send('Emulation.setScriptExecutionDisabled', { value: !js })
  const ev = (x, ms = 8000) => Promise.race([p.evaluate(x), sleep(ms).then(() => { throw new Error('evaluate timed out: ' + x.slice(0, 60)) })])
  // full page load triggered by `fn` (a navigation, a click, a form submit) → { url, docs }
  const full = async (fn) => {
    st.docs = []
    const l = once((m) => m.method === 'Page.loadEventFired')
    await fn()
    await l
    await sleep(js ? 150 : 30)
    return { url: await ev('location.pathname + location.search'), docs: [...st.docs] }
  }
  const go = (url) => full(() => p.send('Page.navigate', { url }))
  const hydrated = async (ms = 6000) => {
    const t = now()
    // a view mounted by its module (#brain-view.$d), or a prerendered page of resumable islands (§13: b/state scripts,
    // the island runtime loaded; each island mounts on its first real event)
    const q = `(() => { const v = document.getElementById('brain-view'); if (v && v.$d) return true
      return !!document.querySelector('script[type="b/state"]') && performance.getEntriesByType('resource').some(e => /\\/_b\\/[^/]+\\.js$/.test(e.name)) })()`
    while (now() - t < ms) { if (await ev(q)) return true; await sleep(30) }
    return false
  }
  return { p, ev, full, go, hydrated, st, close: () => p.close() }
}
const pageInfo = `(() => {
  const m = document.querySelector('main') || document.body
  const h1 = (document.querySelector('h1') || {}).textContent || ''
  const links = [...document.querySelectorAll('a[href]')].map(a => a.getAttribute('href')).filter(h => h.startsWith('/') && !h.startsWith('//'))
  return { h1: h1.trim(), text: (m.innerText || '').replace(/\\s+/g, ' ').trim().length, busy: !!document.querySelector('main[aria-busy=true], [role=progressbar]'),
    links: [...new Set(links)], forms: [...document.querySelectorAll('form')].map(f => ({ method: (f.getAttribute('method') || 'get').toLowerCase(), brain: (f.querySelector('input[name=_brain]') || {}).value || '', names: [...f.elements].map(e => e.name).filter(Boolean) })) }
})()`
const fill = (vals, events) => `(() => { const f = document.querySelector('main form'); for (const [k, v] of Object.entries(${JSON.stringify(vals)})) { const e = f.elements[k]; ${events ? 'e.focus();' : ''} e.value = v; ${events ? "e.dispatchEvent(new Event('input', { bubbles: true })); e.blur();" : ''} } return true })()`
// a native POST: the browser submits the form itself (no page script; HTML constraint validation skipped so the
// server's validation is what answers, as with `novalidate` or an old browser)
const NATIVE = `(HTMLFormElement.prototype.submit.call(document.querySelector('main form')), 1)`
const formState = `(() => { const f = document.querySelector('main form'); if (!f) return { values: {}, msgs: [] }; return { values: Object.fromEntries([...f.elements].filter(e => e.name && e.name !== '_brain' && e.type !== 'password').map(e => [e.name, e.value])),
  msgs: [...document.querySelectorAll('main .err, main [role=alert]')].map(e => e.textContent.trim()).filter(Boolean) } })()`

async function safe(a, name, fn) {
  try { await fn() } catch (e) { rec(a.name, name, 'step finished without an exception', 'run', false, String((e && e.message) || e), 'QA (see the checks of this step)') }
}

async function run() {
  const { port: cdpPort, close } = await launch()
  try {
    for (const a of apps) {
      if (!a.up) continue
      const B = `http://127.0.0.1:${a.px.port}`
      const pages = a.name === 'scaffold'
        ? ['/', '/about', '/posts', '/posts/hello-world', '/posts/second-post', '/login', '/account', '/403', '/no-such-page', '/probe-form', '/probe-client']
        : ['/', '/dashboard', '/no-such-page']
      const seen = {} // href → no-JS { url, h1 } (the reference for the JS run)

      // ===== JS disabled =====
      const n = await page(cdpPort, false)
      await n.p.send('Network.clearBrowserCookies')
      for (const path of pages) {
        const r = await n.go(B + path)
        const info = await n.ev(pageInfo)
        const st = r.docs.at(-1)?.status
        const expect404 = path === '/no-such-page', expect403 = path === '/403'
        const okStatus = expect404 ? st === 404 : expect403 ? st === 403 || st === 200 : st === 200
        rec(a.name, path, 'renders meaningful HTML (h1 + main text, no spinner)', 'no-JS',
          okStatus && info.h1 && (expect404 || info.text >= 30) && !info.busy, // an app without views/404.brain gets the bare fallback

          `status ${r.docs.map((d) => d.status).join('→')} url ${r.url}; h1 "${info.h1}"; main text ${info.text} chars${info.busy ? '; busy marker' : ''}`, 'BUILD prerender/SSR')
        seen[path] = { url: r.url, h1: info.h1 }
        // every internal link: a real <a href> that loads the target page
        const bad = []
        for (const href of info.links) {
          await n.go(B + path)
          const t = await n.full(() => n.ev(`document.querySelector('a[href="${href}"]').click()`)).catch((e) => ({ err: e.message }))
          if (t.err) { bad.push(`${href}: ${t.err}`); continue }
          const h1 = await n.ev(`((document.querySelector('h1') || {}).textContent || '').trim()`)
          seen['link:' + href] = { url: t.url, h1 }
          if (!h1 || t.docs.at(-1)?.status >= 400 && href !== '/403') bad.push(`${href} → ${t.url} ${t.docs.map((d) => d.status).join('→')} h1 "${h1}"`)
        }
        if (info.links.length) rec(a.name, path, `every internal link loads its page (${info.links.length})`, 'no-JS', bad.length === 0, bad.length ? bad.join('; ') : info.links.join(' '), 'BUILD prerender/SSR')
        for (const f of info.forms) seen['form:' + path] = f
      }

      if (a.name === 'scaffold') {
        // the form() lowering itself
        for (const path of ['/login', '/probe-form']) {
          const f = seen['form:' + path] || {}
          rec(a.name, path, 'form() with a `server:` submit is a real <form method="post"> (name= on fields, hidden _brain)', 'no-JS',
            f.method === 'post' && /^\w+\/\w+$/.test(f.brain) && f.names?.length >= 3, `method ${f.method}; _brain "${f.brain}"; names ${f.names?.join(',')}`, 'COMPILER form() → <form> lowering')
        }
        await safe(a, 'nojsLogin', () => nojsLogin(a, n, B))
        await safe(a, 'nojsProbeForm', () => nojsProbeForm(a, n, B))
        await safe(a, 'clientOnly', () => clientOnly(a, n, B, false))
        await safe(a, 'defences', () => defences(a, B))
      } else {
        const f = seen['form:/'] || {}
        rec(a.name, '/ (todo)', 'client-only form (no `server:` submit) stays a plain form, not a POST', 'no-JS', f.method !== 'post' && !f.brain, `method ${f.method}; _brain "${f.brain}"`, 'COMPILER form() → <form> lowering')
      }
      n.close()

      // ===== JS enabled =====
      const j = await page(cdpPort, true)
      await j.p.send('Network.clearBrowserCookies')
      for (const path of pages) {
        if (path === '/no-such-page') continue
        const r = await j.go(B + path)
        const hyd = await j.hydrated()
        const info = await j.ev(pageInfo)
        rec(a.name, path, 'renders and hydrates', 'JS', hyd && info.h1 && info.text >= 30, `url ${r.url}; hydrated ${hyd}; h1 "${info.h1}"`, 'RUNTIME')
        const bad = [], good = []
        for (const href of info.links) {
          if (href === path) continue
          await j.go(B + path)
          await j.hydrated()
          const token = Math.random().toString(36).slice(2)
          await j.ev(`window.__probe = '${token}'`)
          const loads0 = j.st.loads, h0 = await j.ev(`location.pathname + location.search`)
          await j.ev(`document.querySelector('a[href="${href}"]').click()`)
          const ref = seen['link:' + href]
          let url = h0, h1 = '', t = now()
          while (now() - t < 5000) {
            url = await j.ev('location.pathname + location.search').catch(() => url)
            h1 = await j.ev(`((document.querySelector('h1') || {}).textContent || '').trim()`).catch(() => '')
            if (ref ? url === ref.url && h1 === ref.h1 : url !== h0) break
            await sleep(40)
          }
          await sleep(100)
          const survived = await j.ev('window.__probe').catch(() => null) === token
          const reloads = j.st.loads - loads0
          const same = !ref || (url === ref.url && h1 === ref.h1)
          if (survived && reloads === 0 && url !== h0 && same) good.push(href)
          else bad.push(`${href}: url ${url} (no-JS ${ref?.url}), h1 "${h1}" (no-JS "${ref?.h1}"), probe ${survived ? 'kept' : 'lost'}, full loads ${reloads}`)
        }
        if (info.links.filter((h) => h !== path).length) rec(a.name, path, 'links navigate on the client (window probe survives, no load event, same page as no-JS)', 'JS', bad.length === 0, bad.length ? bad.join('; ') : good.join(' '), 'RUNTIME router')
      }
      if (a.name === 'scaffold') {
        await safe(a, 'jsLogin', () => jsLogin(a, j, B))
        await safe(a, 'jsProbeForm', () => jsProbeForm(a, j, B))
        await safe(a, 'clientOnly', () => clientOnly(a, j, B, true))
      } else {
        await j.go(B + '/')
        await j.hydrated()
        const loads0 = j.st.loads
        // like a user: type (the first input event resumes the island), then press Add once it is enabled
        await j.ev(`(() => { const i = document.querySelector('input[aria-label="New task"]'); i.focus(); i.value = 'buy milk'; i.dispatchEvent(new Event('input', { bubbles: true })); return 1 })()`)
        let t = now()
        while (now() - t < 3000 && await j.ev(`document.querySelector('input[aria-label="New task"]').form.querySelector('button').disabled`)) await sleep(30)
        await j.ev(`document.querySelector('input[aria-label="New task"]').form.querySelector('button').click(), 1`)
        await sleep(300)
        const items = await j.ev(`[...document.querySelectorAll('.todo li span')].map(s => s.textContent)`)
        rec(a.name, '/ (todo)', 'client-only form submits without a reload', 'JS', items.includes('buy milk') && j.st.loads === loads0, `items ${JSON.stringify(items)}; full loads ${j.st.loads - loads0}`, 'RUNTIME')
      }
      j.close()
    }
  } finally {
    close()
  }
}

// ---------- scaffold login, no JS ----------
async function nojsLogin(a, n, B) {
  const P = '/login?next=/posts'
  await n.p.send('Network.clearBrowserCookies')
  // wrong credentials
  await n.go(B + P)
  let c0 = a.px.count.posts.length
  await n.ev(fill({ email: 'qa@example.com', password: 'wrong-password' }))
  let r = await n.full(() => n.ev(NATIVE))
  let s = await n.ev(formState)
  rec(a.name, '/login', 'wrong credentials → 422, same page, typed email kept, "Wrong email or password" shown', 'no-JS',
    r.docs.at(-1)?.status === 422 && r.url === P && s.values.email === 'qa@example.com' && s.msgs.some((m) => /wrong email or password/i.test(m)),
    `status ${r.docs.map((d) => d.status).join('→')} url ${r.url}; values ${JSON.stringify(s.values)}; messages ${JSON.stringify(s.msgs)}; POSTs ${a.px.count.posts.length - c0}`,
    'COMPILER form() → <form> lowering (submit is an arrow calling `login`: the page re-renders without the posted state because form() only reads `submit.id`)')
  // invalid input (schema)
  await n.go(B + P)
  await n.ev(fill({ email: 'not-an-email', password: '' }))
  r = await n.full(() => n.ev(NATIVE))
  s = await n.ev(formState)
  rec(a.name, '/login', 'invalid input → 422, value kept, form() messages shown', 'no-JS',
    r.docs.at(-1)?.status === 422 && s.values.email === 'not-an-email' && s.msgs.length >= 2,
    `status ${r.docs.map((d) => d.status).join('→')}; values ${JSON.stringify(s.values)}; messages ${JSON.stringify(s.msgs)}`,
    'COMPILER form() → <form> lowering (same cause as above)')
  // right credentials → the `next` page
  await n.go(B + P)
  c0 = a.px.count.posts.length
  await n.ev(fill({ email: 'qa@example.com', password: PASSWORD || '' }))
  r = await n.full(() => n.ev(NATIVE))
  rec(a.name, '/login', 'right credentials → 303 to the `next` page (/posts)', 'no-JS',
    r.url === '/posts' && r.docs.some((d) => d.status === 303),
    `docs ${r.docs.map((d) => d.status + (d.location ? ' → ' + d.location.replace(B, '') : '')).join(', ')}; final url ${r.url}; POSTs ${a.px.count.posts.length - c0}`,
    'TOOLCHAIN scaffold template (views/login.brain: `login` returns the user, not { redirect: next }, so the PRG goes back to /login)')
  const acc = await n.go(B + '/account')
  const txt = await n.ev(`(document.querySelector('main') || document.body).innerText`)
  rec(a.name, '/account', 'after the no-JS login the session works (account shows the user)', 'no-JS', acc.url === '/account' && /qa@example\.com/.test(txt), `url ${acc.url}; ${JSON.stringify(txt.slice(0, 80))}`, 'RUNTIME server.mjs form handling')
  await n.p.send('Network.clearBrowserCookies')
}

// ---------- probe form, no JS ----------
const PROBE = {
  invalid: { name: '', email: 'bad', age: '12' },
  taken: { name: 'taken', email: 'qa@example.com', age: '20' },
  boom: { name: 'boom', email: 'qa@example.com', age: '20' },
  go: { name: 'go', email: 'qa@example.com', age: '20' },
  ok: { name: 'fine', email: 'qa@example.com', age: '20' },
}
const nojsOut = {}
async function nojsProbeForm(a, n, B) {
  for (const [k, vals] of Object.entries(PROBE)) {
    await n.go(B + '/probe-form')
    const c0 = a.px.count.posts.length
    await n.ev(fill(vals))
    const r = await n.full(() => n.ev(NATIVE))
    const s = await n.ev(formState)
    const chain = r.docs.map((d) => d.status).join('→')
    nojsOut[k] = { url: r.url, msgs: s.msgs, values: s.values }
    const posts = a.px.count.posts.length - c0
    const ev = `status ${chain} url ${r.url}; values ${JSON.stringify(s.values)}; messages ${JSON.stringify(s.msgs)}; POSTs ${posts}`
    const own = 'RUNTIME server.mjs form handling'
    if (k === 'invalid') rec(a.name, '/probe-form', 'invalid → 422, same page, typed values kept, form() messages', 'no-JS', r.docs.at(-1)?.status === 422 && r.url === '/probe-form' && s.values.email === 'bad' && s.values.age === '12' && s.msgs.length === 3, ev, own)
    if (k === 'taken') rec(a.name, '/probe-form', 'server { errors } → shown on its field', 'no-JS', r.docs.at(-1)?.status === 422 && s.values.name === 'taken' && s.msgs.includes('That name is taken'), ev, own)
    if (k === 'boom') rec(a.name, '/probe-form', 'unexpected server failure → generic message', 'no-JS', r.docs.at(-1)?.status >= 400 && s.msgs.some((m) => /something went wrong/i.test(m)) && !s.msgs.some((m) => /probe failure/.test(m)), ev, own)
    if (k === 'go') rec(a.name, '/probe-form', 'valid, function returns { redirect: "/about" } → 303 to /about', 'no-JS', r.docs[0]?.status === 303 && r.url === '/about', ev, own)
    if (k === 'ok') rec(a.name, '/probe-form', 'valid → 303 PRG to the same page', 'no-JS', r.docs[0]?.status === 303 && r.url === '/probe-form' && r.docs.at(-1)?.status === 200 && posts === 1, ev, own)
  }
}

// ---------- <meta client> ----------
const clientMeasure = `(() => { const ph = document.querySelector('[data-client]'), c = document.querySelector('main canvas'), a = document.getElementById('after');
  return { placeholder: ph ? Math.round(ph.getBoundingClientRect().height) : null, canvas: c ? Math.round(c.getBoundingClientRect().height) : null, mounted: c ? c.dataset.mounted || '' : '', afterTop: Math.round(a.getBoundingClientRect().top) } })()`
let clientNoJs = null
async function clientOnly(a, pg, B, js) {
  if (!js) {
    await pg.go(B + '/probe-client')
    clientNoJs = await pg.ev(clientMeasure)
    rec(a.name, '/probe-client', '<meta client> renders a reserved-size placeholder', 'no-JS', clientNoJs.placeholder >= 300, `placeholder height ${clientNoJs.placeholder}px (meta client height="320"); #after top ${clientNoJs.afterTop}px`, 'COMPILER <meta client>')
    return
  }
  await pg.ev(`window.__cls = 0; new PerformanceObserver(l => { for (const e of l.getEntries()) if (!e.hadRecentInput) window.__cls += e.value }).observe({ type: 'layout-shift', buffered: true }); 1`).catch(() => {})
  await pg.go(B + '/probe-client')
  await pg.ev(`window.__cls = 0; new PerformanceObserver(l => { for (const e of l.getEntries()) if (!e.hadRecentInput) window.__cls += e.value }).observe({ type: 'layout-shift', buffered: true }); 1`)
  await pg.hydrated()
  await sleep(500)
  const m = await pg.ev(clientMeasure)
  const cls = await pg.ev('window.__cls')
  const jump = clientNoJs ? Math.abs(m.afterTop - clientNoJs.afterTop) : null
  rec(a.name, '/probe-client', 'client mounts in place: no layout jump vs the placeholder', 'JS', m.mounted === 'yes' && jump !== null && jump <= 1 && cls < 0.01,
    `canvas ${m.canvas}px mounted "${m.mounted}"; #after top ${m.afterTop}px (no-JS ${clientNoJs?.afterTop}px, jump ${jump}px); CLS ${cls.toFixed(4)}`, 'COMPILER <meta client> / RUNTIME mount')
}

// ---------- defences (raw HTTP to brain start) ----------
async function defences(a, B) {
  const page = await raw(a.port, 'GET', '/probe-form')
  const id = (page.body.match(/name="_brain" value="([^"]+)"/) || [])[1] || ''
  const host = `127.0.0.1:${a.port}`, self = `http://${host}`
  const form = 'application/x-www-form-urlencoded'
  const body = `_brain=${encodeURIComponent(id)}&name=fine&email=qa%40example.com&age=20`
  const cases = [
    ['cross-site POST (Origin: https://evil.example) → 403', { origin: 'https://evil.example', 'content-type': form }, body, 403],
    ['cross-site POST (Sec-Fetch-Site: cross-site, no Origin) → 403', { 'sec-fetch-site': 'cross-site', 'content-type': form }, body, 403],
    ['missing Origin and Sec-Fetch-Site → 403', { 'content-type': form }, body, 403],
    ['JSON body → 415', { origin: self, 'content-type': 'application/json' }, JSON.stringify({ _brain: id, name: 'fine' }), 415],
    ['unknown _brain (unknown hash) → 400', { origin: self, 'content-type': form }, '_brain=zzzz/nope&name=x', 400],
    ['unknown _brain (known hash, unknown function) → 400', { origin: self, 'content-type': form }, `_brain=${id.split('/')[0]}/constructor&name=x`, 400],
    ['missing _brain → 400', { origin: self, 'content-type': form }, 'name=x', 400],
    ['control: same origin (Origin) → 303', { origin: self, 'content-type': form }, body, 303],
    ['control: same origin (Sec-Fetch-Site only) → 303', { 'sec-fetch-site': 'same-origin', 'content-type': form }, body, 303],
  ]
  for (const [name, headers, b, want] of cases) {
    const r = await raw(a.port, 'POST', '/probe-form', { host, ...headers, 'content-length': Buffer.byteLength(b) }, b)
    rec(a.name, 'POST /probe-form', name, 'raw HTTP', r.status === want, `status ${r.status}${r.headers?.location ? ' location ' + r.headers.location : ''}; body ${JSON.stringify(r.body.slice(0, 60))}`, 'RUNTIME server.mjs defences')
  }
}

// ---------- scaffold login, JS ----------
async function jsLogin(a, j, B) {
  const P = '/login?next=/posts'
  await j.p.send('Network.clearBrowserCookies')
  // form() starts from the server state: a native (JS-less) submit of wrong credentials, then hydration
  await j.go(B + P)
  await j.hydrated()
  await j.ev(fill({ email: 'qa@example.com', password: 'wrong-password' }))
  let r = await j.full(() => j.ev(NATIVE))
  await j.hydrated()
  await sleep(200)
  let s = await j.ev(formState)
  rec(a.name, '/login', 'after a native POST (422) form() hydrates from the server state (email kept, error shown)', 'JS',
    r.docs.at(-1)?.status === 422 && s.values.email === 'qa@example.com' && s.msgs.some((m) => /wrong email or password/i.test(m)),
    `status ${r.docs.map((d) => d.status).join('→')}; after hydration values ${JSON.stringify(s.values)} messages ${JSON.stringify(s.msgs)}`,
    'COMPILER form() → <form> lowering / RUNTIME client form() (arrow submit: no server id reaches form())')
  // wrong credentials with JS, double click
  await j.go(B + P)
  await j.hydrated()
  let c0 = a.px.count.posts.length, l0 = j.st.loads
  await j.ev(fill({ email: 'qa@example.com', password: 'wrong-password' }, true))
  await j.ev(`(() => { const b = document.querySelector('main form [type=submit]'); b.click(); b.click(); return 1 })()`)
  await sleep(1000)
  s = await j.ev(formState)
  let posts = a.px.count.posts.slice(c0)
  rec(a.name, '/login', 'wrong credentials: double click sends one POST, error shown, no reload (matches no-JS message)', 'JS',
    posts.length === 1 && j.st.loads === l0 && s.msgs.some((m) => /wrong email or password/i.test(m)),
    `POSTs ${posts.length} ${JSON.stringify(posts)}; full loads ${j.st.loads - l0}; messages ${JSON.stringify(s.msgs)}`, 'RUNTIME client form()')
  // right credentials → next, client navigation
  await j.go(B + P)
  await j.hydrated()
  const token = Math.random().toString(36).slice(2)
  await j.ev(`window.__probe = '${token}'`)
  c0 = a.px.count.posts.length; l0 = j.st.loads
  await j.ev(fill({ email: 'qa@example.com', password: PASSWORD || '' }, true))
  await j.ev(`(() => { const b = document.querySelector('main form [type=submit]'); b.click(); b.click(); return 1 })()`)
  let url = '', t = now()
  while (now() - t < 5000) { url = await j.ev('location.pathname + location.search').catch(() => url); if (url === '/posts') break; await sleep(50) }
  await sleep(200)
  posts = a.px.count.posts.slice(c0)
  const kept = (await j.ev('window.__probe').catch(() => null)) === token
  const who = await j.ev(`(document.querySelector('.who') || {}).textContent || ''`).catch(() => '')
  rec(a.name, '/login', 'right credentials: one POST, client navigation to `next` (/posts), probe survives', 'JS',
    posts.length === 1 && url === '/posts' && kept && j.st.loads === l0,
    `POSTs ${posts.length}; url ${url}; probe ${kept ? 'kept' : 'lost'}; full loads ${j.st.loads - l0}; header ${JSON.stringify(who.trim())}`, 'RUNTIME client form()')
  await j.p.send('Network.clearBrowserCookies')
}

// ---------- probe form, JS ----------
async function jsProbeForm(a, j, B) {
  // server state after a native POST
  await j.go(B + '/probe-form')
  await j.hydrated()
  await j.ev(fill(PROBE.invalid))
  const r = await j.full(() => j.ev(NATIVE))
  const hyd = await j.hydrated()
  await sleep(200)
  const s0 = await j.ev(formState)
  rec(a.name, '/probe-form', 'after a native POST (422) form() hydrates from the server state (values + messages)', 'JS',
    r.docs.at(-1)?.status === 422 && hyd && s0.values.email === 'bad' && s0.values.age === '12' && s0.msgs.length === 3,
    `status ${r.docs.map((d) => d.status).join('→')}; hydrated ${hyd}; values ${JSON.stringify(s0.values)}; messages ${JSON.stringify(s0.msgs)}`, 'RUNTIME client form()')
  for (const [k, vals] of Object.entries(PROBE)) {
    await j.go(B + '/probe-form')
    await j.hydrated()
    const token = Math.random().toString(36).slice(2)
    await j.ev(`window.__probe = '${token}'`)
    const c0 = a.px.count.posts.length, l0 = j.st.loads
    await j.ev(fill(vals, true))
    await j.ev(`(() => { const b = document.querySelector('main form [type=submit]'); b.click(); b.click(); return 1 })()`)
    await sleep(900)
    const s = await j.ev(formState).catch(() => ({ values: {}, msgs: [] }))
    const url = await j.ev('location.pathname + location.search')
    const kept = (await j.ev('window.__probe').catch(() => null)) === token
    const posts = a.px.count.posts.length - c0
    const ref = nojsOut[k] || {}
    const ev = `POSTs ${posts}; url ${url} (no-JS ${ref.url}); messages ${JSON.stringify(s.msgs)} (no-JS ${JSON.stringify(ref.msgs)}); probe ${kept ? 'kept' : 'lost'}; full loads ${j.st.loads - l0}`
    const same = (x, y) => JSON.stringify(x) === JSON.stringify(y)
    const noReload = kept && j.st.loads === l0
    if (k === 'invalid') rec(a.name, '/probe-form', 'invalid: no POST, same messages as no-JS', 'JS', posts === 0 && noReload && same(s.msgs, ref.msgs), ev, 'RUNTIME client form()')
    if (k === 'taken') rec(a.name, '/probe-form', 'server { errors }: one POST (double click), same field message as no-JS', 'JS', posts === 1 && noReload && same(s.msgs, ref.msgs), ev, 'RUNTIME client form()')
    if (k === 'boom') rec(a.name, '/probe-form', 'server failure: one POST, same generic message as no-JS', 'JS', posts === 1 && noReload && same(s.msgs, ref.msgs), ev, 'RUNTIME client form()')
    if (k === 'go') rec(a.name, '/probe-form', '{ redirect: "/about" }: one POST, client navigation to /about like no-JS', 'JS', posts === 1 && url === '/about' && kept, ev, 'RUNTIME client form() (the returned redirect is ignored)')
    if (k === 'ok') rec(a.name, '/probe-form', 'valid: one POST (double click), no error, stays on /probe-form like the no-JS PRG', 'JS', posts === 1 && noReload && url === '/probe-form' && s.msgs.length === 0, ev, 'RUNTIME client form()')
  }
}

// ---------- output ----------
function write() {
  const failed = rows.filter((r) => !r.pass)
  const summary = {
    task: 157, section: 'INTERNALS §38', date: new Date().toISOString().slice(0, 10), binary: { path: tilde(BRAIN), sha256_12: SHA },
    node: process.version, apps: { scaffold: tilde(SCAF), examples: tilde(EX), examplesWorkaround: exWorkaround || null },
    total: rows.length, passed: rows.length - failed.length, failed: failed.length, rows,
  }
  writeFileSync(join(RES, 'summary.json'), JSON.stringify(summary, null, 2) + '\n')
  const esc = (s) => String(s).replace(/\|/g, '\\|').replace(/\n/g, ' ')
  const md = [`# HTML first QA (#157) — ${summary.date}`, '',
    `Binary sha256 \`${SHA}\` (${tilde(BRAIN)}), Node ${process.version}. ${summary.passed}/${summary.total} checks pass, ${summary.failed} fail.`,
    exWorkaround ? `Examples workaround: ${exWorkaround}.` : '', '',
    '| App | Page / form | Check | Mode | Result | Evidence | Owner |', '|---|---|---|---|---|---|---|',
    ...rows.map((r) => `| ${r.app} | ${esc(r.target)} | ${esc(r.check)} | ${r.mode} | ${r.pass ? 'PASS' : '**FAIL**'} | ${esc(r.evidence)} | ${esc(r.owner)} |`), ''].join('\n')
  writeFileSync(join(RES, 'summary.md'), md)
  console.log(`\n${summary.passed}/${summary.total} pass · results in ${tilde(RES)}`)
  process.exitCode = failed.length ? 1 : 0
}

// ---------- main (last, after every declaration above) ----------
try {
  const sp = await freePort()
  const sS = startApp(SCAF, sp, ['--env', 'development'])
  apps.push({ name: 'scaffold', dir: SCAF, port: sp, sup: sS, up: await waitHealth(sp) })
  if (exBuild.code === 0) {
    const ep = await freePort()
    const sE = startApp(EX, ep)
    apps.push({ name: 'examples', dir: EX, port: ep, sup: sE, up: await waitHealth(ep) })
  }
  for (const a of apps) {
    rec(a.name, 'brain start', 'serves dist + server.mjs (/_health 200)', 'start', a.up, `port ${a.port}${a.sup.err ? '; stderr ' + a.sup.err.trim().split('\n').pop() : ''}`, 'TOOLCHAIN brain start')
    a.px = await proxy(a.port)
  }
  await run()
} catch (e) {
  rec('run', 'harness', 'no exception', 'run', false, String(e && e.stack || e), 'QA')
} finally {
  let left = []
  for (const s of started) left.push(...(await stopApp(s)) ?? [])
  for (const a of apps) await a.px?.close()
  rec('run', 'cleanup', 'no process left', 'run', left.length === 0, left.length ? `alive: ${left.join(', ')}` : `stopped ${started.length} supervisors and their children`, 'QA')
  write()
}
