// Owned frontend acceptance. No existing app, database, credentials or remote services are used.
import assert from 'node:assert/strict'
import { spawn, spawnSync } from 'node:child_process'
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import net from 'node:net'
const binary=process.env.BRAIN_BIN
assert(binary && existsSync(binary),'Set BRAIN_BIN to the frozen native CLI to test')
const directory=mkdtempSync(join(process.env.TMPDIR||tmpdir(),'brain-route-constraints-'))
const environment={PATH:process.env.PATH,HOME:process.env.HOME,TMPDIR:directory,LANG:'en_US.UTF-8',NO_COLOR:'1',BRAIN_NO_DELEGATE:'1'}
const owned=new Set(), checks=[]
const write=(app,path,source)=>{mkdirSync(dirname(join(app,path)),{recursive:true});writeFileSync(join(app,path),source)}
const pause=ms=>new Promise(resolve=>setTimeout(resolve,ms))
function cli(app,args,ok=true){const result=spawnSync(binary,args,{cwd:app,env:environment,encoding:'utf8',timeout:90000,maxBuffer:4*1024*1024});assert(!result.error&&!result.signal,'CLI command exceeded its bound');if(ok)assert.equal(result.status,0,(result.stdout+result.stderr).slice(-4000));else assert.notEqual(result.status,0,'Invalid source unexpectedly passed');return result.stdout+result.stderr}
async function port(){const socket=net.createServer();await new Promise(resolve=>socket.listen(0,'127.0.0.1',resolve));const p=socket.address().port;await new Promise(resolve=>socket.close(resolve));return p}
function own(command,args,app,extra={}){const child=spawn(command,args,{cwd:app,env:{...environment,...extra},stdio:['ignore','pipe','pipe'],detached:true});child.output='';for(const stream of [child.stdout,child.stderr])stream.on('data',data=>child.output=(child.output+data).slice(-65536));child.on('error',()=>{});owned.add(child);return child}
async function stop(child){if(!owned.delete(child)||child.exitCode!==null||child.signalCode!==null)return;const gone=new Promise(resolve=>child.once('exit',resolve));try{process.kill(-child.pid,'SIGTERM')}catch{}const timer=setTimeout(()=>{try{process.kill(-child.pid,'SIGKILL')}catch{}},3000);await gone;clearTimeout(timer)}
async function ready(child,origin,path='/app/favicon.svg'){const end=Date.now()+30000;while(Date.now()<end){assert(child.exitCode===null&&child.signalCode===null,child.output);try{await(await fetch(origin+path,{signal:AbortSignal.timeout(1000)})).arrayBuffer();return}catch{}await pause(50)}throw Error('Owned listener did not become ready: '+child.output.slice(-1000))}
const get=(origin,path)=>fetch(origin+path,{redirect:'manual',signal:AbortSignal.timeout(5000)})
const routeData=html=>JSON.parse(html.match(/<script type="application\/json" id="brain-route">([^<]*)<\/script>/)[1])
function fixture(name){const app=join(directory,name);write(app,'app.brain','<html lang="en"><head><meta charset="utf-8"><title>Routes</title></head><body><slot /></body></html>');write(app,'tokens.cssbrain',':root { --color-bg: #fff; --color-text: #111; --space-1: 4px; }');write(app,'public/favicon.svg','<svg xmlns="http://www.w3.org/2000/svg"></svg>');write(app,'.brain_env','[development]\nPUBLIC_SITE_URL=http://localhost:5180\n[production]\nPUBLIC_SITE_URL=https://example.com\n');return app}
const config={base:'/app/',i18n:{default:'en',locales:['en','fr']},seo:{minScore:80,og:'off'},cache:{sw:false},routeParams:{'/:code':{code:'[2-9a-z]{7}'}}}
const view='<meta seo title="Record" description="A synthetic record route">\n<script>\nexport async function load(ctx) { if (ctx.params.code.length !== 7) throw Error("ROUTE_LOAD_SENTINEL"); return { code: ctx.params.code } }\nexport async function sitemap() { return [{ code: "3abcdef" }, { code: "zzz" }, { code: "%32abcd" }] }\n</script>\n<main><h1>Record {props.code}</h1></main>'
const deadline=setTimeout(async()=>{console.error('route-constraints: deadline exceeded');await Promise.all([...owned].map(stop));rmSync(directory,{recursive:true,force:true});process.exit(1)},240000)
try {
 const app=fixture('route-fixture');write(app,'i18n/en.json','{}');write(app,'i18n/fr.json','{}');write(app,'views/_loading.brain','<main><h1>Loading record</h1></main>');write(app,'brain.json',JSON.stringify(config));write(app,'views/home.brain','<meta seo title="Home" description="A synthetic home"><main><h1>Home</h1><a href="/3abcdef">Record</a></main>');write(app,'views/[code].brain',view);write(app,'views/4abcdef.brain','<meta seo title="Static" description="A static winner"><main><h1>Static winner</h1></main>');write(app,'redirects.json',JSON.stringify([{from:'/old/:code',params:{code:'[2-9a-z]{7}'},to:'/:code',status:308}]))
 let p=await port(),child=own(binary,['run','--port',String(p),'--host','127.0.0.1','--no-inspect','--no-trace'],app),origin='http://127.0.0.1:'+p
 await ready(child,origin)
 const clean=JSON.parse(cli(app,['check','--json']));assert(!clean.some(d=>d.code==='B1880'))
 for(const path of ['/app/zzz','/app/%32abcdef','/app/%33abcdef','/app/0abcdef'])assert.equal((await get(origin,path)).status,404,'dev '+path)
 assert.equal(routeData(await(await get(origin,'/app/3abcdef')).text()).p.code,'3abcdef');assert.equal(routeData(await(await get(origin,'/app/fr/3abcdef')).text()).p.code,'3abcdef');assert.equal((await get(origin,'/app/fr/zzz')).status,404);assert.equal(routeData(await(await get(origin,'/app/4abcdef')).text()).v,'/views/4abcdef.brain');assert.equal((await get(origin,'/app/favicon.svg')).status,200)
 let response=await get(origin,'/app/old/3abcdef/?q=one');assert.deepEqual([response.status,response.headers.get('location')],[308,'/app/3abcdef?q=one']);for(const path of ['/app/old/%32abcdef','/app/old/%33abcdef'])assert.equal((await get(origin,path)).status,404);assert.equal((await get(origin,'/app/old/zzz')).status,404)
 // Invalid configuration is an actionable response; fixing it reuses the same live watcher/listener.
 write(app,'brain.json',JSON.stringify({...config,routeParams:{'/:code':{missing:'[a-z]{7}'}}}));response=await get(origin,'/app/3abcdef');assert.equal(response.status,500);assert.match(await response.text(),/unknown single-segment parameter/)
 const invalidCheck=JSON.parse(cli(app,['check','--json'],false));assert(invalidCheck.some(d=>d.code==='B1880'&&d.file==='brain.json'))
 const invalidDoctor=JSON.parse(cli(app,['doctor','--json'],false));assert(invalidDoctor.check.diags.some(d=>d.code==='B1880'));assert(invalidDoctor.check.errors>0)
 write(app,'brain.json',JSON.stringify(config));assert.equal(routeData(await(await get(origin,'/app/3abcdef')).text()).p.code,'3abcdef');assert.equal(child.exitCode,null)
 write(app,'brain.json',JSON.stringify({...config,routeParams:{}}));assert.equal(routeData(await(await get(origin,'/app/%33abcdef')).text()).p.code,'3abcdef');response=await get(origin,'/app/zzz');assert.equal(response.status,500);await response.text();write(app,'brain.json',JSON.stringify(config));assert.equal((await get(origin,'/app/zzz')).status,404)
 write(app,'views/[...rest].brain','<meta seo title="Fallback" description="A fallback route"><main><h1>Fallback</h1></main>');assert.equal(routeData(await(await get(origin,'/app/zzz')).text()).v,'/views/[...rest].brain');rmSync(join(app,'views/[...rest].brain'))
 const devSitemap=await(await get(origin,'/app/sitemap.xml')).text();assert.match(devSitemap,/3abcdef/);assert.doesNotMatch(devSitemap,/zzz|%2532abcd/)
 await stop(child);
 const validDoctor=JSON.parse(cli(app,['doctor','--json']));assert.equal(validDoctor.ok,true);assert.equal(validDoctor.check.errors,0)
 write(app,'redirects.json',JSON.stringify([{from:'/old/:code',params:{missing:'a'},to:'/:code'}]));assert(JSON.parse(cli(app,['check','--json'],false)).some(d=>d.code==='B1880'&&d.file==='redirects.json'));write(app,'redirects.json',JSON.stringify([{from:'/old/:code',params:{code:'[2-9a-z]{7}'},to:'/:code',status:308}]))
 checks.push('dev invalid no-load, valid, static/assets, redirects/query/base/slash, live config recovery, sitemap, B1880 check/doctor')
 for(const path of ['_redirects','_brain/nginx.conf','_brain/Caddyfile'])write(app,'dist/'+path,'# stale unsupported host output\n/:code /target 301\n')
 cli(app,['build','--env','production','--strict','--site','https://example.com']);assert(existsSync(join(app,'dist/server.mjs')))
 for(const path of ['_redirects','_brain/nginx.conf','_brain/Caddyfile'])assert(!existsSync(join(app,'dist',path)),'unsupported host file leaked: '+path)
 assert.equal(JSON.parse(readFileSync(join(app,'dist/_server/manifest.json'),'utf8')).rawRouting,true,'constraints must preserve raw transport intent in the runtime manifest')
 const capabilities=JSON.parse(readFileSync(join(app,'dist/_brain/redirect-capabilities.json'),'utf8'));assert.equal(capabilities.unsupported.length,4)
 p=await port();origin='http://127.0.0.1:'+p;child=own(process.execPath,[join(app,'dist/server.mjs')],app,{PORT:String(p),HOST:'127.0.0.1'});await ready(child,origin)
 for(const path of ['/app/zzz','/app/%32abcdef','/app/%33abcdef','/app/0abcdef'])assert.equal((await get(origin,path)).status,404,'SSR '+path)
 assert.match(await(await get(origin,'/app/3abcdef')).text(),/Record 3abcdef/);assert.match(await(await get(origin,'/app/fr/3abcdef')).text(),/Record 3abcdef/);assert.equal((await get(origin,'/app/fr/zzz')).status,404);assert.match(await(await get(origin,'/app/4abcdef')).text(),/Static winner/)
 response=await get(origin,'/app/old/3abcdef?q=one');assert.deepEqual([response.status,response.headers.get('location')],[308,'/app/3abcdef?q=one']);for(const path of ['/app/old/%32abcdef','/app/old/%33abcdef'])assert.equal((await get(origin,path)).status,404)
 const sitemap=await(await get(origin,'/app/sitemap.xml')).text();assert.match(sitemap,/3abcdef/);assert.doesNotMatch(sitemap,/zzz|%2532abcd/)
 await stop(child);checks.push('compiled SSR, raw encodings, sitemap and unsupported convenience exports')
 // A static host cannot enforce an open constrained redirect; failure precedes replacement of existing dist.
 const signature=readFileSync(join(app,'dist/_server/manifest.json'),'utf8');assert.match(cli(app,['build','--static'],false),/static output cannot represent constrained redirects/);assert.equal(readFileSync(join(app,'dist/_server/manifest.json'),'utf8'),signature)
 // Invalid SSG params are rejected during planning, before invoking any page load.
 rmSync(join(app,'redirects.json'));write(app,'views/[code].brain','<meta seo title="Record" description="A generated record">\n<script>\nexport function paths() { return [{ code: "3abcdef" }, { code: "zzz" }] }\nexport async function load() { throw Error("ROUTE_LOAD_SENTINEL") }\n</script>\n<main><h1>Record</h1></main>');const invalid=cli(app,['build','--static'],false);assert.match(invalid,/paths\(\) violates routeParams/);assert.doesNotMatch(invalid,/ROUTE_LOAD_SENTINEL/)
 write(app,'views/[code].brain','<meta seo title="Record" description="A generated record">\n<script>\nexport function paths() { return [{ code: "3abcdef" }] }\nexport async function load(ctx) { return { code: ctx.params.code } }\n</script>\n<main><h1>Record {props.code}</h1></main>');cli(app,['build','--static','--env','production','--site','https://example.com']);assert(!existsSync(join(app,'dist/server.mjs')));assert.match(readFileSync(join(app,'dist/sitemap.xml'),'utf8'),/3abcdef/)
 cli(app,['seo','urls','--write','--json']);const registered=readFileSync(join(app,'seo-urls.txt'),'utf8');assert.match(registered,/3abcdef/);assert.doesNotMatch(registered,/zzz/);checks.push('SSG rejects before load, valid finite pages canonical/sitemap/register, static redirect refusal preserves dist')
 // Opt-in alone must select the server evaluator when every actual page is static.
 const only=fixture('redirect-only');write(only,'brain.json',JSON.stringify({seo:{minScore:80,og:'off'},cache:{sw:false}}));write(only,'views/home.brain','<meta seo title="Home" description="A static home"><main><h1>Home</h1></main>');write(only,'redirects.json',JSON.stringify([{from:'/old/:id',params:{id:'[a-z]{2}'},to:'/'}]));cli(only,['build','--env','production','--site','https://example.com']);assert(existsSync(join(only,'dist/server.mjs')))
 p=await port();origin='http://127.0.0.1:'+p;child=own(process.execPath,[join(only,'dist/server.mjs')],only,{PORT:String(p),HOST:'127.0.0.1'});await ready(child,origin,'/favicon.svg');response=await get(origin,'/old/ab');assert.deepEqual([response.status,response.headers.get('location')],[301,'/']);assert.equal((await get(origin,'/old/abc')).status,404);await stop(child);checks.push('otherwise-static automatic build retains functional server redirects')
 console.log(JSON.stringify({schemaVersion:1,ok:true,checks,processesStopped:true}))
} finally {clearTimeout(deadline);await Promise.all([...owned].map(stop));rmSync(directory,{recursive:true,force:true})}
