// Language fixtures (INTERNALS §37, B1401–B1405 and B0306). Each case is a mini app generated in a temp dir, because
// the i18n setup (brain.json `i18n`, `i18n/<locale>.json`) belongs to the whole app. Every case starts from the same
// valid control app (3 locales, one of them right-to-left, every visible string through `t()`, logical CSS only) and
// applies exactly one change, so the expected code must be the only code in the output.
// Repository rule: no Spanish or Arabic in files, so the other locales hold English text tagged with the locale.
// Usage: node bench/fixtures-pending/i18n/run.mjs [path to brain]
//   → exit 1 if the control app is not clean, a case shows other codes (EXTRA) or the expected code is absent (MISS).
import { execFileSync } from 'node:child_process'
import { mkdtempSync, writeFileSync, mkdirSync, readFileSync, rmSync } from 'node:fs'
import { join } from 'node:path'
import { tmpdir } from 'node:os'

const ROOT = new URL('../../..', import.meta.url).pathname
const BRAIN = process.argv[2] ?? join(ROOT, 'target/debug/brain')
const base = mkdtempSync(join(tmpdir(), 'brainllm-i18n-'))

// ---------- the valid control app ----------
const KEYS = {
  'nav.home': 'Home page',
  'nav.about': 'About this site',
  'home.title': 'Welcome to the probe',
  'home.lead': 'A page that reads every visible string through its key.',
  'home.greeting': 'Hello again {name}',
  'posts.count': { one: '{n} new post', other: '{n} new posts' },
  'about.title': 'About the probe',
  'about.body': 'This site exists to test the language checks of the toolchain.',
  'notfound.title': 'Page not found',
  'notfound.body': 'The address may be mistyped, or the page was moved.',
  'notfound.back': 'Back to the home page',
  'offline.title': 'You are offline',
  'offline.body': 'This page has not been saved on this device yet.',
  'offline.back': 'Try the home page',
}
const tag = (v, l) => (typeof v === 'string' ? `${v} [${l}]` : Object.fromEntries(Object.entries(v).map(([k, s]) => [k, tag(s, l)])))
const localeFile = (l, over = {}) => {
  const o = Object.fromEntries(Object.entries(KEYS).map(([k, v]) => [k, l === 'en' ? v : tag(v, l)]))
  return JSON.stringify({ ...o, ...over }, null, 2) + '\n'
}
const seo = (title, desc, extra = '') => `<meta seo title="${title}" description="${desc}"${extra}>`
const APP = `<meta seo titleTemplate="%s · probe" og:image="/og.png">

<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <link rel="icon" href="/favicon.svg" type="image/svg+xml">
</head>
<body>
  <header class="top">
    <nav aria-label="Main">
      <a href="/">{t('nav.home')}</a>
      <a href="/about">{t('nav.about')}</a>
    </nav>
  </header>
  <slot />
</body>
</html>

<style>
  .top { display: flex; gap: var(--space-4); padding-block: var(--space-3); padding-inline: var(--space-5); }
  nav { display: flex; gap: var(--space-4); }
</style>
`
const home = ({ markup = '', css = '' } = {}) => `${seo('Language probe home page', 'A probe page for the language checks of the BRAINLLM toolchain fixtures, written once with keys.', ' canonical="/"')}
<script>
  let name = sig('Ada')
  let n = sig(2)
</script>
<main class="page">
  <h1>{t('home.title')}</h1>
  <p class="lead">{t('home.lead')}</p>
  <p>{t('home.greeting', { name })}</p>
  <p>{t('posts.count', { n })}</p>
${markup}</main>
<style>
  .page { max-width: 60ch; margin-inline: auto; padding-inline-start: var(--space-4); text-align: start; }
  .lead { margin-block-end: var(--space-3); }
${css}</style>
`
const simple = (k, meta = '', robots = '') => `${seo(`Language probe ${k} page`, `The ${k} page of the probe app used by the language checks of the BRAINLLM toolchain fixtures.`, robots)}
${meta}<main class="page">
  <h1>{t('${k}.title')}</h1>
  <p>{t('${k}.body')}</p>
${k === 'about' ? '' : `  <p><a href="/">{t('${k}.back')}</a></p>\n`}</main>
<style>
  .page { max-width: 60ch; margin-inline: auto; text-align: center; }
</style>
`

