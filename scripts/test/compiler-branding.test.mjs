// An explicit project policy removes only compiler attribution, before hashes, integrity and compression.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { spawn,spawnSync } from 'node:child_process'
import { mkdtempSync,mkdirSync,writeFileSync,readFileSync,readdirSync,existsSync,rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { createHash } from 'node:crypto'
import { gunzipSync,brotliDecompressSync } from 'node:zlib'
import { createServer } from '../../runtime/server/server.mjs'
import { pathToFileURL } from 'node:url'
import net from 'node:net'
const brain=process.env.BRAIN_BIN??new URL('../../target/debug/brain',import.meta.url).pathname
const home='https://chrisifm.github.io/BRAINLLM/',repo='https://github.com/chrisifm/BRAINLLM'
const cli=(args,cwd)=>spawnSync(brain,args,{cwd,encoding:'utf8',timeout:90000,maxBuffer:4*1024*1024})
const ok=(args,cwd)=>{const out=cli(args,cwd);assert.equal(out.status,0,out.stdout+out.stderr);return JSON.parse(out.stdout)}
function walk(root,at=root,rows=new Map()) {for(const item of readdirSync(at,{withFileTypes:true})){const path=join(at,item.name);if(item.isDirectory())walk(root,path,rows);else rows.set(path.slice(root.length+1).replaceAll('\\','/'),readFileSync(path))}return rows}
for(const mode of ['static','ordinary'])test(`explicit branding policy covers ${mode} output, headers, integrity and transitions`,{timeout:240000},async()=>{
  const root=mkdtempSync(join(tmpdir(),'brain-compiler-branding-'))
  let server
  try{
    ok(['new','site','--static','--json'],root)
    const app=join(root,'site'),dist=join(app,'dist'),cfg=JSON.parse(readFileSync(join(app,'brain.json'),'utf8'))
    cfg.output=mode==='static'?'static':'auto';cfg.base='/app/'
    cfg.seo={...cfg.seo,siteName:'Example public site',credit:true}
    cfg.google={analytics:'G-EXAMPLE123'}
    // Explicit public content and licenses are not compiler attribution and must retain their bytes.
    const license='Authored license reference: '+home+'\n'
    writeFileSync(join(app,'public/license-reference.txt'),license)
    mkdirSync(join(app,'views/interactive'),{recursive:true})
    writeFileSync(join(app,'views/interactive/index.brain'),`<meta seo keyword="example interaction" title="Example browser interaction" description="An example browser interaction with rendered text and a native button, used to verify production resources and their final content integrity."><script>let count=sig(0)</script><main><h1>Example browser interaction</h1><button on:click={() => count++}>Count {count}</button></main><style>main{padding:var(--space-4)}</style>`)
    if(mode==='ordinary'){
      mkdirSync(join(app,'views/live'),{recursive:true})
      writeFileSync(join(app,'views/live/[id].brain'),`<meta seo keyword="example live page" title="Example live page" description="An example live rendered page that keeps its metadata and readable content while testing explicit compiler attribution in ordinary server output."><script>export async function load(){return{value:'Ready'}}</script><main><h1>Example live page</h1><p>{props.value}</p></main>`)
    }
    const build=()=>{writeFileSync(join(app,'brain.json'),JSON.stringify(cfg));return ok(['build','--site','https://example.test','--json'],app)}
    build()
    assert.ok(readFileSync(join(dist,'index.html'),'utf8').includes(home),'default branding remains enabled')
    assert.ok(readFileSync(join(dist,'index.html'),'utf8').includes('Ultrafast frontend and full-stack web compiler written in Rust'),'default generator describes the compiler in English')
    assert.ok(JSON.parse(readFileSync(join(dist,'brain-build.json'),'utf8')).generator.url===home)
    cfg.compilerBranding=false;build()
    const output=walk(dist)
    for(const [path,bytes]of output){
      if(bytes.equals(Buffer.from(license)))continue
      if(/\.(?:gz|br)$/.test(path)){
        const raw=output.get(path.slice(0,-3));assert.ok(raw,path)
        assert.deepEqual(path.endsWith('.gz')?gunzipSync(bytes):brotliDecompressSync(bytes),raw,path)
      }
      if(/\.(?:html|xml|md|txt|json|js|css|mjs|conf)$/.test(path)||['_headers','_redirects','_brain/Caddyfile'].includes(path)){
        const text=bytes.toString();assert.ok(!text.includes(home)&&!text.includes(repo),path+' retained compiler attribution')
        assert.ok(!text.includes('data-brain-credit')&&!text.includes('name="generator"'),path+' retained generator credit')
      }
    }
    assert.equal(readFileSync(join(dist,'license-reference.txt'),'utf8'),license)
    assert.equal(JSON.parse(readFileSync(join(dist,'brain-build.json'),'utf8')).compilerBranding,false)
    const html=readFileSync(join(dist,'interactive/index.html'),'utf8')
    for(const tag of html.match(/<(?:script|link)\b[^>]*>/g)??[]){
      const sum=/integrity="([^"]+)"/.exec(tag)?.[1],url=/(?:src|href)="([^"]+)"/.exec(tag)?.[1]
      if(!sum||!url?.startsWith('/app/'))continue
      assert.equal(sum,'sha384-'+createHash('sha384').update(output.get(decodeURI(url.slice('/app/'.length)))).digest('base64'),tag)
    }
    if(mode==='ordinary'){
      assert.equal(JSON.parse(readFileSync(join(dist,'_server/manifest.json'),'utf8')).compilerBranding,false)
      assert.ok(!readFileSync(join(dist,'server.mjs'),'utf8').includes('// BRAINLLM production server'))
      const syntax=spawnSync(process.execPath,['--check',join(dist,'server.mjs')],{encoding:'utf8',timeout:10000});assert.equal(syntax.status,0,syntax.stderr)
    }
    const factory=mode==='ordinary'?(await import(pathToFileURL(join(dist,'server.mjs')).href)).createServer:createServer
    server=factory({dist,log(){}});await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve))
    const origin=`http://127.0.0.1:${server.address().port}`
    for(const path of mode==='ordinary'?['/app/','/app/missing','/app/live/one']:['/','/missing']){
      const response=await fetch(origin+path)
      assert.equal(response.status,path.endsWith('/missing')?404:200,path)
      assert.equal(response.headers.get('x-generated-by'),null)
      assert.ok(response.headers.get('content-security-policy'))
      assert.equal(response.headers.get('cross-origin-opener-policy'),'same-origin')
      const text=await response.text();assert.ok(!text.includes(home)&&!text.includes(repo),path)
    }
    await new Promise(resolve=>{server.close(resolve);server.closeAllConnections()});server=null
    if(mode==='static'){
      const listener=net.createServer();await new Promise(resolve=>listener.listen(0,'127.0.0.1',resolve));const port=listener.address().port;await new Promise(resolve=>listener.close(resolve))
      const child=spawn(brain,['start','--host','127.0.0.1','--port',String(port),'--instances','1'],{cwd:app,detached:true,stdio:['ignore','pipe','pipe']})
      let diagnostics='';for(const stream of [child.stdout,child.stderr])stream.on('data',chunk=>{if(diagnostics.length<65536)diagnostics+=chunk})
      try{
        let reply
        for(let i=0;i<100;i++){try{reply=await fetch(`http://127.0.0.1:${port}/app/`,{signal:AbortSignal.timeout(500)});if(reply.status===200)break;await reply.text()}catch{}await new Promise(resolve=>setTimeout(resolve,100))}
        assert.equal(reply?.status,200,diagnostics)
        assert.equal(reply.headers.get('x-generated-by'),null)
        assert.ok(reply.headers.get('content-security-policy'))
        assert.ok(!(await reply.text()).includes(home))
      }finally{
        try{process.kill(-child.pid,'SIGTERM')}catch{}
        for(let i=0;i<50&&child.exitCode===null&&child.signalCode===null;i++)await new Promise(resolve=>setTimeout(resolve,100))
        if(child.exitCode===null&&child.signalCode===null){try{process.kill(-child.pid,'SIGKILL')}catch{}await new Promise(resolve=>child.once('exit',resolve))}
      }
    }
    const before=readFileSync(join(dist,'index.html'),'utf8')
    for(const invalid of ['false',0,null]){
      cfg.compilerBranding=invalid;writeFileSync(join(app,'brain.json'),JSON.stringify(cfg))
      const failed=cli(['build','--site','https://example.test','--json'],app);assert.notEqual(failed.status,0);assert.match(failed.stdout+failed.stderr,/compilerBranding/)
      assert.equal(readFileSync(join(dist,'index.html'),'utf8'),before)
    }
    cfg.compilerBranding=true;build()
    assert.ok(readFileSync(join(dist,'index.html'),'utf8').includes(home))
    assert.ok(readFileSync(join(dist,'_headers'),'utf8').includes('X-Generated-By'))
    assert.ok(readFileSync(join(dist,'llms.txt'),'utf8').includes('## Built with'))
    assert.equal(readFileSync(join(dist,'license-reference.txt'),'utf8'),license)
  }finally{if(server)await new Promise(resolve=>{server.close(resolve);server.closeAllConnections()});rmSync(root,{recursive:true,force:true})}
})
