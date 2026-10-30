#!/usr/bin/env node
// Real reference API and native backend, one disposable cluster, explicit source and finite owned children.
import {mkdtempSync,mkdirSync,rmSync,writeFileSync,readFileSync,openSync,closeSync,copyFileSync,chmodSync,readdirSync,lstatSync} from 'node:fs'
import {tmpdir,cpus,totalmem,platform,release} from 'node:os'
import {join,resolve} from 'node:path'
import {fileURLToPath} from 'node:url'
import {spawn,spawnSync} from 'node:child_process'
import {createRequire} from 'node:module'
import {randomBytes,createHash} from 'node:crypto'
import net from 'node:net'
import {deflateSync} from 'node:zlib'
import {measure} from './measure.mjs'
const repository=fileURLToPath(new URL('../../',import.meta.url))
const reference=process.env.BRAIN_REFERENCE_API
const inputBinary=resolve(process.env.BRAIN_BIN||join(repository,'target/debug/brain'))
if(!reference||!process.env.BRAIN_BENCH_OUTPUT)throw Error('Set BRAIN_REFERENCE_API and BRAIN_BENCH_OUTPUT explicitly; no existing database or .env is used')
const profile=process.env.BRAIN_BENCH_PROFILE
if(!['debug','release'].includes(profile))throw Error('Set BRAIN_BENCH_PROFILE=debug|release to identify the actual supplied binary; a profile is not inferred from its filename')
const output=resolve(process.env.BRAIN_BENCH_OUTPUT)
const dir=mkdtempSync(join(process.env.TMPDIR||tmpdir(),'brain-backend-bench-'))
const binary=join(dir,'brain-frozen')
let binarySha256,referenceSource
const children=new Set(),logs=[]
const env={PATH:process.env.PATH,TMPDIR:dir,NODE_ENV:'production',LC_ALL:'C',LANG:'C'}
const password=randomBytes(24).toString('base64url')
function referenceIdentity(){
 const entries=[];let bytes=0
 function visit(relative){
  const path=join(reference,relative),info=lstatSync(path)
  if(info.isSymbolicLink())throw Error('Reference source cannot cross symlinks')
  if(info.isDirectory()){for(const name of readdirSync(path).sort())visit(join(relative,name));return}
  if(!info.isFile()||info.size>8*1024*1024||(bytes+=info.size)>32*1024*1024)throw Error('Reference source identity budget exceeded')
  entries.push([relative,createHash('sha256').update(readFileSync(path)).digest('hex')])
 }
 for(const path of ['src','seeds','migrations','package.json','package-lock.json'])visit(path)
 const revision=spawnSync('git',['rev-parse','--verify','HEAD'],{cwd:reference,encoding:'utf8',timeout:5000,maxBuffer:1024})
 return {commit:revision.status===0?revision.stdout.trim():null,sourceSha256:createHash('sha256').update(JSON.stringify(entries)).digest('hex'),files:entries.length,bytes,scope:'src, seeds, migrations and package manifests; actual checkout, not assumed clean'}
}

