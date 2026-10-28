// Separate seven-stack benchmark; never overwrites the original results. See expanded.md.
import { spawn } from 'node:child_process'
import { mkdirSync, cpSync, existsSync, readFileSync, writeFileSync, rmSync, createWriteStream, readdirSync } from 'node:fs'
import { resolve, join, dirname } from 'node:path'
import { fileURLToPath } from 'node:url'
import { createHash } from 'node:crypto'
import { cpus, totalmem, loadavg, platform, release, arch } from 'node:os'
const here=dirname(fileURLToPath(import.meta.url))
const work=resolve(process.env.BENCH_WORK || ''), binary=process.env.BRAIN_BIN
if (!process.env.BENCH_WORK || !binary) throw new Error('Set BENCH_WORK to a new owned scratch directory and BRAIN_BIN to a frozen release binary')
const out=resolve(process.env.BENCH_OUT || join(here,'results',`expanded-${new Date().toISOString().slice(0,10)}`))
const runs=Number(process.env.RUNS || 20), warmups=2
if (!Number.isInteger(runs)||runs<2||runs>100) throw new Error('RUNS must be 2..100')
mkdirSync(work,{recursive:true});mkdirSync(out,{recursive:true})
const node=process.execPath
const env={...process.env,CI:'1',NG_CLI_ANALYTICS:'false',ASTRO_TELEMETRY_DISABLED:'1',npm_config_cache:join(work,'npm-cache'),npm_config_registry:'https://registry.npmjs.org'}
const candidates=[
 {id:'brain',label:'BRAINLLM',command:()=>[join(work,'brain-bin'),'build'],route:'/dashboard'},
 {id:'react',label:'React + Vite',command:()=>[node,'node_modules/vite/bin/vite.js','build','--logLevel','silent'],route:'/'},
 {id:'svelte',label:'Svelte + Vite',command:()=>[node,'node_modules/vite/bin/vite.js','build','--logLevel','silent'],route:'/'},
 {id:'vue',label:'Vue + Vite',command:()=>[node,'node_modules/vite/bin/vite.js','build','--logLevel','silent'],route:'/'},
 {id:'angular',label:'Angular CLI',command:()=>[node,'node_modules/@angular/cli/bin/ng.js','build','--configuration','production'],route:'/',dist:'dist/browser'},
 {id:'astro',label:'Astro + React island',command:()=>[node,'node_modules/astro/bin/astro.mjs','build'],route:'/'},
 {id:'react-webpack',label:'React + webpack',command:()=>[node,'node_modules/webpack/bin/webpack.js','--mode','production','--stats','errors-only'],route:'/'},
]
const selected=process.env.BENCH_ONLY?candidates.filter(c=>process.env.BENCH_ONLY.split(',').includes(c.id)):candidates
const active=new Set()
for(const signal of ['SIGINT','SIGTERM'])process.once(signal,()=>{
 for(const child of active){try{if(process.platform==='win32')child.kill('SIGTERM');else process.kill(-child.pid,'SIGTERM')}catch{}}
 setTimeout(()=>{
  for(const child of active){try{if(process.platform==='win32')child.kill('SIGKILL');else process.kill(-child.pid,'SIGKILL')}catch{}}
  process.exit(signal==='SIGINT'?130:143)
 },1000)
})
async function run(cmd,args,cwd,tag){
 const path=join(work,`${tag}.log`),log=createWriteStream(path);let bytes=0
 return await new Promise((yes,no)=>{
  const start=process.hrtime.bigint(),child=spawn(cmd,args,{cwd,env,detached:process.platform!=='win32',stdio:['ignore','pipe','pipe']})
  active.add(child)
  let timer,killTimer,timed=false
  const kill=s=>{try{if(process.platform==='win32')child.kill(s);else process.kill(-child.pid,s)}catch{}}
  timer=setTimeout(()=>{timed=true;kill('SIGTERM');killTimer=setTimeout(()=>kill('SIGKILL'),1000)},600000)
  for(const stream of [child.stdout,child.stderr])stream.on('data',b=>{const left=4*1024*1024-bytes;if(left>0){log.write(b.subarray(0,left));bytes+=Math.min(left,b.length)}})
  child.on('error',e=>{active.delete(child);clearTimeout(timer);clearTimeout(killTimer);log.end();no(e)})
  child.on('close',(code,signal)=>{active.delete(child);clearTimeout(timer);clearTimeout(killTimer);log.end();if(code||signal||timed)no(new Error(`${tag} failed (${code}/${signal}, timeout=${timed}); inspect owned scratch log`));else yes(Number(process.hrtime.bigint()-start)/1e6)})
 })
}
function fixtureHash(dir){
 const h=createHash('sha256'),names=[]
 function visit(base=''){
  for(const entry of readdirSync(join(dir,base),{withFileTypes:true})){
   if(['node_modules','dist','.brain','.angular','.astro'].includes(entry.name))continue
   const name=join(base,entry.name)
   if(entry.isDirectory())visit(name)
   else if(entry.isFile())names.push(name)
  }
 }
 visit()
 for(const name of names.sort()){h.update(name);h.update('\0');h.update(readFileSync(join(dir,name)));h.update('\0')}
 return h.digest('hex')
}
const sha=path=>createHash('sha256').update(readFileSync(path)).digest('hex')
if(!existsSync(join(work,'brain-bin')))cpSync(resolve(binary),join(work,'brain-bin'))
const metadata={schemaVersion:1,date:new Date().toISOString(),sourceCommit:process.env.BRAIN_SOURCE_COMMIT || 'record separately',binarySha256:sha(join(work,'brain-bin')),node:process.version,platform:platform(),release:release(),arch:arch(),cpu:cpus()[0]?.model,cores:cpus().length,memoryBytes:totalmem(),runs,warmups,loadBefore:loadavg(),commandPolicy:'Direct native executable or node + installed CLI; no npx/npm launcher in timed builds',timeoutMs:600000,parallelBuilds:1}
writeFileSync(join(out,'environment.json'),JSON.stringify(metadata,null,2)+'\n')
for(const c of selected){
 const dir=join(work,c.id)
 if(!existsSync(dir))cpSync(c.id==='brain'?join(here,'app'):join(here,'ref',c.id),dir,{recursive:true,filter:s=>!/(^|\/)(node_modules|dist|\.brain)(\/|$)/.test(s)})
 if(c.id!=='brain'&&!existsSync(join(dir,'node_modules'))){
  console.log(`Installing ${c.id}`)
  await run('npm',[existsSync(join(dir,'package-lock.json'))?'ci':'install','--no-audit','--no-fund','--loglevel=error'],dir,`install-${c.id}`)
  if(!['react','svelte'].includes(c.id))cpSync(join(dir,'package-lock.json'),join(here,'ref',c.id,'package-lock.json'))
 }
 if(c.id!=='brain'){
  const pkg=JSON.parse(readFileSync(join(dir,'package.json'))),versions={}
  for(const name of Object.keys({...pkg.dependencies,...pkg.devDependencies}))versions[name]=JSON.parse(readFileSync(join(dir,'node_modules',name,'package.json'))).version
  writeFileSync(join(out,`versions-${c.id}.json`),JSON.stringify({versions,lockSha256:sha(join(dir,'package-lock.json'))},null,2)+'\n')
 }
 console.log(`Building ${c.id}`)
 const [cmd,...args]=c.command()
 await run(cmd,args,dir,`sanity-${c.id}`)
 if(process.env.BENCH_SETUP_ONLY==='1')continue
 const data={schemaVersion:1,id:c.id,label:c.label,route:c.route,dist:c.dist||'dist',fixtureSha256:fixtureHash(dir),binarySha256:metadata.binarySha256,samplesMs:[],warmups,runs,loadBefore:loadavg()}
 for(let n=-warmups;n<runs;n++){
  rmSync(join(dir,'dist'),{recursive:true,force:true})
  const elapsed=await run(cmd,args,dir,`build-${c.id}-${n<0?'warmup'+(-n):n}`)
  if(n>=0)data.samplesMs.push(elapsed)
 }
 const sorted=[...data.samplesMs].sort((a,b)=>a-b)
 data.medianMs=(sorted[Math.floor((runs-1)/2)]+sorted[Math.floor(runs/2)])/2
 const mean=data.samplesMs.reduce((a,b)=>a+b,0)/runs;data.meanMs=mean;data.stddevMs=Math.sqrt(data.samplesMs.reduce((a,b)=>a+(b-mean)**2,0)/(runs-1));data.p95Ms=sorted[Math.ceil(runs*0.95)-1];
 data.minMs=sorted[0];data.maxMs=sorted.at(-1);data.loadAfter=loadavg()
 writeFileSync(join(out,`timing-${c.id}.json`),JSON.stringify(data,null,2)+'\n')
 console.log(`${c.label}: ${data.medianMs.toFixed(2)} ms`)
}
metadata.loadAfter=loadavg();metadata.finishedAt=new Date().toISOString()
writeFileSync(join(out,'environment.json'),JSON.stringify(metadata,null,2)+'\n')
