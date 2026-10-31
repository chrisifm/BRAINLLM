// Opt-in end-to-end gate. Never connects to an ambient or personal database.
import fs from 'node:fs'
import path from 'node:path'
import os from 'node:os'
import crypto from 'node:crypto'
import { spawn } from 'node:child_process'
import { fileURLToPath } from 'node:url'
const args=process.argv.slice(2), option=(name,fallback)=>{const at=args.indexOf(name);return at<0?fallback:args[at+1]}
const binary=option('--binary'), sourceSHA=option('--source-sha'), image=option('--image','brainllm-next-verify:rust-1.99.1-owned')
if(!binary||!sourceSHA||!(/^[0-9a-f]{7,40}$/.test(sourceSHA))){console.error('Usage: node bench/native-live/run.mjs --binary <owned Linux brain> --source-sha <actual binary source commit> [--scratch <private directory>] [--image <owned image>]');process.exit(2)}
const scratch=fs.mkdtempSync(path.join(option('--scratch',os.tmpdir()),'brain-live-fullchain-'));fs.chmodSync(scratch,0o700)
const id=crypto.randomBytes(6).toString('hex'), network='brain-live-'+id, pg='brain-live-pg-'+id, worker='brain-live-browser-'+id
const output=option('--out',path.join(scratch,'report.json')), source=path.dirname(fileURLToPath(import.meta.url))
let active, timedOut=false
const ensure=(value,code)=>{if(!value)throw Error(code)}
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
  ensure(fs.lstatSync(binary).isFile(),'OWNED_REGULAR_LINUX_BINARY_REQUIRED');const bytes=fs.readFileSync(binary);ensure(bytes.subarray(0,4).equals(Buffer.from([127,69,76,70])),'LINUX_ELF_BINARY_REQUIRED');report.binarySHA256=crypto.createHash('sha256').update(bytes).digest('hex')
  const expected=option('--expect-sha');if(expected)ensure(report.binarySHA256===expected,'BINARY_DIGEST_MISMATCH')
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
  const result=await command(['run','--rm','--name',worker,'--label',`brainllm.native-live=${id}`,'--network',network,'--user','1000:1000','--cpus','2','--memory','3g','--pids-limit','256','--read-only','--tmpfs','/tmp:rw,exec,nosuid,size=1g,mode=1777','--log-opt','compress=false','--log-opt','max-size=128k','--log-opt','max-file=1','--env-file',appEnv,'--mount',`type=bind,src=${path.resolve(binary)},dst=/binary/brain,readonly`,'--mount',`type=bind,src=${source}/fixture,dst=/fixture,readonly`,'--mount',`type=bind,src=${source}/inside.mjs,dst=/inside.mjs,readonly`,'--mount',`type=bind,src=${source}/frame-metadata.mjs,dst=/frame-metadata.mjs,readonly`,'--entrypoint','node',image,'/inside.mjs'],{deadline:195000,allowFailure:true})
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