function makeApp(name, { locales = ['en', 'es', 'ar'], files = {}, homeOpts, aboutMeta = '' } = {}) {
  const app = join(base, name)
  mkdirSync(app, { recursive: true })
  execFileSync(BRAIN, ['new', '.'], { cwd: app, stdio: 'ignore' })
  const cfg = JSON.parse(readFileSync(join(app, 'brain.json'), 'utf8'))
  cfg.i18n = { default: locales[0], locales }
  writeFileSync(join(app, 'brain.json'), JSON.stringify(cfg, null, 2) + '\n')
  mkdirSync(join(app, 'i18n'), { recursive: true })
  for (const l of locales) if (/^[a-z]{2}(-[A-Za-z0-9]+)?$/.test(l)) writeFileSync(join(app, `i18n/${l}.json`), localeFile(l))
  writeFileSync(join(app, 'app.brain'), APP)
  writeFileSync(join(app, 'views/home.brain'), home(homeOpts))
  writeFileSync(join(app, 'views/about.brain'), simple('about', aboutMeta))
  writeFileSync(join(app, 'views/404.brain'), simple('notfound', '', ' robots="noindex"'))
  writeFileSync(join(app, 'views/_offline.brain'), simple('offline', '', ' robots="noindex"'))
  // The scaffold's demo component and its test carry fixed text that is not part of these cases.
  for (const f of ['comp/counter.brain', 'comp/counter.test.brain', 'ui/badge.brain']) rmSync(join(app, f), { force: true })
  for (const [p, s] of Object.entries(files)) writeFileSync(join(app, p), s)
  return app
}

