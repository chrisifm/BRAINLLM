// The actual compiler/prerender pipeline must not load one language and render another.
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
test('localized static and ordinary prerenders load data in each page language before rendering', { timeout: 180000 }, () => {
  const root = mkdtempSync(join(tmpdir(), 'brain-prerender-locale-'))
  try {
    for (const mode of ['static', 'ordinary']) {
      cli(['new', mode, ...(mode === 'static' ? ['--static'] : []), '--json'], root)
      const app = join(root, mode), configPath = join(app, 'brain.json')
      const config = JSON.parse(readFileSync(configPath, 'utf8'))
      config.i18n = { default: 'en', locales: ['en', 'es'] }
      writeFileSync(configPath, JSON.stringify(config))
      mkdirSync(join(app, 'i18n'), { recursive: true })
      const dictionary = existsSync(join(app, 'i18n/en.json')) ? JSON.parse(readFileSync(join(app, 'i18n/en.json'), 'utf8')) : {}
      dictionary['probe.content'] = 'Default example content'
      writeFileSync(join(app, 'i18n/en.json'), JSON.stringify(dictionary))
      const translate = value => typeof value === 'string' ? value + ' [translated]' : Object.fromEntries(Object.entries(value).map(([key, child]) => [key, translate(child)]))
      const translated = translate(dictionary)
      translated['probe.content'] = 'Translated example content'
      writeFileSync(join(app, 'i18n/es.json'), JSON.stringify(translated))
      mkdirSync(join(app, 'views/articles'), { recursive: true })
      writeFileSync(join(app, 'views/articles/[slug].brain'), `<meta seo title="Localized example content" description="Read localized example content selected while loading this generated page, with the same language throughout its complete static HTML." keyword="example">
<script>
export function paths() { return [{ slug: 'sample' }] }
export async function load() { await Promise.resolve(); return { language: locale(), message: t('probe.content') } }
</script>
<main><h1>Localized example content</h1><p>{props.language}|{props.message}</p></main>`)
      cli(['build', '--site', 'https://example.test', '--json'], app)
      for (const [path, locale, message] of [['articles/sample/index.html', 'en', 'Default example content'], ['es/articles/sample/index.html', 'es', 'Translated example content']]) {
        const html = readFileSync(join(app, 'dist', path), 'utf8')
        assert.ok(html.includes(`lang="${locale}"`), path)
        assert.ok(html.replace(/<!--[\s\S]*?-->/g, '').includes(`${locale}|${message}`), path + ': ' + html)
        assert.ok(!html.includes(locale === 'en' ? 'Translated example content' : 'Default example content'), path)
      }
    }
  } finally { rmSync(root, { recursive: true, force: true }) }
})
