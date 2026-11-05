// New sites own an editable brand reference while builds refresh only source-derived facts.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { mkdtempSync, readFileSync, writeFileSync, rmSync, existsSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
const brain = process.env.BRAIN_BIN ?? new URL('../../target/debug/brain', import.meta.url).pathname
function cli(args, cwd, success = true) {
  const result = spawnSync(brain, args, { cwd, encoding: 'utf8', timeout: 60000, maxBuffer: 4 * 1024 * 1024 })
  if (success) assert.equal(result.status, 0, result.stdout + result.stderr)
  return { result, report: JSON.parse(result.stdout) }
}
test('managed multilingual brandbooks retain dictionary identities and check/build preserve source contracts', { timeout: 240000 }, () => {
  const root = mkdtempSync(join(tmpdir(), 'brain-brandbook-locales-'))
  try {
    for (const mode of ['static', 'ordinary']) {
      cli(['new', mode, ...(mode === 'static' ? ['--static'] : []), '--json'], root)
      const app = join(root, mode)
      cli(['add', 'locale', 'es', '--json'], app)
      const designPath = join(app, 'design.brain.json'), viewPath = join(app, 'views/brandbook.brain')
      const design = readFileSync(designPath, 'utf8'), view = readFileSync(viewPath, 'utf8')
      const key = JSON.parse(design).brandbook.messages.Strategy
      assert.equal(typeof key, 'string', design)
      assert.ok(view.includes('<section data-brain-brandbook="content"></section>'))
      for (const [locale, text] of [['en', 'Reviewed strategy'], ['es', 'Translated strategy']]) {
        const path = join(app, `i18n/${locale}.json`), dictionary = JSON.parse(readFileSync(path, 'utf8'))
        dictionary[key] = text
        dictionary['brandbook.root'] = locale === 'en' ? 'root' : 'DO NOT translate technical facts'
        writeFileSync(path, JSON.stringify(dictionary))
      }
      const diagnostics = cli(['check', '--json'], app).report
      assert.ok(!diagnostics.some(diagnostic => diagnostic.code === 'B1403'), JSON.stringify(diagnostics))
      cli(['build', '--site', 'https://example.test', '--json'], app)
      assert.ok(readFileSync(join(app, 'dist/brandbook/index.html'), 'utf8').includes('Reviewed strategy'))
      const translated = readFileSync(join(app, 'dist/es/brandbook/index.html'), 'utf8')
      assert.ok(translated.includes('Translated strategy'))
      assert.ok(translated.includes('<code>root</code>') && !translated.includes('DO NOT translate technical facts'), 'technical declaration contexts must stay identical to source')
      assert.equal(readFileSync(designPath, 'utf8'), design, 'checks and builds never rewrite the message identity map')
      assert.equal(readFileSync(viewPath, 'utf8'), view, 'the live placeholder remains authored source')
    }
  } finally { rmSync(root, { recursive: true, force: true }) }
})
test('new static and ordinary sites build a living brandbook with stable editorial ownership and current design facts', { timeout: 180000 }, () => {
  const root = mkdtempSync(join(tmpdir(), 'brain-living-brandbook-'))
  try {
    for (const mode of ['static', 'ordinary']) {
      cli(['new', mode, ...(mode === 'static' ? ['--static'] : []), '--json'], root)
      const app = join(root, mode)
      assert.ok(existsSync(join(app, 'views/brandbook.brain')))
      assert.ok(readFileSync(join(app, 'views/_layout.brain'), 'utf8').includes('href="/brandbook"'))
      const design = join(app, 'design.brain.json')
      const data = JSON.parse(readFileSync(design, 'utf8'))
      data.brandbook.sections.voice = { status: 'approved', text: 'Use clear, specific language and show one useful recovery action.', evidence: ['Project owner decision recorded in the public brand reference.'] }
      writeFileSync(design, JSON.stringify(data))
      const editorial = readFileSync(design, 'utf8')
      cli(['check', '--json'], app)
      cli(['build', '--site', 'https://example.test', '--json'], app)
      const landing = 'https://chrisifm.github.io/BRAINLLM/'
      for (const file of ['robots.txt', 'sitemap.xml', 'llms.txt', 'llms-full.txt']) {
        assert.ok(readFileSync(join(app, 'dist', file), 'utf8').includes(landing), mode + ': ' + file)
      }
      assert.equal(JSON.parse(readFileSync(join(app, 'dist/brain-build.json'), 'utf8')).generator.url, landing)
      if (mode === 'ordinary') assert.ok(readFileSync(join(app, 'dist/server.mjs'), 'utf8').includes('/* by BRAINLLM ' + landing + ' */'))
      const path = join(app, 'dist/brandbook/index.html')
      let html = readFileSync(path, 'utf8')
      for (const text of ['Strategy', 'Voice', 'Identity and product', 'Assistant', 'Channels and governance', 'Research', 'Current design', 'clear, specific language']) assert.ok(html.includes(text), text)
      assert.ok(html.includes('approved.') && html.includes('draft.'))
      assert.ok(html.includes('by BRAINLLM https://chrisifm.github.io/BRAINLLM/'))
      assert.equal(readFileSync(design, 'utf8'), editorial)
      const tokens = join(app, 'tokens.cssbrain')
      writeFileSync(tokens, readFileSync(tokens, 'utf8') + '\n:root { --brandbook-proof: #13579b; }\n')
      writeFileSync(join(app, 'ui/brandbook-proof.brain'), '<span>Proof</span>')
      cli(['build', '--site', 'https://example.test', '--json'], app)
      html = readFileSync(path, 'utf8')
      assert.ok(html.includes('--brandbook-proof') && html.includes('#13579b') && html.includes('ui/brandbook-proof.brain'))
      assert.equal(readFileSync(design, 'utf8'), editorial)
      if (mode === 'static') assert.ok(!/type="module"/.test(html), 'the pure static brand reference does not require client hydration')
      assert.equal(existsSync(join(app, 'dist/server.mjs')), mode === 'ordinary')
    }
  } finally { rmSync(root, { recursive: true, force: true }) }
})
