// Real trailing-slash multilingual builds must agree with browser and SSR route identity.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, rmSync, existsSync } from 'node:fs'
import { join } from 'node:path'
import { tmpdir } from 'node:os'
const brain = process.env.BRAIN_BIN ?? new URL('../../target/debug/brain', import.meta.url).pathname
const cli = (args, cwd) => {
  const result = spawnSync(brain, args, { cwd, encoding: 'utf8', timeout: 60000, maxBuffer: 4 * 1024 * 1024 })
  assert.equal(result.status, 0, result.stdout + result.stderr)
  return JSON.parse(result.stdout)
}
test('localized trailing-slash links and canonicals build under a base without duplicating dynamic language prefixes', { timeout: 180000 }, () => {
  const root = mkdtempSync(join(tmpdir(), 'brain-localized-links-'))
  try {
    for (const mode of ['static', 'ordinary']) {
      cli(['new', mode, ...(mode === 'static' ? ['--static'] : []), '--json'], root)
      const app = join(root, mode), configPath = join(app, 'brain.json')
      const config = JSON.parse(readFileSync(configPath, 'utf8'))
      config.base = '/app/'
      config.i18n = { default: 'en', locales: ['en', 'es'] }
      config.seo = { ...config.seo, trailingSlash: 'always' }
      writeFileSync(configPath, JSON.stringify(config))
      mkdirSync(join(app, 'i18n'), { recursive: true })
      const dictionary = existsSync(join(app, 'i18n/en.json')) ? JSON.parse(readFileSync(join(app, 'i18n/en.json'), 'utf8')) : {}
      writeFileSync(join(app, 'i18n/en.json'), JSON.stringify(dictionary))
      const translate = value => typeof value === 'string' ? value + ' [translated]' : Object.fromEntries(Object.entries(value).map(([key, child]) => [key, translate(child)]))
      writeFileSync(join(app, 'i18n/es.json'), JSON.stringify(translate(dictionary)))
      writeFileSync(join(app, 'views/about.brain'), `<meta route es="/es/contact-page">
<meta seo title="About this example project" description="Read the example project details and follow its language-specific routes, preserving trailing slashes, query parameters and local fragments." keyword="example" canonical="/about/">
<main><h1>About this example project</h1><p id="details">Example project details.</p><a href="/about/?ref=proof#details">Details</a><a href="/es/">Localized home</a></main>`)
      writeFileSync(join(app, 'views/[slug].brain'), `<meta seo title="Example dynamic page" description="Read this example dynamic page, generated from an explicit list of route parameters and available consistently in each configured language." keyword="example">
<script>export function paths() { return [{ slug: 'sample' }] }</script><main><h1>Example dynamic page</h1></main>`)
      cli(['build', '--site', 'https://example.test', '--json'], app)
      const html = readFileSync(join(app, 'dist/es/contact-page/index.html'), 'utf8')
      assert.ok(html.includes('href="/app/es/contact-page/?ref=proof#details"'))
      assert.ok(html.includes('href="/app/es/"'))
      assert.ok(!html.includes('/app/es/es'))
      assert.ok(html.includes('rel="canonical" href="https://example.test/app/es/contact-page/"'), html)
      const ordinary = readFileSync(join(app, 'dist/about/index.html'), 'utf8')
      assert.ok(ordinary.includes('rel="canonical" href="https://example.test/app/about/"'), ordinary)
    }
  } finally { rmSync(root, { recursive: true, force: true }) }
})
