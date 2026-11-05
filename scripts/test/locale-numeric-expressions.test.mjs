// Adding a language must preserve arithmetic inside the messages it extracts.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { mkdtempSync, writeFileSync, readFileSync, rmSync } from 'node:fs'
import { join } from 'node:path'
import { tmpdir } from 'node:os'
const brain = process.env.BRAIN_BIN ?? new URL('../../target/debug/brain', import.meta.url).pathname
test('adding a locale preserves evaluated numeric groups in static page text and attributes', { timeout: 180000 }, () => {
  const root = mkdtempSync(join(tmpdir(), 'brain-locale-numeric-'))
  const cli = (args, cwd) => {
    const result = spawnSync(brain, args, { cwd, encoding: 'utf8', timeout: 60000, maxBuffer: 4 * 1024 * 1024 })
    assert.equal(result.status, 0, result.stdout + result.stderr)
  }
  try {
    cli(['new', 'example', '--static', '--json'], root)
    const app = join(root, 'example')
    writeFileSync(join(app, 'views/home.brain'), `<meta seo title="Example numbered steps" description="Example numbered steps preserve arithmetic when their visible sentences become translated messages, including grouped expressions in accessible attributes." keyword="example">
<script>let i = sig(0)
let n = sig(4)</script>
<main><h1>Example numbered steps</h1><p>{i + 1 + ' of ' + n}</p><p title={'Step ' + (i + 1)}>Step preview</p><p>{('Page ') + (i + 1)}</p></main>`)
    cli(['add', 'locale', 'es', '--json'], app)
    const dictionary = JSON.parse(readFileSync(join(app, 'i18n/en.json'), 'utf8'))
    assert.ok(Object.values(dictionary).includes('{value} of {n}'))
    assert.ok(!Object.values(dictionary).some(value => typeof value === 'string' && value.includes('{i}{value}')))
    cli(['build', '--site', 'https://example.test', '--json'], app)
    for (const path of ['index.html', 'es/index.html']) {
      const html = readFileSync(join(app, 'dist', path), 'utf8').replace(/<!--[\s\S]*?-->/g, '')
      assert.ok(html.includes('>1 of 4</p>'), html)
      assert.ok(html.includes('title="Step 1"'), html)
      assert.ok(html.includes('>Page 1</p>'), html)
      assert.ok(!html.includes('01 of 4'), html)
    }
  } finally { rmSync(root, { recursive: true, force: true }) }
})
