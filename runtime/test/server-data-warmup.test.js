// Opt-in data warmup runs against the instance's own dispatcher before public readiness.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { createServer as tcp } from 'node:net'
import { request, createServer as http } from 'node:http'
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, copyFileSync, rmSync, existsSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { warmupSettings, safePath } from '../server/warmup.mjs'
const runtime = fileURLToPath(new URL('../', import.meta.url))
const pause = ms => new Promise(resolve => setTimeout(resolve, ms))
async function port() {
  const server = tcp()
  await new Promise(resolve => server.listen(0,'127.0.0.1',resolve))
  const value = server.address().port
  await new Promise(resolve => server.close(resolve))
  return value
}
function fixture(publicPort, {warmup, code, paths=['/data'], base='/', cache=60, guards=[], redirects} = {}) {
  const root = mkdtempSync(join(tmpdir(),'brain-data-warmup-')), dist=join(root,'dist'), marks=join(root,'reads.log')
  const write = (path, source) => { const full=join(dist,path); mkdirSync(join(full,'..'),{recursive:true}); writeFileSync(full,source) }
  for (const file of ['server.mjs','llm.mjs','native-sockets.mjs','redis-cache.mjs','warmup.mjs','warmup-watchdog.mjs']) {
    mkdirSync(dist,{recursive:true}); copyFileSync(join(runtime,'server',file),join(dist,file))
  }
  write('_b/brain.js',readFileSync(join(runtime,'brain.js'),'utf8'))
  write('_server/dom.js',readFileSync(join(runtime,'shim/dom.js'),'utf8'))
  write('_b/view.js',`import {tpl} from '/_b/brain.js'\nexport default function View(p){return tpl('<p>read '+p.n+'</p>')()}\n`)
  write('_server/data.mjs',`import {appendFileSync} from 'node:fs'\nlet count=0; export async function load(ctx){ count++; appendFileSync(${JSON.stringify(marks)},JSON.stringify({pid:process.pid,path:ctx.url.pathname,count,auth:!!ctx.req.headers.authorization,cookie:!!ctx.req.headers.cookie})+'\\n'); ${code || 'return {n:count}'} }\n`)
  write('_server/page.html','<!doctype html><html><head><title>Warmup</title></head><body><div id="brain-view"><!--brain:ssr--></div><script type="application/json" id="brain-props"><!--brain:props--></script></body></html>')
  const manifest={v:1,build:'warmup-test',redirects,site:`http://localhost:${publicPort}`,base,core:'/_b/brain.js',rpc:{},routes:paths.map(path=>({path,rx:'^'+path+'/?$',view:'/_b/view.js',layouts:[],guards,middleware:[],server:'_server/data.mjs',page:'_server/page.html',cache})),server:warmup===undefined?{}:{warmup}}
  const save = () => write('_server/manifest.json',JSON.stringify(manifest))
  save(); writeFileSync(join(root,'tokens.cssbrain'),''); writeFileSync(join(root,'brain.json'),'{}')
  return {root,dist,marks,manifest,save,reads:()=>existsSync(marks)?readFileSync(marks,'utf8').trim().split('\n').filter(Boolean).map(JSON.parse):[],close:()=>rmSync(root,{recursive:true,force:true})}
}
function launch(fixture, publicPort, extraEnv={}) {
  const child=spawn(process.execPath,[join(fixture.dist,'server.mjs')],{env:{PATH:process.env.PATH,PORT:String(publicPort),HOST:'127.0.0.1',BRAIN_SECRET:'synthetic-session-key-0123456789abcdef',...extraEnv},stdio:['ignore','pipe','pipe']})
  let out='',err='',done=false,readyResolve,readyReject
  const ready=new Promise((resolve,reject)=>{readyResolve=resolve;readyReject=reject});ready.catch(()=>{})
  const deadline=setTimeout(()=>child.kill('SIGKILL'),6000)
  const exit=new Promise(resolve=>child.once('exit',(code,signal)=>{done=true;clearTimeout(deadline);readyReject(new Error('worker exited before readiness'));resolve({code,signal})}))
  child.stdout.on('data',chunk=>{out+=chunk;if(out.length>65536)child.kill('SIGKILL');if(out.includes('brain:ready '))readyResolve()})
  child.stderr.on('data',chunk=>{err+=chunk;if(err.length>65536)child.kill('SIGKILL')})
  return {child,ready,exit,get output(){return out+err},async stop(){if(done)return;child.kill('SIGTERM');const timer=setTimeout(()=>child.kill('SIGKILL'),1200);await exit;clearTimeout(timer)}}
}
function get(publicPort, host=`localhost:${publicPort}`, path='/data') {
  return new Promise((resolve,reject)=>{
    const req=request({host:'127.0.0.1',port:publicPort,path,agent:false,headers:{host,connection:'close'}},res=>{let text='';res.on('data',chunk=>{text+=chunk;if(text.length>65536){res.destroy();reject(new Error('test response exceeded limit'))}});res.on('end',()=>resolve({status:res.statusCode,text}));res.on('error',reject)})
    req.once('error',reject);req.setTimeout(2000,()=>req.destroy(new Error('test request exceeded deadline')));req.end()
  })
}
const publicRoute={path:'/data',server:'_server/data.mjs',cache:60,guards:[],middleware:[]}
test('warmup is explicit, exact, anonymous, read-only and bounded configuration',()=>{
  assert.equal(warmupSettings(undefined,[],'/'),null)
  const settings=warmupSettings({paths:['/app/en/data']},[{...publicRoute,path:'/en/data'}],'/app/','https://public.localhost')
  assert.equal(settings.timeoutMs,8000);assert.equal(settings.onError,'continue')
  for(const value of [{paths:[]},{paths:Array(9).fill('/data')},{paths:['/data','/data']},{paths:['/data'],timeoutMs:10001},{paths:['/data'],method:'POST'},{paths:['/data'],headers:{cookie:'secret'}},{paths:['/data'],onError:'ignore'}]) assert.throws(()=>warmupSettings(value,[publicRoute],'/','http://localhost'))
  for(const path of ['//elsewhere','/a//b','/a/../data','/a/%2e%2e/data','/a%2fdata','/a%5cdata','/data?token=synthetic','/data#fragment','/a\\data','/data\r\n','/data%00','/data%3fkey']) assert.equal(safePath(path),false,path)
  const credentialed = new URL('https://public.localhost'); credentialed.username='synthetic-user'; credentialed.password='synthetic-placeholder'
  for(const site of [undefined,credentialed.href,'https://public.localhost/path','https://example.com','https://public.localhost\r\n','file:///data']) assert.throws(()=>warmupSettings({paths:['/data']},[publicRoute],'/',site))
  for(const prefix of ['api','_brain','_r','_server','s','_health','health']) assert.throws(()=>warmupSettings({paths:[`/${prefix}/data`]},[{...publicRoute,path:`/${prefix}/data`}],'/','http://localhost'))
  for(const route of [{...publicRoute,cache:0},{...publicRoute,guards:['guard.mjs']},{...publicRoute,middleware:['middleware.mjs']},{...publicRoute,server:null},{...publicRoute,path:'/fr/data'}]) assert.throws(()=>warmupSettings({paths:['/data']},[route],'/','http://localhost'))
})
test('no authored data load runs by default, even while modules are warmed',async()=>{
  const p=await port(),f=fixture(p),worker=launch(f,p)
  try {await worker.ready;assert.equal(f.reads().length,0);assert.equal((await get(p)).status,200);assert.equal(f.reads().length,1)}finally{await worker.stop();f.close()}
})
test('data is warm before readiness and matches only its configured host cache key',async()=>{
  const p=await port(),f=fixture(p,{warmup:{paths:['/data'],onError:'abort'}}),worker=launch(f,p)
  try {
    await worker.ready;assert.equal(f.reads().length,1);assert.equal(f.reads()[0].auth,false);assert.equal(f.reads()[0].cookie,false)
    assert.match((await get(p)).text,/read 1/);assert.equal(f.reads().length,1,'matching Host reuses the warmed cache')
    assert.match((await get(p,`other.localhost:${p}`)).text,/read 2/);assert.equal(f.reads().length,2,'another Host keeps its own cache')
  }finally{await worker.stop();f.close()}
})
test('up to eight paths use at most two concurrent loads and include exact base and locale',async()=>{
  const p=await port(),paths=['/en/a','/en/b','/en/c','/en/d'],code=`const state=globalThis.__warmFixture||=( {active:0,max:0} ); state.active++; state.max=Math.max(state.max,state.active); await new Promise(r=>setTimeout(r,30)); state.active--; return {n:state.max}`
  const f=fixture(p,{paths,base:'/app/',warmup:{paths:paths.map(path=>'/app'+path),onError:'abort'},code}),worker=launch(f,p)
  try{await worker.ready;assert.equal(f.reads().length,4);const response=await get(p,undefined,'/app/en/d');assert.equal(response.status,200);assert.match(response.text,/read 2/);assert.equal(f.reads().length,4)}finally{await worker.stop();f.close()}
})
test('clean optional failure continues, required failure exits, and logs omit thrown values',async()=>{
  const secret='synthetic-private-message-never-log'
  for(const onError of ['continue','abort']){
    const p=await port(),f=fixture(p,{warmup:{paths:['/data'],onError},code:`throw new Error('${secret}')`}),worker=launch(f,p)
    try{if(onError==='continue')await worker.ready;else{const result=await worker.exit;assert.equal(result.code,1);assert.ok(!worker.output.includes('brain:ready '))}assert.ok(!worker.output.includes(secret));assert.equal(f.reads().length,1)}finally{await worker.stop();f.close()}
  }
})
test('a load that ignores cancellation and a CPU loop cannot survive behind readiness',async()=>{
  for(const code of ['await new Promise(()=>{})','while(true){}']){
    const p=await port(),f=fixture(p,{warmup:{paths:['/data'],timeoutMs:400,onError:'continue'},code}),worker=launch(f,p),started=performance.now()
    try{const result=await worker.exit;assert.equal(result.signal,'SIGKILL');assert.ok(performance.now()-started<2500);assert.ok(!worker.output.includes('brain:ready '));assert.match(worker.output,/WARMUP_DEADLINE/)}finally{await worker.stop();f.close()}
  }
})
test('a killed worker warms again after restart and a fatal replacement leaves the old worker serving',async()=>{
  const p=await port(),f=fixture(p,{warmup:{paths:['/data'],onError:'abort'}})
  let worker=launch(f,p),replacement,broken
  try{
    await worker.ready;worker.child.kill('SIGKILL');await worker.exit
    worker=launch(f,p);await worker.ready;assert.equal(f.reads().length,2,'the replacement process warms independently')
    broken=fixture(p,{warmup:{paths:['/data'],onError:'abort'},code:"throw new Error('synthetic warmup failure')"});replacement=launch(broken,p)
    const result=await replacement.exit;assert.equal(result.code,1);assert.ok(!replacement.output.includes('brain:ready '));assert.equal((await get(p)).status,200)
  }finally{await worker.stop();await replacement?.stop();broken?.close();f.close()}
})

