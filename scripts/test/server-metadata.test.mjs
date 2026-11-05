// Server hook examples in strings must never become executable build capabilities.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { mkdtempSync, writeFileSync, readFileSync, rmSync } from 'node:fs'
import { join } from 'node:path'
import { tmpdir } from 'node:os'
const brain = process.env.BRAIN_BIN ?? new URL('../../target/debug/brain', import.meta.url).pathname
const cli = (args, cwd) => {
  const result = spawnSync(brain, args, { cwd, encoding: 'utf8', timeout: 60000, maxBuffer: 4 * 1024 * 1024 })
  assert.equal(result.status, 0, result.stdout + result.stderr)
  return JSON.parse(result.stdout)
}
test('static and ordinary builds ignore hook-shaped text in SEO and map only real load exports', { timeout: 180000 }, () => {
  const root = mkdtempSync(join(tmpdir(), 'brain-server-metadata-'))
  try {
    for (const mode of ['static', 'ordinary']) {
      cli(['new', mode, ...(mode === 'static' ? ['--static'] : []), '--json'], root)
      const app = join(root, mode)
      writeFileSync(join(app, 'views/about.brain'), `<meta seo title="About this project" description="Examples include function load(, function paths(, function sitemap( and const $forms. These are documentation text and never executable server declarations." canonical="/about">
<script>
// function load() is an example in a comment.
</script>
<main><h1>About this project</h1><p>Read the examples.</p></main>`)
      for (let pass = 0; pass < 2; pass++) {
        cli(['build', '--site', 'https://example.test', '--json'], app)
        const html = readFileSync(join(app, 'dist/about/index.html'), 'utf8')
        assert.ok(html.includes('function paths(') && html.includes('<h1>About this project</h1>'))
        assert.ok(!html.includes('<!--brain-body-->'), 'the page is ordinary prerendered output')
        const map = cli(['map', '--json'], app)
        const about = map.routes.find(route => route.file === 'views/about.brain')
        assert.equal(about.load, false)
      }
    }
  } finally { rmSync(root, { recursive: true, force: true }) }
})
