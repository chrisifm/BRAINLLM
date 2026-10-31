#!/usr/bin/env node
// Own one disposable Postgres cluster and run the backend integration tests. Never reads DATABASE_URL.
import { mkdtempSync, mkdirSync, rmSync, openSync, closeSync, readFileSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import { spawn, spawnSync } from 'node:child_process'
import net from 'node:net'
import { createHmac, randomUUID } from 'node:crypto'
import { fileURLToPath } from 'node:url'
const project = fileURLToPath(new URL('../', import.meta.url))
const targets=['analytics','api_keys','backend','cluster','data_queries','domain_writes','edge','exact_types','files','foundation','jwt_sessions','links','media','native_openapi','routes','auth_flows','auth_http','native_sockets','queues']
const selection=process.argv.indexOf('--only-tests')
const onlyCLI=process.argv.includes('--only-cli')
if(onlyCLI&&selection>=0)throw Error('--only-cli and --only-tests cannot be combined')
const selected=selection<0?targets:(process.argv[selection+1]||'').split(',')
if(!selected.length||selected.length>targets.length||new Set(selected).size!==selected.length||selected.some(name=>!targets.includes(name)))throw Error('--only-tests requires a comma-separated list of known backend test targets')
const dir = mkdtempSync(join(process.env.TMPDIR || tmpdir(), 'brain-backend-check-'))
const pg = process.env.PG_BIN || ''
const exe = name => pg ? join(pg,name) : name
let server, test, log, deadline, stopped = false
const running=child=>child && Number.isInteger(child.pid) && child.exitCode===null && child.signalCode===null
const sleep = ms => new Promise(resolve => setTimeout(resolve,ms))
function waitExit(child, ms) {
  if (!running(child)) return Promise.resolve()
  return new Promise(resolve => {
    const finish = () => { clearTimeout(timer); child.off('exit',finish); child.off('error',finish); resolve() }
    const timer = setTimeout(finish,ms)
    child.once('exit',finish); child.once('error',finish)
  })
}
async function terminateOwned(child, signal='SIGTERM') {
  if (!running(child)) return
  try { process.kill(-child.pid,signal) } catch {}
  await waitExit(child,3000)
  if (running(child)) { try { process.kill(-child.pid,'SIGKILL') } catch {}; await waitExit(child,3000) }
  if (running(child)) throw Error('Owned backend child did not exit after termination')
}
async function phase(command,args,env,ms,label) {
  const child=spawn(command,args,{cwd:project,stdio:'inherit',detached:true,env})
  test=child
  let timer, expired=false
  try {
    return await new Promise((ok,no)=>{
      timer=setTimeout(()=>{expired=true;void terminateOwned(child).then(()=>no(Error(label+' exceeded its deadline')),no)},ms)
      deadline=timer
      child.once('error',no); child.once('exit',code=>expired?no(Error(label+' exceeded its deadline')):ok(code??1))
    })
  } finally {
    clearTimeout(timer); if(deadline===timer) deadline=undefined
    await terminateOwned(child)
  }
}
function run(command,args) {
  const r = spawnSync(command,args,{cwd:project,encoding:'utf8',timeout:30000,maxBuffer:256*1024})
  if (r.error || r.status !== 0) throw Error(command + ' failed: ' + (r.error?.message || r.stderr || '').slice(0,2000))
}
async function stop() {
  if (stopped) return
  stopped = true
  clearTimeout(deadline)
  let failure
  try { await terminateOwned(test) } catch(error) { failure=error }
  if (running(server)) {
    spawnSync(exe('pg_ctl'),['-D',join(dir,'data'),'-m','immediate','-w','-t','10','stop'],{stdio:'ignore',timeout:15000})
    await waitExit(server,1000)
    for(const signal of ['SIGINT','SIGKILL']) {
      if(!running(server)) break
      try { server.kill(signal) } catch {}
      await waitExit(server,3000)
    }
    if(running(server)) failure=Error('Owned Postgres did not exit; its private directory was preserved')
  }
  if (log !== undefined) closeSync(log)
  if(!running(server)) rmSync(dir,{recursive:true,force:true})
  if(failure) throw failure
}
process.on('SIGINT',async()=>{await stop();process.exit(130)})
process.on('SIGTERM',async()=>{await stop();process.exit(143)})
try {
  if (process.platform === 'win32') throw Error('This disposable-cluster harness currently needs macOS or Linux')
  run(exe('initdb'),['-D',join(dir,'data'),'-A','trust','--no-locale','-U','brain_test'])
  const reservation = net.createServer()
  await new Promise((ok,no)=>reservation.once('error',no).listen(0,'127.0.0.1',ok))
  const port = reservation.address().port
  await new Promise(ok=>reservation.close(ok))
  log = openSync(join(dir,'postgres.log'),'w')
  server = spawn(exe('postgres'),['-D',join(dir,'data'),'-h','127.0.0.1','-p',String(port),'-k',dir],{stdio:['ignore',log,log]})
  server.once('error',()=>{})
  let ready = false
  for(let i=0;i<100;i++) {
    if(!running(server)) throw Error('Disposable Postgres exited: '+readFileSync(join(dir,'postgres.log'),'utf8').slice(-2000))
    const r=spawnSync(exe('pg_isready'),['-h','127.0.0.1','-p',String(port),'-U','brain_test'],{stdio:'ignore',timeout:1000})
    if(r.status===0){ready=true;break}
    await sleep(100)
  }
  if(!ready) throw Error('Disposable Postgres did not become ready within 10 seconds')
  if(onlyCLI)console.log('Backend integration explicitly skipped: CLI resume only; no Rust test pass is inferred')
  else console.log('Backend integration targets: '+(selection<0?'library + ':'')+selected.join(', ')+(selection<0?'':' (focused; other targets are not rerun)'))
  const code=onlyCLI?0:await phase('cargo',['test','--locked','-p','brain-backend','--jobs','2',...(selection<0?['--lib']:[]),'--test','integration','--','--include-ignored','--nocapture','--test-threads=2',...targets.filter(name=>!selected.includes(name)).flatMap(name=>['--skip',name+'::'])],
    {...process.env,BRAIN_BACKEND_TEST_URL:`postgres://brain_test@127.0.0.1:${port}/postgres`},15*60*1000,'Backend integration')
  process.exitCode=code
  if(code===0) {
    if(!process.env.BRAIN_BIN) {
      // A measured 11m25s cold build and macOS pre-main loader samples justify a finite 20-minute build budget.
      // Integration assertions and their separate deadline stay unchanged.
      const built=await phase('cargo',['build','--locked','-p','brain-cli','--jobs','2'],{...process.env},20*60*1000,'Fresh backend CLI build')
      if(built!==0) throw Error('Fresh backend CLI build failed')
    }
    run(exe('createdb'),['-h','127.0.0.1','-p',String(port),'-U','brain_test','backend_cli'])
    const binary=process.env.BRAIN_BIN || join(project,'target/debug/brain')
    const cliEnv={...process.env,DATABASE_URL:`postgres://brain_test@127.0.0.1:${port}/backend_cli`}
    const brain=(args,cwd)=>{
      const r=spawnSync(binary,args,{cwd,env:cliEnv,encoding:'utf8',timeout:20000,maxBuffer:256*1024})
      if(r.status!==0) throw Error('Backend CLI smoke failed at '+args[0]+' '+(args[1]||'')+' (exit '+r.status+', signal '+r.signal+'): '+(r.stderr||r.stdout||r.error?.message||'No output').slice(0,2000))
      return r.stdout.trim()?JSON.parse(r.stdout):null
    }
    brain(['new','app','--json'],dir)
    const app=join(dir,'app')
    brain(['add','job','deliver','--fields','message:str!','--json'],app)
    writeFileSync(join(app,'server/jobs/deliver/handler.jsbrain'),"<meta server>\nexport default async function handle(ctx) { const digest = await hashText(ctx.payload.message); if (digest.length !== 64 || dateAdd(0, 1, 'day') !== 86400000) throw new Error('Native helpers failed'); return digest }\n")
    brain(['backend','migrate','--json'],app)
    const queued=brain(['jobs','enqueue','deliver','--payload','{"message":"Hello"}','--key','smoke-event','--json'],app)
    assertId(queued.id)
    const duplicate=brain(['jobs','enqueue','deliver','--payload','{"message":"Hello"}','--key','smoke-event','--json'],app)
    if(queued.id!==duplicate.id) throw Error('CLI idempotency key created a duplicate job')
    test=spawn(binary,['jobs','work','deliver','--concurrency','1'],{cwd:app,env:cliEnv,stdio:'ignore',detached:true})
    test.once('error',()=>{})
    try {
      let succeeded=false
      for(let i=0;i<100;i++) {
        if(!running(test)) throw Error('CLI worker exited before handling its job')
        const status=brain(['jobs','status',queued.id,'--json'],app)
        if(status.job?.state==='succeeded'){succeeded=true;break}
        if(status.job?.state==='dead') throw Error('CLI worker exhausted its retries')
        await sleep(100)
      }
      if(!succeeded) throw Error('CLI worker did not finish the synthetic job within its bounded smoke check')
      console.log('Backend CLI queue smoke: PASS (compiled handler, leased delivery, idempotency, acknowledgement)')
    } finally {
      await terminateOwned(test,'SIGINT')
    }
    mkdirSync(join(app,'server/auth'),{recursive:true})
    writeFileSync(join(app,'server/auth/roles.jsbrain'),"export default { roles: ['admin','company','candidate','user','editor'], signup_role: 'user' }\n")
    mkdirSync(join(app,'server/webhooks'),{recursive:true})
    writeFileSync(join(app,'server/api-keys.jsbrain'),"export default { enabled: true, dailyQuota: 100 }\n")
    writeFileSync(join(app,'server/webhooks/config.jsbrain'),"export default { encryptionKeys: ['BRAIN_WEBHOOK_ENCRYPTION_KEY'], inbound: { smoke: { queue: 'deliver', signingKeys: ['BRAIN_WEBHOOK_SMOKE_KEY'] } } }\n")
    brain(['backend','migrate','--json'],app)
    let backendURL='',line='',captured=0
    const authEnv={...cliEnv,BRAIN_WEBHOOK_ENCRYPTION_KEY:'11'.repeat(32),BRAIN_WEBHOOK_SMOKE_KEY:'22'.repeat(32),PUBLIC_SITE_URL:'https://app.example.test/project/app',BRAIN_MAIL_RESEND_KEY:'',BRAIN_MAIL_FROM:'',GOOGLE_CLIENT_ID:'',BRAIN_JWT_SECRET:'synthetic-cli-jwt-signing-key-over-32-bytes',BRAIN_GCS_BUCKET:''}
    test=spawn(binary,['backend','serve','--port','0','--origin','https://app.example.test'],{cwd:app,env:authEnv,stdio:['ignore','pipe','pipe'],detached:true})
    test.once('error',()=>{})
    const capture=(chunk)=>{
      captured+=chunk.length
      if(captured>65536){try{process.kill(-test.pid,'SIGTERM')}catch{};return}
      line+=chunk.toString('utf8')
      let end
      while((end=line.indexOf('\n'))!==-1) {
        const text=line.slice(0,end);line=line.slice(end+1)
        try {const event=JSON.parse(text);if(event.event==='backend-starting'&&/^http:\/\/127\.0\.0\.1:\d+$/.test(event.url)) backendURL=event.url}catch{}
      }
    }
    test.stderr.on('data',capture);test.stdout.on('data',capture)
    try {
      let ready=false
      for(let i=0;i<100;i++) {
        if(!running(test)||captured>65536) throw Error('Owned native auth server failed to start within output limits')
        if(backendURL){try{const response=await fetch(backendURL+'/health',{signal:AbortSignal.timeout(500)});if(response.ok){await response.text();ready=true;break}}catch{}}
        await sleep(100)
      }
      if(!ready) throw Error('Owned native auth server was not ready within its bounded startup check')
      const http=async(path,body,cookie)=>{
        const response=await fetch(backendURL+path,{method:body===undefined?'GET':'POST',headers:{Origin:'https://app.example.test',Accept:'application/json',...(body!==undefined?{'Content-Type':'application/json'}:{}),...(cookie?{Cookie:cookie}:{})},body:body===undefined?undefined:JSON.stringify(body),signal:AbortSignal.timeout(3000),redirect:'manual'})
        const reader=response.body.getReader();let length=0,parts=[]
        try{for(;;){const {done,value}=await reader.read();if(done)break;length+=value.length;if(length>256*1024)throw Error('Auth CLI smoke response exceeds its bound');parts.push(Buffer.from(value))}}finally{await reader.cancel().catch(()=>{})}
        let json=null;try{json=JSON.parse(Buffer.concat(parts).toString('utf8'))}catch{}
        return {status:response.status,json,headers:response.headers}
      }
      const requireStatus=(result,status)=>{if(result.status!==status)throw Error('Native auth CLI smoke returned unexpected HTTP status '+result.status+'; expected '+status);return result}
      const sessionCookie=headers=>{
        const cookies=headers.getSetCookie().filter(value=>!value.includes('Max-Age=0'))
        const cookie=cookies.find(value=>value.startsWith('__Host-brain-access='))||cookies.find(value=>value.startsWith('__Host-brain='))
        if(!cookie)throw Error('Backend-selected native login did not establish a session')
        return cookie.split(';')[0]
      }
      const password='synthetic-cli-admin-account-password'
      const first=requireStatus(await http('/_brain/auth/signup',{email:'operator@example.test',password}),200)
      const second=requireStatus(await http('/_brain/auth/signup',{email:'another@example.test',password}),200)
      if(first.json.user.role!=='user'||second.json.user.role!=='user') throw Error('Configured signup role was not used by the CLI server')
      const before=requireStatus(await http('/_brain/auth/login',{email:'operator@example.test',password}),200)
      if(typeof before.json.access_token!=='string')throw Error('/login did not prefer configured native JWT authority')
      const cookie=sessionCookie(before.headers)
      const promoted=brain(['backend','bootstrap-admin','--user',first.json.user.id,'--json'],app)
      if(promoted.ok!==true||promoted.role!=='admin'||promoted.sessionsRevoked!==true)throw Error('Explicit CLI admin bootstrap did not report its actual mutation')
      requireStatus(await http('/_brain/auth/me',undefined,cookie),401)
      const repeat=spawnSync(binary,['backend','bootstrap-admin','--user',second.json.user.id,'--json'],{cwd:app,env:cliEnv,encoding:'utf8',timeout:20000,maxBuffer:65536})
      if(repeat.status===0||repeat.error)throw Error('A second administrator was bootstrapped or its command failed to complete')
      const signed=requireStatus(await http('/_brain/auth/login',{email:'operator@example.test',password}),200)
      const adminCookie=sessionCookie(signed.headers)
      const api=requireStatus(await http('/_brain/api',undefined,adminCookie),200).json
      const loginSchemas=api.paths['/_brain/auth/login'].post.responses['200'].content['application/json'].schema.oneOf
      if(loginSchemas.length!==2||loginSchemas.some(schema=>!schema.properties.user.properties.role.enum.includes('editor')||!schema.properties.user.properties.role.enum.includes('user')))throw Error('Live OpenAPI omitted configured custom/native roles or a login response mode')
      requireStatus(await http('/_brain/auth/users/'+second.json.user.id+'/role',{role:'editor'},adminCookie),200)
      const updated=requireStatus(await http('/_brain/auth/login',{email:'another@example.test',password}),200)
      if(updated.json.user.role!=='editor')throw Error('Administrator role assignment did not reach the actual account')
      console.log('Backend CLI auth smoke: PASS (configured signup, private first-admin bootstrap, revocation, custom-role HTTP and live OpenAPI)')
      const issued=brain(['backend','keys','create','--user',second.json.user.id,'--name','Smoke key','--scopes','*:read'],app)
      if(!/^bk_live_[0-9a-f]{12}_[0-9a-f]{64}$/.test(issued.secret)||issued.key.prefix!==issued.secret.slice(8,20))throw Error('CLI did not issue a well-formed API key')
      const keyed=async(path)=>{const response=await fetch(backendURL+path,{headers:{'X-API-Key':issued.secret,Accept:'application/json'},signal:AbortSignal.timeout(3000),redirect:'manual'});const json=await response.json().catch(()=>null);return {status:response.status,code:json?.error?.code,headers:response.headers}}
      const refused=await keyed('/_brain/auth/me');if(refused.status!==401||refused.code!=='API_KEY_NOT_ACCEPTED')throw Error('Live server accepted an API key on a session-only route')
      const used=await keyed('/api/absent');if(used.status!==404||used.headers.get('x-quota-limit')!=='100')throw Error('Live server did not authenticate the CLI-issued key with quota headers')
      const listed=brain(['backend','keys','list','--user',second.json.user.id],app)
      if(listed.items.length!==1||JSON.stringify(listed).includes(issued.secret.slice(21)))throw Error('CLI key list was wrong or exposed the secret')
      brain(['backend','keys','revoke',issued.key.prefix],app)
      const revokedKey=await keyed('/api/absent');if(revokedKey.status!==401||revokedKey.code!=='API_KEY_REVOKED')throw Error('Revoked CLI key was still accepted')
      let traced=false
      for(let i=0;i<50&&!traced;i++){traced=brain(['backend','keys','events',issued.key.prefix],app).items.length>=2;if(!traced)await sleep(100)}
      if(!traced)throw Error('CLI key events did not return the live request trace')
      console.log('Backend CLI API-key smoke: PASS (opt-in file, CLI create/list/revoke/events, live X-API-Key auth, quota headers)')
      const webhookID=randomUUID(),stamp=String(Math.floor(Date.now()/1000)),raw=Buffer.from(JSON.stringify({message:'Synthetic private CLI webhook'}))
      const signature='v1='+createHmac('sha256',Buffer.from(authEnv.BRAIN_WEBHOOK_SMOKE_KEY,'hex')).update(`brain-webhook-v1\n${webhookID}\n${stamp}\n`).update(raw).digest('hex')
      const received=await fetch(backendURL+'/_brain/webhooks/in/smoke',{method:'POST',headers:{'Content-Type':'application/json','X-Brain-Webhook-ID':webhookID,'X-Brain-Webhook-Timestamp':stamp,'X-Brain-Webhook-Signature':signature},body:raw,signal:AbortSignal.timeout(3000),redirect:'manual'})
      if(received.status!==202)throw Error('Signed webhook was not accepted by the actual CLI server')
      const receiptBytes=Buffer.from(await received.arrayBuffer());if(receiptBytes.length>4096)throw Error('Webhook receipt exceeded its bound')
      const receipt=JSON.parse(receiptBytes);assertId(receipt.id)
      const webhookWorker=spawn(binary,['jobs','work','deliver','--concurrency','1'],{cwd:app,env:authEnv,stdio:'ignore',detached:true});webhookWorker.once('error',()=>{})
      try {
        let succeeded=false;const end=Date.now()+30000
        while(Date.now()<end) {
          if(!running(webhookWorker))throw Error('Encrypted webhook worker exited before acknowledgement')
          // This identifier comes from the owned backend and is hex/hyphens only; no request body enters SQL.
          const result=spawnSync(exe('psql'),['-h','127.0.0.1','-p',String(port),'-U','brain_test','-d','backend_cli','-At','-c',`SELECT j.state FROM public._brain_jobs j JOIN public._brain_webhook_events e ON e.job_id=j.id WHERE e.id='${receipt.id}'::uuid`],{encoding:'utf8',timeout:2000,maxBuffer:4096})
          if(result.error||result.status!==0)throw Error('Cannot inspect the owned webhook smoke acknowledgement')
          const state=result.stdout.trim();if(state==='succeeded'){succeeded=true;break}if(state==='dead')throw Error('Encrypted webhook handler exhausted its finite attempts')
          await sleep(100)
        }
        if(!succeeded)throw Error('Encrypted webhook CLI handler did not acknowledge within 30 seconds')
        console.log('Backend CLI webhook smoke: PASS (signed HTTP, encrypted durable job, authorized decryption, compiled handler and acknowledgement)')
      } finally {await terminateOwned(webhookWorker,'SIGINT')}

    } finally {await terminateOwned(test,'SIGINT')}
  }
} catch(e) {console.error(e.message);process.exitCode=1}
finally {await stop()}

function assertId(id){if(typeof id!=="string"||!/^[0-9a-f-]{36}$/.test(id))throw Error("CLI enqueue returned no UUID")}