function run(app, cmd) {
  let out = ''
  try { out = execFileSync(BRAIN, cmd, { cwd: app, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'], timeout: 120_000 }) }
  catch (e) { out = `${e.stdout ?? ''}${e.stderr ?? ''}` }
  let diags = []
  try { const j = JSON.parse(out.slice(out.indexOf('['))); diags = Array.isArray(j) ? j : [] }
  catch { diags = [...out.matchAll(/\b(B\d{4})\b/g)].map((m) => ({ code: m[1], file: '?', level: '?' })) }
  return { out, diags }
}

// ---------- cases: one change on top of the control app each ----------
const CHECK = ['check', '--json', '.']
const cases = [
  { id: 'B1401', label: 'view calls t(\'nav.contact\'), key in no locale file', expect: 'B1401', count: 3,
    app: { homeOpts: { markup: "  <p>{t('nav.contact')}</p>\n" } } },
  { id: 'B1401-file', label: 'i18n/es.json is not valid JSON', expect: 'B1401',
    app: { files: { 'i18n/es.json': '{ "nav.home": "Home page [es]"\n' } } },
  { id: 'B1402', label: 'es value identical to en, several words (warning)', expect: 'B1402', level: 'warning',
    app: { files: { 'i18n/es.json': localeFile('es', { 'home.lead': KEYS['home.lead'] }) } } },
  { id: 'B1402-word', label: 'es value identical to en, one word (info)', expect: 'B1402', level: 'info',
    app: { files: { 'i18n/en.json': localeFile('en', { 'nav.home': 'Home' }), 'i18n/es.json': localeFile('es', { 'nav.home': 'Home' }) } } },
  { id: 'B1403', label: 'fixed text <p>Welcome back</p> with 3 locales', expect: 'B1403', count: 4,
    app: { homeOpts: { markup: '  <p>Welcome back</p>\n' } } },
  { id: 'B1403-one', label: 'same fixed text, single-locale site (must stay silent)', expect: null,
    app: { locales: ['en'], homeOpts: { markup: '  <p>Welcome back</p>\n' } } },
  { id: 'B1404', label: "t('home.greeting') without { name }", expect: 'B1404', count: 1,
    app: { homeOpts: { markup: "  <p>{t('home.greeting')}</p>\n" } } },
  { id: 'B1404-file', label: 'es uses {username} where en uses {name}', expect: 'B1404',
    app: { files: { 'i18n/es.json': localeFile('es', { 'home.greeting': 'Hello again {username} [es]' }) } } },
  { id: 'B1405', label: 'brain.json locales include "klingon"', expect: 'B1405',
    app: { locales: ['en', 'es', 'ar', 'klingon'], files: { 'i18n/klingon.json': localeFile('klingon', {}) } } },
  { id: 'B1401-missing', label: 'brain.json declares fr but i18n/fr.json does not exist (edit creates it)', expect: 'B1401',
    app: { locales: ['en', 'es', 'ar', 'fr'] } },
  { id: 'B1406', label: '<meta route es=… fr=…>, fr is not a site locale', expect: 'B1406', count: 1,
    app: { aboutMeta: '<meta route es="/es/about-us" ar="/ar/about-us" fr="/fr/about-us">\n' } },
  { id: 'B0306', label: 'margin-left + text-align: right with an RTL locale', expect: 'B0306', count: 2,
    app: { homeOpts: { css: '  .x { margin-left: var(--space-2); text-align: right; }\n', markup: '  <p class="x">{t(\'home.lead\')}</p>\n' } } },
  { id: 'B0306-ltr', label: 'same CSS, LTR locales only (must stay silent)', expect: null,
    app: { locales: ['en', 'es'], homeOpts: { css: '  .x { margin-left: var(--space-2); text-align: right; }\n', markup: '  <p class="x">{t(\'home.lead\')}</p>\n' } } },
]

const fmt = (d) => `${d.code}@${d.file}${d.level && d.level !== '?' ? `(${d.level})` : ''}`
let fail = 0
const misses = []

const control = makeApp('control')
const c0 = run(control, CHECK)
const cOk = c0.diags.length === 0
if (!cOk) fail++
console.log(`${cOk ? 'OK   ' : 'FAIL '} control  3 locales (en, es, ar=RTL), all text through t(), logical CSS → ${cOk ? '0 diagnostics' : c0.diags.map(fmt).join(', ')}`)

for (const c of cases) {
  const app = makeApp(c.id.toLowerCase(), c.app)
  const { diags } = run(app, CHECK)
  const hits = c.expect ? diags.filter((d) => d.code === c.expect) : []
  const others = diags.filter((d) => d.code !== c.expect)
  const levelOk = !c.level || hits.some((d) => d.level === c.level)
  const countOk = !c.count || hits.length === c.count
  let status
  if (others.length) status = 'EXTRA'
  else if (c.expect && !hits.length) status = 'MISS '
  else if (!levelOk || !countOk) status = 'DIFF '
  else status = 'OK   '
  if (status !== 'OK   ') fail++
  if (status === 'MISS ') misses.push(c.id)
  const want = c.expect ? `${c.expect}${c.count ? ` ×${c.count}` : ''}${c.level ? ` (${c.level})` : ''}` : 'nothing'
  console.log(`${status} ${c.id.padEnd(11)} ${c.label}\n      expected ${want} · seen ${diags.length ? diags.map(fmt).join(', ') : 'nothing'}`)
}

console.log(`\napps in ${base} · command: brain ${CHECK.join(' ')}`)
if (misses.length) console.log(`MISS (proved by crates/brain-check/tests/i18n.rs, not reachable from the CLI yet): ${misses.join(', ')}`)
process.exit(fail ? 1 : 0)
