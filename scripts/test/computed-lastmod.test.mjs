// Evaluated per-page dates must reach discovery output, not only the generated head.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, rmSync } from 'node:fs'
import { join } from 'node:path'
import { tmpdir } from 'node:os'
const brain=process.env.BRAIN_BIN??new URL('../../target/debug/brain',import.meta.url).pathname
test('computed lastmod from build-time load reaches each static and ordinary sitemap entry', {timeout:180000},()=>{
  const root=mkdtempSync(join(tmpdir(),'brain-computed-lastmod-'))
  const cli=(args,cwd)=>{const r=spawnSync(brain,args,{cwd,encoding:'utf8',timeout:60000,maxBuffer:4*1024*1024});assert.equal(r.status,0,r.stdout+r.stderr);return JSON.parse(r.stdout)}
  try {
    for(const mode of ['static','ordinary']) {
      cli(['new',mode,...(mode==='static'?['--static']:[]),'--json'],root)
      const app=join(root,mode)
      mkdirSync(join(app,'views/articles'),{recursive:true})
      writeFileSync(join(app,'views/articles/[slug].brain'),`<meta seo title="Example article dates" description="Read this example article with its explicit content update date, evaluated from loaded page data and published consistently in discovery metadata." keyword="example" schema="Article" lastmod={props.modified}>
<script>
export function paths(){return[{slug:'older'},{slug:'newer'},{slug:'undated'}]}
export async function load(ctx){return{slug:ctx.params.slug,modified:ctx.params.slug==='older'?'2026-01-02':ctx.params.slug==='newer'?'2026-02-03':''}}
</script><main><h1>Example article {props.slug}</h1><p>This example has its own content update date.</p></main>`)
      cli(['build','--site','https://example.test','--json'],app)
      const sitemap=readFileSync(join(app,'dist/sitemap.xml'),'utf8')
      for(const [slug,date] of [['older','2026-01-02'],['newer','2026-02-03']]) {
        assert.ok(sitemap.includes(`<loc>https://example.test/articles/${slug}</loc><lastmod>${date}</lastmod>`),sitemap)
        const html=readFileSync(join(app,`dist/articles/${slug}/index.html`),'utf8')
        assert.ok(html.includes(`"dateModified":"${date}"`),html)
      }
      assert.ok(sitemap.includes('<loc>https://example.test/articles/undated</loc></url>'))
      assert.ok(!sitemap.includes('<lastmod></lastmod>'))
    }
  } finally {rmSync(root,{recursive:true,force:true})}
})
