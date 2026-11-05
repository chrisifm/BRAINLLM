// Caller-owned public IDs must survive both production pipelines; static doctor checks emitted associations.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, rmSync } from 'node:fs'
import { join } from 'node:path'
import { tmpdir } from 'node:os'
const brain = process.env.BRAIN_BIN ?? new URL('../../target/debug/brain', import.meta.url).pathname
test('static and ordinary builds preserve public prop IDs; static doctor rejects missing ID targets', { timeout: 240000 }, () => {
  const root = mkdtempSync(join(tmpdir(), 'brain-public-id-'))
  const cli = (args, cwd, success = true) => {
    const r = spawnSync(brain, args, { cwd, encoding: 'utf8', timeout: 90000, maxBuffer: 4 * 1024 * 1024 })
    if (success) assert.equal(r.status, 0, r.stdout + r.stderr)
    else assert.notEqual(r.status, 0)
    return JSON.parse(r.stdout)
  }
  try {
    for (const mode of ['static', 'ordinary']) {
      cli(['new', mode, ...(mode === 'static' ? ['--static'] : []), '--json'], root)
      const app = join(root, mode)
      mkdirSync(join(app, 'comp'), { recursive: true })
      writeFileSync(join(app, 'comp/public-heading.brain'), `<meta props="heading:str">
<section><h2 id={heading}>Example section</h2><p aria-labelledby={heading}>Example description</p></section>`)
      writeFileSync(join(app, 'views/home.brain'), `<meta seo title="Example public identities" description="Example reusable components expose caller-owned document identities for external accessible references, with the same behavior in every production output mode." keyword="example">
<script>import PublicHeading from '../comp/public-heading.brain'</script>
<main><h1>Example public identities</h1><div aria-labelledby="first"><PublicHeading heading="first" /></div><div aria-labelledby="second"><PublicHeading heading="second" /></div></main>`)
      cli(['build', '--site', 'https://example.test', '--json'], app)
      const html = readFileSync(join(app, 'dist/index.html'), 'utf8')
      assert.ok(html.includes('id="first"'), html)
      assert.ok(html.includes('id="second"'), html)
      assert.ok(!html.includes('id="second~2"'), html)
      if (mode === 'static') {
        assert.equal(cli(['doctor', '--json'], app).staticArtifact.ok, true)
        writeFileSync(join(app, 'public/extra.html'), '<h1 id="present">Example</h1><p aria-labelledby="missing">Missing association</p>')
        const report = cli(['doctor', '--json'], app, false)
        assert.ok(report.failed.includes('staticArtifact'), JSON.stringify(report))
        assert.ok(report.staticArtifact.issues.some(issue => issue.file === 'extra.html' && issue.message?.includes('missing document ID reference')), JSON.stringify(report.staticArtifact))
      }
    }
  } finally { rmSync(root, { recursive: true, force: true }) }
})
