// A Pages artifact keeps its index at the root and all browser resources under the configured public prefix.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { createServer } from 'node:http'
import { launch,openPage } from '../../bench/lib/cdp.mjs'
import { mkdtempSync,readFileSync,writeFileSync,readdirSync,existsSync,statSync,rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join,resolve,sep,extname } from 'node:path'
const brain=process.env.BRAIN_BIN??new URL('../../target/debug/brain',import.meta.url).pathname
const call=(args,cwd)=>spawnSync(brain,args,{cwd,encoding:'utf8',timeout:90000,maxBuffer:4*1024*1024})
const ok=(args,cwd)=>{const r=call(args,cwd);assert.equal(r.status,0,r.stdout+r.stderr);return JSON.parse(r.stdout)}
function files(root,prefix=''){return readdirSync(root,{withFileTypes:true}).flatMap(entry=>entry.isDirectory()?files(join(root,entry.name),prefix+entry.name+'/'):[prefix+entry.name])}
test('GitHub Pages exports root and repository sites, validates origins, and previews only a reviewed workflow',{timeout:180000},async()=>{
  const root=mkdtempSync(join(tmpdir(),'brain-github-pages-'))
  let server,chrome,page
  try{
    ok(['new','site','--static','--json'],root)
    const app=join(root,'site'),config=JSON.parse(readFileSync(join(app,'brain.json'),'utf8')),dist=join(app,'dist')
    config.staticOutput={host:'github-pages',js:'js',css:'css',images:'img',inlineCss:false}
    // The doctor gets the same real public origin policy as the build, without a build-only --site override.
    writeFileSync(join(app,'.brain_env'),readFileSync(join(app,'.brain_env'),'utf8').replaceAll('https://example.com','https://example.github.io'))
    writeFileSync(join(app,'views/home.brain'),`<meta seo keyword="static hosting guide" title="Static hosting guide for GitHub Pages" description="A complete static hosting guide with useful rendered content, repository-aware links and browser interactions that run directly from ordinary published files.">
<script>let count=sig(0)</script>
<main class="page"><h1>Static hosting guide</h1><p>Publish complete static pages from the build folder.</p><button id="increment" on:click={() => count++}>Count {count}</button><a href="/about">About</a></main>`)
    for(const base of ['/','/project/']){
      config.base=base;config.compilerBranding=base==='/';writeFileSync(join(app,'brain.json'),JSON.stringify(config))
      ok(['build','--site','https://example.github.io','--json'],app)
      assert.equal(readFileSync(join(dist,'.nojekyll'),'utf8'),'')
      assert.ok(existsSync(join(dist,'index.html'))&&existsSync(join(dist,'about/index.html'))&&existsSync(join(dist,'404.html')))
      assert.equal(existsSync(join(dist,'project')),false,'the URL prefix is not a physical artifact wrapper')
      const names=files(dist)
      assert.ok(!names.some(name=>['_headers','_redirects','server.mjs'].includes(name)||name.startsWith('_server/')||name.startsWith('_brain/')))
      const html=readFileSync(join(dist,'index.html'),'utf8')
      assert.ok(html.includes(base+'about'))
      assert.ok(readFileSync(join(dist,'sitemap.xml'),'utf8').includes('https://example.github.io'+base))
      for(const ref of html.matchAll(/(?:src|href)="([^"#]+)"/g)){
        if(!ref[1].startsWith('/')||ref[1].startsWith('//'))continue
        assert.ok(ref[1].startsWith(base),ref[1])
        const relative=decodeURI(ref[1].slice(base.length).split(/[?#]/)[0])
        assert.ok(existsSync(join(dist,relative))||existsSync(join(dist,relative,'index.html')),ref[1])
      }
      const missing=[]
      server=createServer((req,res)=>{
        const path=new URL(req.url,'http://localhost').pathname
        try{
          if(!path.startsWith(base))throw Error('outside base')
          let file=resolve(dist,decodeURIComponent(path.slice(base.length)))
          if(file!==dist&&!file.startsWith(dist+sep))throw Error('outside artifact')
          if(statSync(file).isDirectory())file=join(file,'index.html')
          res.setHeader('content-type',({'.html':'text/html; charset=utf-8','.js':'text/javascript','.css':'text/css','.svg':'image/svg+xml'})[extname(file)]??'application/octet-stream')
          res.end(readFileSync(file))
        }catch{missing.push(path);res.writeHead(404,{'content-type':'text/html'}).end(readFileSync(join(dist,'404.html')))}
      })
      await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve))
      const origin=`http://127.0.0.1:${server.address().port}`
      chrome??=await launch();page=await openPage(chrome.port,origin+base)
      const until=async(expression)=>{for(let i=0;i<120;i++){try{if(await page.evaluate(expression))return}catch{}await new Promise(resolve=>setTimeout(resolve,50))}assert.fail(expression)}
      await until("document.$h===1&&document.querySelector('#increment')?.textContent==='Count 0'")
      await page.evaluate("document.querySelector('#increment').click()")
      await until("document.querySelector('#increment')?.textContent==='Count 1'")
      await page.evaluate(`document.querySelector('a[href="${base}about"]').click()`)
      await until(`location.pathname==='${base}about'&&document.querySelector('h1')?.textContent==='About site'`)
      assert.deepEqual(missing.filter(path=>!path.endsWith('/favicon.ico')),[],'all hydrated resources load on an ordinary prefix-mounted file host')
      const absent=await fetch(origin+base+'missing-page');assert.equal(absent.status,404);await absent.text()
      page.close();page=null
      await new Promise(resolve=>{server.close(resolve);server.closeAllConnections()});server=null
    }
    // Both authored route and public custom errors take precedence over the generated fallback.
    const authored=readFileSync(join(dist,'404.html'),'utf8')
    assert.ok(authored.length>0)
    rmSync(join(app,'views/404.brain'))
    const custom='<!doctype html><html><head><title>Custom missing page</title><meta name="robots" content="noindex"></head><body><h1>Custom missing page</h1></body></html>'
    writeFileSync(join(app,'public/404.html'),custom)
    ok(['build','--site','https://example.github.io','--json'],app)
    assert.equal(readFileSync(join(dist,'404.html'),'utf8'),custom,'public custom error page is never replaced by the preset')
    rmSync(join(app,'public/404.html'))
    ok(['build','--site','https://example.github.io','--json'],app)
    const fallback=readFileSync(join(dist,'404.html'),'utf8')
    assert.match(fallback,/noindex/);assert.ok(fallback.includes('href="/project/"'))
    assert.ok(!fallback.includes('<!-- by BRAINLLM https://chrisifm.github.io/BRAINLLM/ -->'))
    const doctor=ok(['doctor','--json'],app)
    assert.equal(doctor.staticArtifact.ok,true)
    assert.equal(doctor.build.outputScope,'isolated-doctor-output')
    const before=readFileSync(join(dist,'index.html'),'utf8')
    const credentialSite=new URL('https://example.github.io');credentialSite.username='example';credentialSite.password='example'
    for(const site of ['https://example.github.io/project/',credentialSite.href,'http://example.github.io','https://example.github.io?q=1']){
      const invalid=call(['build','--site',site,'--json'],app);assert.notEqual(invalid.status,0,invalid.stdout);assert.match(invalid.stdout+invalid.stderr,/GitHub Pages|origin|site URL/)
      assert.equal(readFileSync(join(dist,'index.html'),'utf8'),before,'invalid preset configuration preserves the previous artifact')
    }
    const preview=ok(['deploy','config','github-pages','--json'],app)
    assert.equal(preview.dryRun,true);assert.equal(preview.activation.published,false)
    const workflow=preview.operations[0].content
    assert.ok(workflow.includes('path: dist')&&workflow.includes('PAGES_BASE_PATH')&&workflow.includes('needs: build'))
    assert.ok(!workflow.includes('https://chrisifm.github.io/BRAINLLM/'))
    assert.equal(existsSync(join(app,'deploy/github-pages.yml')),false)
    const pm2=ok(['deploy','config','pm2','--json'],app)
    const native=JSON.parse(pm2.operations[0].content.split('module.exports = ')[1].trim().replace(/;$/,''))
    assert.equal(native.apps[0].script,brain)
    assert.equal(native.apps[0].interpreter,'none')
  }finally{page?.close();chrome?.close();if(server)await new Promise(resolve=>{server.close(resolve);server.closeAllConnections()});rmSync(root,{recursive:true,force:true})}
})
