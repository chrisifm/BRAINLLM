// Opt-in end-to-end gate. Never connects to an ambient or personal database.
import fs from 'node:fs'
import path from 'node:path'
import os from 'node:os'
import crypto from 'node:crypto'
import { spawn } from 'node:child_process'
import { fileURLToPath } from 'node:url'
const args=process.argv.slice(2), option=(name,fallback)=>{const at=args.indexOf(name);return at<0?fallback:args[at+1]}
const binary=option('--binary'), sourceSHA=option('--source-sha'), image=option('--image','brainllm-next-verify:rust-1.99.1-owned'), browser=option('--browser','/usr/bin/chromium')
if(!binary||!sourceSHA||!(/^[0-9a-f]{7,40}$/.test(sourceSHA))){console.error('Usage: node bench/native-live/run.mjs --binary <owned Linux brain> --source-sha <actual binary source commit> [--scratch <private directory>] [--image <owned image>] [--browser <owned container executable>]');process.exit(2)}
const scratch=fs.mkdtempSync(path.join(option('--scratch',os.tmpdir()),'brain-live-fullchain-'));fs.chmodSync(scratch,0o700)
const id=crypto.randomBytes(6).toString('hex'), network='brain-live-'+id, pg='brain-live-pg-'+id, worker='brain-live-browser-'+id
const output=option('--out',path.join(scratch,'report.json')), source=path.dirname(fileURLToPath(import.meta.url))
let active, timedOut=false
const ensure=(value,code)=>{if(!value)throw Error(code)}
const digest=bytes=>crypto.createHash('sha256').update(bytes).digest('hex')
function binaryFingerprint() {
  const fd=fs.openSync(binary,fs.constants.O_RDONLY|(fs.constants.O_NOFOLLOW||0))
  try {
    const stat=fs.fstatSync(fd);ensure(stat.isFile()&&stat.size>=4&&stat.size<=1073741824,'OWNED_BOUNDED_LINUX_BINARY_REQUIRED')
    const header=Buffer.alloc(4);ensure(fs.readSync(fd,header,0,4,0)===4&&header.equals(Buffer.from([127,69,76,70])),'LINUX_ELF_BINARY_REQUIRED')
    const hash=crypto.createHash('sha256'),buffer=Buffer.allocUnsafe(1048576);let offset=0
    while(offset<stat.size){const length=fs.readSync(fd,buffer,0,Math.min(buffer.length,stat.size-offset),offset);ensure(length>0,'BINARY_CHANGED_DURING_READ');hash.update(buffer.subarray(0,length));offset+=length}
    ensure(fs.fstatSync(fd).size===stat.size,'BINARY_CHANGED_DURING_READ');return hash.digest('hex')
  }finally{fs.closeSync(fd)}
}
function fixtureFingerprint() {
  const hashes=[],pending=['fixture'];let bytes=0
  while(pending.length){const relative=pending.pop(),full=path.join(source,relative),stat=fs.lstatSync(full)
    ensure(!stat.isSymbolicLink(),'REGULAR_FIXTURE_SOURCE_REQUIRED')
    if(stat.isDirectory()){for(const name of fs.readdirSync(full).sort().reverse())pending.push(path.join(relative,name));continue}
    ensure(stat.isFile()&&stat.size<=262144&&hashes.length<64,'FIXTURE_SOURCE_BOUND')
    bytes+=stat.size;ensure(bytes<=1048576,'FIXTURE_SOURCE_BOUND');hashes.push([relative.split(path.sep).join('/'),digest(fs.readFileSync(full))])
  }
  hashes.sort(([a],[b])=>a<b?-1:a>b?1:0);return {sha256:digest(JSON.stringify(hashes)),files:hashes.length,bytes}
}
async function command(args,{deadline=30000,allowFailure=false}={}) {
  ensure(!timedOut||allowFailure,'TOTAL_DEADLINE')
  const child=spawn('docker',args,{stdio:['ignore','pipe','pipe']});active=child
  let out='',err='',count=0,limit=false
  let escalation
  const terminate=()=>{if(child.exitCode===null&&child.signalCode===null){child.kill('SIGTERM');if(!escalation)escalation=setTimeout(()=>{if(child.exitCode===null&&child.signalCode===null)child.kill('SIGKILL')},2000)}}
  child.terminateOwned=terminate
  for(const [stream,kind] of [[child.stdout,'out'],[child.stderr,'err']])stream.on('data',chunk=>{count+=chunk.length;if(count>1048576){limit=true;terminate();return}if(kind==='out')out+=chunk;else err+=chunk})
  const timer=setTimeout(terminate,deadline)
  try {const status=await new Promise((resolve,reject)=>{child.once('error',()=>reject(Error('DOCKER_PROCESS_START_FAILED')));child.once('close',(code,signal)=>resolve(signal?124:code))});ensure(!limit,'DOCKER_OUTPUT_BOUND');if(status!==0&&!allowFailure)throw Error('DOCKER_OPERATION_FAILED');return {status,out,err}}
  finally{clearTimeout(timer);clearTimeout(escalation);if(active===child)active=null}
}
let canceled=false
const cancel=()=>{canceled=true;timedOut=true;active?.terminateOwned()}
process.on('SIGINT',cancel);process.on('SIGTERM',cancel)
const total=setTimeout(()=>{timedOut=true;active?.terminateOwned()},240000)
let report={schemaVersion:1,ok:false,sourceSHA,viewer:{used:false,reason:'multiple-connected-profiles-without-explicit-selection'},valuesCaptured:false}
try{
  ensure(fs.lstatSync(binary).isFile(),'OWNED_REGULAR_LINUX_BINARY_REQUIRED');report.binarySHA256=binaryFingerprint()
  ensure(browser.startsWith('/')&&browser.length<=1024&&!/[\x00-\x1f\x7f]/.test(browser),'OWNED_BROWSER_PATH_REQUIRED')
  const expected=option('--expect-sha');if(expected)ensure(report.binarySHA256===expected,'BINARY_DIGEST_MISMATCH')
  report.fixture=fixtureFingerprint();report.gateScripts=Object.fromEntries(['run.mjs','inside.mjs','frame-metadata.mjs'].map(name=>[name,digest(fs.readFileSync(path.join(source,name)))]))
  report.image=(await command(['image','inspect',image,'--format','{{.Id}}'])).out.trim()
  const pgImage=(await command(['image','inspect','postgres:18','--format','{{.Id}}'])).out.trim();report.postgresImage=pgImage
  const database='brain_live_'+id,password=crypto.randomBytes(24).toString('hex')
  const pgEnv=path.join(scratch,'postgres.env'),appEnv=path.join(scratch,'app.env')
  fs.writeFileSync(pgEnv,`POSTGRES_USER=brain\nPOSTGRES_PASSWORD=${password}\nPOSTGRES_DB=${database}\n`,{mode:0o600})
  const databaseURL=new URL(`postgresql://owned-pg:5432/${database}`);databaseURL.username='brain';databaseURL.password=password
  fs.writeFileSync(appEnv,`DATABASE_URL=${databaseURL.href}\n`,{mode:0o600})
  await command(['network','create','--internal','--label',`brainllm.native-live=${id}`,network])
  await command(['run','-d','--name',pg,'--label',`brainllm.native-live=${id}`,'--network',network,'--network-alias','owned-pg','--cpus','1','--memory','256m','--pids-limit','96','--log-opt','compress=false','--log-opt','max-size=128k','--log-opt','max-file=1','--env-file',pgEnv,'postgres:18'])
  let ready=false
  for(let attempt=0;attempt<40;attempt++){const value=await command(['exec',pg,'pg_isready','-h','127.0.0.1','-U','brain','-d',database],{allowFailure:true});if(value.status===0){ready=true;break}await new Promise(resolve=>setTimeout(resolve,150))}ensure(ready,'OWNED_POSTGRES_READY_DEADLINE')
  report.postgresVersion=(await command(['exec',pg,'postgres','--version'])).out.trim()
  const result=await command(['run','--rm','--name',worker,'--label',`brainllm.native-live=${id}`,'--network',network,'--user','1000:1000','--cpus','2','--memory','3g','--pids-limit','256','--read-only','--tmpfs','/tmp:rw,exec,nosuid,size=1g,mode=1777','--log-opt','compress=false','--log-opt','max-size=128k','--log-opt','max-file=1','--env-file',appEnv,'--env',`BRAIN_CHROME=${browser}`,'--mount',`type=bind,src=${path.resolve(binary)},dst=/binary/brain,readonly`,'--mount',`type=bind,src=${source}/fixture,dst=/fixture,readonly`,'--mount',`type=bind,src=${source}/inside.mjs,dst=/inside.mjs,readonly`,'--mount',`type=bind,src=${source}/frame-metadata.mjs,dst=/frame-metadata.mjs,readonly`,'--entrypoint','node',image,'/inside.mjs'],{deadline:195000,allowFailure:true})
  report.workerExit=result.status;report.workerErrorKinds=[...new Set(result.err.match(/\b(?:EACCES|ENOENT|MODULE_NOT_FOUND|ERR_[A-Z_]+|SyntaxError|TypeError|ReferenceError|RangeError)\b/g)||[])];report.workerSourceLines=[...result.err.matchAll(/file:\/\/[^\n:]+:(\d+):(\d+)/g)].map(match=>({line:Number(match[1]),column:Number(match[2])})).slice(0,8)
  const records=(result.out+'\n'+result.err).split('\n').filter(line=>line.startsWith('{')).map(line=>{try{return JSON.parse(line)}catch{return null}}).filter(Boolean)
  const final=records.find(value=>value.schemaVersion===1),cleanup=records.find(value=>value.kind==='cleanup')
  ensure(final,'STRUCTURED_BROWSER_REPORT_REQUIRED');report={...report,...final,binarySHA256:report.binarySHA256,sourceSHA,cleanup}
  ensure(result.status===0&&final.ok,'NATIVE_LIVE_BROWSER_ACCEPTANCE_FAILED')
  ensure(cleanup?.ownedChildrenReaped&&cleanup.browserContextsCreated===cleanup.browserContextsDisposed,'OWNED_CHILDREN_OR_CONTEXTS_NOT_REAPED')
  const connections=await command(['exec',pg,'psql','-U','brain','-d',database,'-Atc','SELECT count(*) FROM pg_stat_activity WHERE datname=current_database() AND pid<>pg_backend_pid()'])
  report.remainingDatabaseConnections=Number(connections.out.trim());ensure(report.remainingDatabaseConnections===0,'OWNED_DATABASE_CONNECTIONS_REMAIN')
}catch(error){report.ok=false;report.gateCode=canceled?'NATIVE_LIVE_GATE_CANCELED':/^[A-Z0-9_]+$/.test(error.message)?error.message:'NATIVE_LIVE_GATE_FAILED';process.exitCode=1}
finally{
  clearTimeout(total)
  // A timed-out create may already have succeeded. Inspect exact ownership
  // labels instead of trusting completion flags or removing an existing namesake.
  for(const [kind,name] of [['container',worker],['container',pg],['network',network]]) {
    const expression=kind==='network'?'{{index .Labels "brainllm.native-live"}}':'{{index .Config.Labels "brainllm.native-live"}}'
    const owned=await command([kind,'inspect',name,'--format',expression],{deadline:10000,allowFailure:true}).catch(()=>null)
    if(owned?.status===0&&owned.out.trim()===id)await command([kind,'rm',...(kind==='container'?['-f']:[]),name],{deadline:10000,allowFailure:true}).catch(()=>{})
  }
  for(const file of ['postgres.env','app.env'])fs.rmSync(path.join(scratch,file),{force:true})
  const containers=await command(['ps','-aq','--filter','name='+id],{allowFailure:true}).catch(()=>({status:1,out:'unknown'}));const networks=await command(['network','ls','--filter','name='+network,'--format','{{.ID}}'],{allowFailure:true}).catch(()=>({status:1,out:'unknown'}));report.ownedDockerRemoved=containers.status===0&&!containers.out.trim()&&networks.status===0&&!networks.out.trim();if(!report.ownedDockerRemoved){report.ok=false;report.cleanupCode='OWNED_DOCKER_CLEANUP_NOT_CONFIRMED';process.exitCode=1};fs.writeFileSync(output,JSON.stringify(report,null,2)+'\n',{mode:0o600});console.log(JSON.stringify(report));process.removeListener('SIGINT',cancel);process.removeListener('SIGTERM',cancel)
}
