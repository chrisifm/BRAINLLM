import { test } from 'node:test'
import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { mkdtempSync, mkdirSync, readFileSync, writeFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { createHash } from 'node:crypto'
import { gunzipSync, brotliDecompressSync } from 'node:zlib'

const binary = process.env.BRAIN_BIN ?? new URL('../../target/debug/brain', import.meta.url).pathname
const landing = 'https://chrisifm.github.io/BRAINLLM/'
const comment = `/* by BRAINLLM ${landing} */`
const signature = `<!-- by BRAINLLM ${landing} -->`
function run(args, cwd) {
  const result = spawnSync(binary, args, { cwd, encoding: 'utf8', timeout: 120000, maxBuffer: 4 * 1024 * 1024 })
  assert.equal(result.status, 0, result.stdout + result.stderr)
  return JSON.parse(result.stdout)
}
for (const mode of ['static', 'ordinary']) test(`explicit page branding reduces ${mode} route bytes without disabling attribution`, { timeout: 240000 }, () => {
  const root = mkdtempSync(join(tmpdir(), 'brain-branding-scope-'))
  try {
    run(['new', 'app', '--static', '--json'], root)
    const app = join(root, 'app'), dist = join(app, 'dist')
    const config = JSON.parse(readFileSync(join(app, 'brain.json'), 'utf8'))
    config.output = mode === 'static' ? 'static' : 'auto'
    config.docs = false
    config.budget = { ...config.budget, viewKB: 256 }
    config.seo = { ...config.seo, minScore: 0, credit: true }
    config.google = { analytics: 'G-EXAMPLE123' }
    const imports = [], widgets = []
    for (let index = 0; index < 40; index++) {
      const name = `Widget${index}`, file = `widget-${index}.brain`
      writeFileSync(join(app, 'comp', file), '<script>let count=sig(0)</script><button on:click={() => count++}>Count {count}</button><style>button{padding:var(--space-2)}</style>')
      imports.push(`import ${name} from '../comp/${file}'`)
      widgets.push(`<${name} />`)
    }
    writeFileSync(join(app, 'views/home.brain'), `<meta seo title="Example counter page" description="A synthetic page with independent interactive components used to measure the actual compressed route graph without changing its visible attribution or behavior." keyword="example"><script>${imports.join('\n')}</script><main><h1>Example counter page</h1>${widgets.join('')}</main>`)
    if (mode === 'ordinary') {
      mkdirSync(join(app, 'views/live'), { recursive: true })
      writeFileSync(join(app, 'views/live/[id].brain'), '<meta seo title="Example live page" description="A synthetic page rendered per request to preserve the ordinary server output while exercising its document attribution and browser asset scope." keyword="example"><script>export async function load(){return {value:"Ready"}}</script><main><h1>Example live page</h1><p>{props.value}</p></main>')
    }
    const authored = `Authored reference ${comment}\n`
    writeFileSync(join(app, 'public/reference.txt'), authored)
    const build = () => {
      writeFileSync(join(app, 'brain.json'), JSON.stringify(config))
      return run(['build', '--site', 'https://example.test', '--json'], app)
    }
    const baseline = build()
    const baselineRoute = baseline.ledger.find(row => row.route === '/')
    assert.ok(baselineRoute.assets.some(asset => asset.kind === 'js' && readFileSync(join(dist, asset.path), 'utf8').includes(comment)), 'per-file comments remain the default')
    config.compilerBrandingScope = 'page'
    const reduced = build(), route = reduced.ledger.find(row => row.route === '/')
    assert.ok(baselineRoute.conservativeJs.gzip - route.conservativeJs.gzip > 1000, 'page scope must recover the repeated signature cost on the same 40-component route')
    assert.ok(route.conservativeJs.brotli < baselineRoute.conservativeJs.brotli)
    for (const asset of route.assets.filter(asset => ['js', 'css'].includes(asset.kind))) {
      const raw = readFileSync(join(dist, asset.path))
      assert.ok(!raw.toString().includes(comment), asset.path)
      for (const [suffix, decode] of [['.gz', gunzipSync], ['.br', brotliDecompressSync]]) {
        try { assert.deepEqual(decode(readFileSync(join(dist, asset.path + suffix))), raw, asset.path + suffix) }
        catch (error) { if (error.code !== 'ENOENT') throw error }
      }
    }
    const html = readFileSync(join(dist, 'index.html'), 'utf8')
    assert.equal(html.split(signature).length - 1, 1)
    assert.ok(html.includes('name="generator"') && html.includes('data-brain-credit'))
    assert.ok(readFileSync(join(dist, '_headers'), 'utf8').includes('X-Generated-By'))
    assert.ok(readFileSync(join(dist, 'robots.txt'), 'utf8').includes(landing))
    assert.ok(readFileSync(join(dist, 'llms.txt'), 'utf8').includes('## Built with'))
    const metadata = JSON.parse(readFileSync(join(dist, 'brain-build.json'), 'utf8'))
    assert.equal(metadata.compilerBranding, true)
    assert.equal(metadata.compilerBrandingScope, 'page')
    assert.equal(metadata.generator.url, landing)
    assert.equal(readFileSync(join(dist, 'reference.txt'), 'utf8'), authored)
    for (const tag of html.match(/<(?:script|link)\b[^>]*>/g) ?? []) {
      const integrity = /integrity="([^"]+)"/.exec(tag)?.[1], path = /(?:src|href)="([^"]+)"/.exec(tag)?.[1]
      if (integrity && path?.startsWith('/_b/')) assert.equal(integrity, 'sha384-' + createHash('sha384').update(readFileSync(join(dist, path.slice(1)))).digest('base64'))
    }
    config.compilerBranding = false
    build()
    assert.equal(JSON.parse(readFileSync(join(dist, 'brain-build.json'), 'utf8')).compilerBranding, false)
    assert.ok(!readFileSync(join(dist, 'index.html'), 'utf8').includes(landing))
    assert.equal(readFileSync(join(dist, 'reference.txt'), 'utf8'), authored)
  } finally { rmSync(root, { recursive: true, force: true }) }
})