test('oversized responses and private/write capabilities cannot produce readiness under abort policy',async()=>{
  for(const code of ["return {n:'x'.repeat(300*1024)}","ctx.session.set({user:'synthetic'}); return {n:1}","ctx.backend.service('notes'); return {n:1}","await ctx.backend.resource('notes').create({title:'synthetic'}); return {n:1}"]){
    const p=await port(),f=fixture(p,{warmup:{paths:['/data'],onError:'abort'},code}),worker=launch(f,p,{BRAIN_BACKEND_URL:'http://127.0.0.1:1',BRAIN_SERVICE_API_KEY:'synthetic-service-key-for-negative-fixture'})
    try{const result=await worker.exit;assert.equal(result.code,1);assert.ok(!worker.output.includes('brain:ready '));assert.equal(f.reads().length,1)}finally{await worker.stop();f.close()}
  }
})
test('an anonymous native GET is canceled and reaped before optional timed-out warmup becomes ready',async()=>{
  let hits=0,active=0,closed=0
  const provider=http((req,res)=>{hits++;active++;assert.equal(req.method,'GET');assert.equal(req.headers.authorization,undefined);assert.equal(req.headers.cookie,undefined);res.once('close',()=>{active--;closed++})})
  await new Promise(resolve=>provider.listen(0,'127.0.0.1',resolve))
  const p=await port(),f=fixture(p,{warmup:{paths:['/data'],timeoutMs:500,onError:'continue'},code:"await ctx.backend.resource('notes').get('00000000-0000-0000-0000-000000000001'); return {n:1}"}),worker=launch(f,p,{BRAIN_BACKEND_URL:`http://127.0.0.1:${provider.address().port}`})
  try{await worker.ready;assert.equal(hits,1);assert.equal(active,0);assert.equal(closed,1);assert.ok(!worker.output.includes('WARMUP_DEADLINE'))}finally{await worker.stop();provider.closeAllConnections();await new Promise(resolve=>provider.close(resolve));f.close()}
})
test('module import CPU work shares the same absolute warmup deadline',async()=>{
  const p=await port(),f=fixture(p,{warmup:{paths:['/data'],timeoutMs:400,onError:'abort'}})
  const path=join(f.dist,'_server/data.mjs');writeFileSync(path,'while(true){}\n'+readFileSync(path,'utf8'))
  const worker=launch(f,p),started=performance.now()
  try{const result=await worker.exit;assert.equal(result.signal,'SIGKILL');assert.ok(performance.now()-started<2500);assert.ok(!worker.output.includes('brain:ready '));assert.equal(f.reads().length,0)}finally{await worker.stop();f.close()}
})

test('warmup follows the exact canonical trailing-slash route without following a redirect',async()=>{
  const p=await port(),f=fixture(p,{warmup:{paths:['/data/'],onError:'abort'},redirects:{slash:'always'}}),worker=launch(f,p)
  try{await worker.ready;assert.equal(f.reads().length,1);assert.equal((await get(p,undefined,'/data/')).status,200);assert.equal(f.reads().length,1)}finally{await worker.stop();f.close()}
})