const pg=name=>process.env.PG_BIN?join(process.env.PG_BIN,name):name
let postgres,timeout
const sleep=ms=>new Promise(ok=>setTimeout(ok,ms))
const running=child=>child&&child.exitCode===null&&child.signalCode===null
async function stop(child) {
 if(!running(child)){children.delete(child);return}
 const exited=new Promise(ok=>child.once('exit',ok))
 try{process.kill(-child.pid,'SIGTERM')}catch{}
 await Promise.race([exited,sleep(2000)])
 if(running(child)){try{process.kill(-child.pid,'SIGKILL')}catch{};await Promise.race([exited,sleep(3000)])}
 if(running(child))throw Error('Owned benchmark process did not stop')
 children.delete(child)
}
let cleanupPromise
function cleanup(){
 return cleanupPromise ||= (async()=>{
  clearTimeout(timeout);let failure
  for(const child of [...children].reverse())try{await stop(child)}catch(error){failure ||= error}
  for(const fd of logs)closeSync(fd)
  if(![...children].some(running))rmSync(dir,{recursive:true,force:true})
  if(failure)throw failure
 })()
}
for(const signal of ['SIGINT','SIGTERM'])process.once(signal,()=>{void cleanup().finally(()=>process.exit(130))})
function command(executable,args,cwd=dir,extra={}) {
 const result=spawnSync(executable,args,{cwd,env:{...env,...extra},encoding:'utf8',timeout:120000,maxBuffer:256*1024})
 if(result.error||result.status!==0)throw Error('Benchmark command failed: '+args.slice(0,2).join(' ')+'; inspect the owned private log before retrying')
 return result.stdout
}
async function server(executable,args,cwd,extra,readyEvent) {
 const child=spawn(executable,args,{cwd,env:{...env,...extra},stdio:['ignore','pipe','pipe'],detached:true})
 children.add(child)
 let text='',bytes=0,event,failure
 const consume=chunk=>{bytes+=chunk.length;if(bytes>262144){failure=Error('Owned server output exceeded 256 KiB');void stop(child);return}text+=chunk.toString();let end;while((end=text.indexOf('\n'))>=0){const line=text.slice(0,end);text=text.slice(end+1);try{const value=JSON.parse(line);if(value.event===readyEvent)event=value}catch{}}}
 child.stdout.on('data',consume);child.stderr.on('data',consume);child.once('error',()=>{failure=Error('Cannot start owned benchmark server')})
 const deadline=performance.now()+30000
 while(!event){if(failure)throw failure;if(!running(child))throw Error('Owned benchmark server exited before readiness');if(performance.now()>deadline)throw Error('Owned benchmark server readiness timeout');await sleep(50)}
 return {child,...event}
}
async function request(url,{method='GET',body,cookie}={}) {
 const response=await fetch(url,{method,headers:{Origin:'https://benchmark.example.test',Accept:'application/json',...(cookie?{Cookie:cookie}:{}),...(body&&!(body instanceof FormData)?{'Content-Type':'application/json'}:{})},body:body instanceof FormData?body:body===undefined?undefined:JSON.stringify(body),signal:AbortSignal.timeout(10000),redirect:'manual'})
 const parts=[];let size=0
 const reader=response.body?.getReader()
 try{if(reader)for(;;){const {value,done}=await reader.read();if(done)break;size+=value.length;if(size>2*1024*1024)throw Error('Benchmark response too large');parts.push(Buffer.from(value))}}finally{await reader?.cancel().catch(()=>{})}
 if(!response.ok)throw Error('Benchmark rejects failed/limited samples: HTTP '+response.status)
 const data=Buffer.concat(parts)
 return {bytes:size,headers:response.headers,json:()=>JSON.parse(data)}
}
const cookie=result=>{const value=result.headers.getSetCookie().find(v=>/^(?:vd_session|__Host-brain)=/.test(v));if(!value)throw Error('Expected benchmark session cookie');return value.split(';')[0]}
try {
 timeout=setTimeout(()=>{void cleanup().finally(()=>process.exit(124))},10*60*1000)
 const binaryInfo=lstatSync(inputBinary)
 if(!binaryInfo.isFile()||binaryInfo.isSymbolicLink()||binaryInfo.size>128*1024*1024)throw Error('Select a regular native binary within 128 MiB')
 copyFileSync(inputBinary,binary);chmodSync(binary,0o700)
 binarySha256=createHash('sha256').update(readFileSync(binary)).digest('hex')
 referenceSource=referenceIdentity()
 command(pg('initdb'),['-D',join(dir,'pg'),'-A','trust','--no-locale','-U','brain_bench'])
 const reservation=net.createServer();await new Promise(ok=>reservation.listen(0,'127.0.0.1',ok));const port=reservation.address().port;await new Promise(ok=>reservation.close(ok))
 const log=openSync(join(dir,'postgres.log'),'w');logs.push(log)
 postgres=spawn(pg('postgres'),['-D',join(dir,'pg'),'-h','127.0.0.1','-p',String(port),'-k',''],{env,stdio:['ignore',log,log],detached:true});children.add(postgres)
 for(let i=0;;i++){const probe=spawnSync(pg('pg_isready'),['-h','127.0.0.1','-p',String(port),'-U','brain_bench'],{stdio:'ignore',timeout:1000});if(probe.status===0)break;if(i===100||!running(postgres))throw Error('Disposable Postgres readiness failed: '+readFileSync(join(dir,'postgres.log'),'utf8').slice(-2000));await sleep(100)}
 for(const name of ['reference_bench','native_bench'])command(pg('createdb'),['-h','127.0.0.1','-p',String(port),'-U','brain_bench',name])
 const database=name=>`postgres://brain_bench@127.0.0.1:${port}/${name}_bench`
 const refEnv={BRAIN_REFERENCE_API:reference,BRAIN_BENCH_DATABASE:database('reference'),BRAIN_BENCH_PASSWORD:password,BRAIN_BENCH_STORAGE:join(dir,'reference-media')}
 let ref=await server(process.execPath,[fileURLToPath(new URL('reference.mjs',import.meta.url))],dir,refEnv,'ready')
 const rows=ref.rows
 if(!rows.length||rows.length>100)throw Error('Reference seed must supply 1..100 real records')
 const app=join(dir,'native');mkdirSync(app)
 writeFileSync(join(app,'brain.json'),'{}\n');writeFileSync(join(app,'tokens.cssbrain'),':root {}\n')
 mkdirSync(join(app,'server/resources/jobs'),{recursive:true})
 writeFileSync(join(app,'server/resources/jobs/schema.jsbrain'),"export default { title: 'str!', body: 'str!' }\n")
 writeFileSync(join(app,'server/resources/jobs/policy.jsbrain'),"export default { read: ['public'], create: ['admin'], update: ['admin'], delete: ['admin'] }\n")
 writeFileSync(join(app,'server/resources/jobs/search.jsbrain'),"export default ['title', 'body']\n")
 const nativeEnv={DATABASE_URL:database('native'),PUBLIC_SITE_URL:'https://benchmark.example.test'}
 command(binary,['backend','migrate'],app,nativeEnv)
 const startNative=async()=>{
  const owned=await server(binary,['backend','serve','--port','0','--origin','https://benchmark.example.test'],app,nativeEnv,'backend-starting')
  for(let attempt=0;attempt<100;attempt++){
   if(!running(owned.child))throw Error('Owned native backend exited before HTTP readiness')
   try{await request(owned.url+'/health');return owned}catch{}
   await sleep(100)
  }
  throw Error('Owned native backend HTTP readiness timeout')
 }
 let native=await startNative()
 const user=(await request(native.url+'/_brain/auth/signup',{method:'POST',body:{email:'benchmark@example.test',password}})).json().user
 const require=createRequire(join(reference,'package.json'));const {Client}=require('pg');const client=new Client({connectionString:database('native')});await client.connect()
 try{for(const row of rows)await client.query('INSERT INTO _brain_res_jobs(id,owner,data) VALUES(gen_random_uuid(),$1,$2)',[user.id,row])}finally{await client.end()}
 function chunk(type,data){
  const name=Buffer.from(type),payload=Buffer.concat([name,data]);let crc=0xffffffff
  for(const byte of payload){crc^=byte;for(let bit=0;bit<8;bit++)crc=(crc>>>1)^((crc&1)?0xedb88320:0)}
  const size=Buffer.alloc(4),sum=Buffer.alloc(4);size.writeUInt32BE(data.length);sum.writeUInt32BE((crc^0xffffffff)>>>0)
  return Buffer.concat([size,payload,sum])
 }
 const header=Buffer.alloc(13);header.writeUInt32BE(1,0);header.writeUInt32BE(1,4);header[8]=8;header[9]=6
 const parts=[Buffer.from([137,80,78,71,13,10,26,10]),chunk('IHDR',header),chunk('IDAT',deflateSync(Buffer.from([0,255,0,0,255])))],end=chunk('IEND',Buffer.alloc(0))
 const padding=65536-Buffer.concat([...parts,end]).length-12
 const image=Buffer.concat([...parts,chunk('npAd',Buffer.alloc(padding)),end])
 const pdf=Buffer.alloc(65536);pdf.write('%PDF-1.4\n')
 const results=[]
 for(const scenario of ['login','list','upload']) {
  // Fresh limiters per scenario; production limits remain enabled and every status is checked.
  if(scenario!=='login'){await stop(ref.child);await stop(native.child);ref=await server(process.execPath,[fileURLToPath(new URL('reference.mjs',import.meta.url))],dir,refEnv,'ready');native=await startNative()}
  const refCookie=cookie(await request(ref.url+'/api/v1/auth/login',{method:'POST',body:{email:'candidato@vacantes.local',password}}))
  const nativeCookie=cookie(await request(native.url+'/_brain/auth/login',{method:'POST',body:{email:'benchmark@example.test',password}}))
  const operations={
   reference:()=>scenario==='login'?request(ref.url+'/api/v1/auth/login',{method:'POST',body:{email:'candidato@vacantes.local',password}}):scenario==='list'?request(ref.url+'/api/v1/jobs?pageSize=20'):request(ref.url+'/api/v1/me/cv/file',{method:'PUT',cookie:refCookie,body:{name:'benchmark.pdf',dataBase64:pdf.toString('base64')}}),
   native:()=>{if(scenario==='login')return request(native.url+'/_brain/auth/login',{method:'POST',body:{email:'benchmark@example.test',password}});if(scenario==='list')return request(native.url+'/api/jobs?limit=20');const form=new FormData();form.set('file',new Blob([image],{type:'image/png'}),'benchmark.png');return request(native.url+'/_brain/uploads',{method:'POST',cookie:nativeCookie,body:form})}
  }
  for(const implementation of ['reference','native']){console.log(JSON.stringify({event:'measuring',scenario,implementation}));results.push({scenario,implementation,...await measure(async()=>{const result=await operations[implementation]();return result.bytes})})}
 }
 const result={schemaVersion:1,measuredAt:new Date().toISOString(),hardware:{platform:platform(),release:release(),cpu:cpus()[0]?.model,logicalCpus:cpus().length,memoryBytes:totalmem()},source:{nativeSha256:binarySha256,nativeBuildProfile:profile,reference:referenceSource,referenceFastify:createRequire(join(reference,'package.json'))('fastify/package.json').version},dataset:{records:rows.length,projectionSha256:createHash('sha256').update(JSON.stringify(rows)).digest('hex')},methodology:{transport:'actual loopback HTTP, closed-loop concurrency 2, consumed responses, no retries',poolConnections:8,samples:100,warmup:5,limits:'production quotas enabled; any failed or limited response rejects the run',comparison:'Operational scenarios, not equivalent implementations: reference login uses scrypt and profile queries; native uses Argon2id. Reference list returns domain joins/counts, native returns canonical resource projection. Reference PDF upload checks magic and replaces one file; native PNG upload decodes, creates variants and retains originals. No framework-speed winner is inferred.',p99:'Empirical nearest-rank p99 of 100 samples; one local run, no statistical or internet-latency guarantee.'},results}
 writeFileSync(output,JSON.stringify(result,null,2)+'\n');console.log(JSON.stringify({ok:true,scenarios:results.length,output}))
} finally {await cleanup()}
