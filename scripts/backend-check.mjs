#!/usr/bin/env node
// Own one disposable Postgres cluster and run the backend integration tests. Never reads DATABASE_URL.
import { mkdtempSync, mkdirSync, rmSync, openSync, closeSync, readFileSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import { spawn, spawnSync } from 'node:child_process'
import net from 'node:net'
import { createHmac, randomUUID } from 'node:crypto'
import { fileURLToPath } from 'node:url'
import { hasDoctorAdvice } from './gates/doctor-observation.mjs'
const project = fileURLToPath(new URL('../', import.meta.url))
const targets=['vitals','session_controls','analytics','api_keys','backend','caching','cluster','data_queries','domain_writes','edge','exact_types','files','imports','foundation','jwt_sessions','links','media','native_openapi','production_shape','query_shape','routes','auth_flows','auth_http','native_sockets','organizations','queues','row_policy','security_matrix','system_keys']
// Release gates that drive the real binary over real sockets (scripts/gates/<name>.mjs); selectable like a test target.
const gates=['route-constraints','native-vitals','http-truth-table','failure-logs','captcha-joint','session-boundary','job-diagnostics','custom-route-verifier','database-audit','edge-audit','named-aggregates','redis-cache','computed-backfill']
// A module missing from `targets` could not be selected and would silently run in every focused pass of another target.
const declared=[...readFileSync(join(project,'crates/brain-backend/tests/integration.rs'),'utf8').matchAll(/^mod (\w+);/gm)].map(match=>match[1])
const unlisted=[...declared.filter(name=>!targets.includes(name)),...targets.filter(name=>!declared.includes(name))]
if(unlisted.length)throw Error('scripts/backend-check.mjs targets and tests/integration.rs differ: '+unlisted.join(', '))
const selection=process.argv.indexOf('--only-tests')
const onlyCLI=process.argv.includes('--only-cli')
if(onlyCLI&&selection>=0)throw Error('--only-cli and --only-tests cannot be combined')
const selected=selection<0?[...targets,...gates]:(process.argv[selection+1]||'').split(',')
const known=[...targets,...gates]
if(!selected.length||new Set(selected).size!==selected.length||selected.some(name=>!known.includes(name)))throw Error('--only-tests requires a comma-separated list of known backend test targets: '+known.join(', '))
const selectedTests=selected.filter(name=>targets.includes(name)),selectedGates=selected.filter(name=>gates.includes(name))
const dir = mkdtempSync(join(process.env.TMPDIR || tmpdir(), 'brain-backend-check-'))
const pg = process.env.PG_BIN || ''
const exe = name => pg ? join(pg,name) : name
let server, test, log, deadline, stopped = false
const auxiliary = new Set()
const suspendedOwnedDatabase = new Set()
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
  // Signal handlers exit after stop(), so they cannot rely on a suspended-session fixture's local finally.
  for (const pid of suspendedOwnedDatabase) {try {process.kill(pid,'SIGCONT')} catch {}}
  suspendedOwnedDatabase.clear()
  let failure
  try { await terminateOwned(test) } catch(error) { failure=error }
  for (const child of auxiliary) {
    try { await terminateOwned(child) } catch(error) { failure=error }
  }
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
  const gatesOnly=selection>=0&&!selectedTests.length
  const code=onlyCLI||gatesOnly?0:await phase('cargo',['test','--locked','-p','brain-backend','--jobs','2',...(selection<0?['--lib','--test','session_idle']:[]),'--test','integration','--','--include-ignored','--nocapture','--test-threads=2',
    // This explicitly ignored release-profile measurement is not a database acceptance test. Keep every DB assertion.
    '--skip','media::avif_cost::encode_time_by_size',...targets.filter(name=>!selectedTests.includes(name)).flatMap(name=>['--skip',name+'::'])],
    {...process.env,BRAIN_BACKEND_TEST_URL:`postgres://brain_test@127.0.0.1:${port}/postgres`},15*60*1000,'Backend integration')
  process.exitCode=code
  if(code===0&&!process.env.BRAIN_BIN&&(!gatesOnly||selectedGates.length)) {
      // A measured 11m25s cold build and macOS pre-main loader samples justify a finite 20-minute build budget.
      // Integration assertions and their separate deadline stay unchanged.
      const built=await phase('cargo',['build','--locked','-p','brain-cli','--jobs','2'],{...process.env},20*60*1000,'Fresh backend CLI build')
      if(built!==0) throw Error('Fresh backend CLI build failed')
  }
  if(code===0&&!gatesOnly) {
    run(exe('createdb'),['-h','127.0.0.1','-p',String(port),'-U','brain_test','backend_cli'])
    const binary=process.env.BRAIN_BIN || join(process.env.CARGO_TARGET_DIR||join(project,'target'),'debug/brain')
    const cliEnv={...process.env,DATABASE_URL:`postgres://brain_test@127.0.0.1:${port}/backend_cli`}
    const brain=(args,cwd)=>{
      const r=spawnSync(binary,args,{cwd,env:cliEnv,encoding:'utf8',timeout:20000,maxBuffer:256*1024})
      if(r.status!==0) throw Error('Backend CLI smoke failed at '+args[0]+' '+(args[1]||'')+' (exit '+r.status+', signal '+r.signal+'): '+(r.stderr||r.stdout||r.error?.message||'No output').slice(0,2000))
      return r.stdout.trim()?JSON.parse(r.stdout):null
    }
    // Accounts stored under the implicit roles of 0.6.1 and earlier survive every roles change; nothing moves them unasked.
    // check-general: begin 0.6.1 upgrade fixture
    run(exe('createdb'),['-h','127.0.0.1','-p',String(port),'-U','brain_test','backend_roles'])
    {
      const rolesEnv={...cliEnv,DATABASE_URL:`postgres://brain_test@127.0.0.1:${port}/backend_roles`}
      const cli=(args,cwd)=>{const r=spawnSync(binary,args,{cwd,env:rolesEnv,encoding:'utf8',timeout:20000,maxBuffer:8*1024*1024});if(r.status!==0)throw Error('Roles upgrade smoke failed at '+args.join(' ')+': '+(r.stderr||r.stdout).slice(0,1000));return JSON.parse(r.stdout)}
      const sql=(text)=>{const r=spawnSync(exe('psql'),['-h','127.0.0.1','-p',String(port),'-U','brain_test','-d','backend_roles','-At','-v','ON_ERROR_STOP=1','-c',text],{encoding:'utf8'});if(r.status!==0)throw Error('Roles upgrade smoke SQL failed: '+r.stderr.slice(0,500));return r.stdout.trim()}
      spawnSync(binary,['new','roles-app','--json'],{cwd:dir,env:rolesEnv,encoding:'utf8',timeout:20000})
      const legacyApp=join(dir,'roles-app'),write=(path,text)=>{mkdirSync(join(legacyApp,path,'..'),{recursive:true});writeFileSync(join(legacyApp,path),text)}
      write('server/resources/items/schema.jsbrain',"<meta server>\nexport default { title: 'str!' }\n")
      write('server/resources/items/policy.jsbrain',"<meta server>\nexport default { read: ['owner', 'company'], create: ['candidate'], update: ['owner'], delete: ['owner'] }\n")
      cli(['backend','migrate','--json'],legacyApp)
      sql("INSERT INTO _brain_users(id,email,password_hash,role) SELECT gen_random_uuid(),'old-'||n||'@example.test','preserve-exact','candidate' FROM generate_series(1,3) n")
      if(cli(['backend','migrate','--json'],legacyApp).undeclaredRoles) throw Error('Implicit roles must stay declared for an app that names them')
      const schema=cli(['backend','schema'],legacyApp)
      if(schema.roles.signup_role!=='candidate') throw Error('An app naming the implicit roles must keep its signup role')
      write('server/auth/roles.jsbrain',"<meta server>\nexport default { roles: ['admin', 'company', 'candidate', 'user'], signup_role: 'user' }\n")
      if(cli(['backend','migrate','--json'],legacyApp).undeclaredRoles) throw Error('Names declared only to compile must keep compiling')
      write('server/resources/items/policy.jsbrain',"<meta server>\nexport default { read: ['owner'], create: ['member'], update: ['owner'], delete: ['owner'] }\n")
      write('server/auth/roles.jsbrain',"<meta server>\nexport default { roles: ['admin', 'member'], signup_role: 'member' }\n")
      const report=cli(['backend','migrate','--json'],legacyApp)
      if(report.undeclaredRoles?.[0]?.role!=='candidate'||report.undeclaredRoles[0].accounts!==3||!report.next) throw Error('Migrate must report accounts whose role is no longer declared')
      if(sql("SELECT count(*) FROM _brain_users WHERE role='candidate' AND password_hash='preserve-exact'")!=='3') throw Error('Migrate must not rewrite accounts')
      const preview=cli(['backend','rename-role','--from','candidate','--to','member'],legacyApp)
      if(preview.accounts!==3||preview.applied||sql("SELECT count(*) FROM _brain_users WHERE role='candidate'")!=='3') throw Error('rename-role must only count without --apply')
      if(cli(['backend','rename-role','--from','candidate','--to','member','--apply'],legacyApp).accounts!==3) throw Error('rename-role must move the accounts')
      if(sql("SELECT count(*) FROM _brain_users WHERE role='member' AND password_hash='preserve-exact' AND auth_version>0")!=='3') throw Error('rename-role must keep accounts and bump their auth version')
      if(cli(['backend','migrate','--json'],legacyApp).undeclaredRoles) throw Error('No undeclared accounts may remain after the move')
      console.log('Backend CLI roles upgrade smoke: PASS (implicit roles kept, omitted names compile, accounts preserved, explicit move)')
    }
    // check-general: end 0.6.1 upgrade fixture
    // Each default repair smoke owns a separate database and app; no queue/auth fixture state is changed.
    run(exe('createdb'),['-h','127.0.0.1','-p',String(port),'-U','brain_test','backend_defaults'])
    {
      const repairEnv={...cliEnv,DATABASE_URL:`postgres://brain_test@127.0.0.1:${port}/backend_defaults`}
      const cli=(args,cwd)=>{
        const result=spawnSync(binary,args,{cwd,env:repairEnv,encoding:'utf8',timeout:20000,maxBuffer:1024*1024})
        if(result.status!==0)throw Error('Defaults CLI smoke failed at '+args[0]+': '+(result.stderr||result.stdout).slice(0,1000))
        return JSON.parse(result.stdout)
      }
      const sql=text=>{
        const result=spawnSync(exe('psql'),['-X','-h','127.0.0.1','-p',String(port),'-U','brain_test','-d','backend_defaults','-At','-v','ON_ERROR_STOP=1','-c',text],{encoding:'utf8',timeout:10000,maxBuffer:256*1024})
        if(result.status!==0)throw Error('Defaults CLI smoke SQL failed: '+(result.stderr||'').slice(0,1000))
        return result.stdout.trim()
      }
      cli(['new','defaults-app','--json'],dir)
      const repairApp=join(dir,'defaults-app'),resource=join(repairApp,'server/resources/items')
      mkdirSync(resource,{recursive:true})
      writeFileSync(join(resource,'schema.jsbrain'),"<meta server>\nexport default { title: 'str!', label: { type: 'str', default: 'Default' } }\n")
      writeFileSync(join(resource,'policy.jsbrain'),"<meta server>\nexport default { read: ['admin'], create: ['admin'], update: ['admin'], delete: ['admin'] }\n")
      cli(['backend','migrate','--json'],repairApp)
      sql("INSERT INTO _brain_users(id,email,password_hash,role) VALUES(gen_random_uuid(),'defaults-owner@example.test','synthetic-unused-hash','admin')")
      sql(`INSERT INTO _brain_res_items(id,owner,data) SELECT gen_random_uuid(),u.id,v.data FROM _brain_users u CROSS JOIN (VALUES ('{"title":"Old"}'::jsonb),('{"title":"Cleared","label":null}'::jsonb),('{"title":"Custom","label":"Custom"}'::jsonb)) v(data) WHERE u.email='defaults-owner@example.test'`)
      const preview=cli(['data','defaults','--resource','items','--json'],repairApp)
      if(preview.applied||preview.fieldWrites!==0||preview.resources[0].fields[0].absentRows!==1||sql("SELECT count(*) FROM _brain_res_items WHERE NOT(data ? 'label')")!=='1')throw Error('Default preview changed rows or omitted counts')
      const applied=cli(['data','defaults','--resource','items','--limit','1','--apply','--json'],repairApp)
      if(applied.fieldWrites!==1||applied.hasMore||sql("SELECT count(*) FROM _brain_res_items WHERE data->'label'='null'::jsonb")!=='1')throw Error('Default apply must preserve explicit nulls')
      if(cli(['data','defaults','--resource','items','--include-null','--apply','--json'],repairApp).fieldWrites!==1)throw Error('Explicit null repair must fill only the remaining null')
      if(cli(['data','defaults','--resource','items','--include-null','--apply','--json'],repairApp).fieldWrites!==0||sql("SELECT count(*) FROM _brain_res_items WHERE data->>'label'='Custom'")!=='1')throw Error('Default repair must preserve custom values and become idempotent')
      console.log('Backend CLI defaults smoke: PASS (preview, bounded apply, explicit-null opt-in, preserved values and idempotence)')
    }
    // Reconstruct only the prior profile schema in a separate owned database; never change published SQL bytes.
    run(exe('createdb'),['-h','127.0.0.1','-p',String(port),'-U','brain_test','backend_framework_boundary'])
    {
      const migrationEnv={...cliEnv,DATABASE_URL:`postgres://brain_test@127.0.0.1:${port}/backend_framework_boundary`}
      let lastStderr=''
      const cli=(args,cwd)=>{
        const result=spawnSync(binary,args,{cwd,env:migrationEnv,encoding:'utf8',timeout:20000,maxBuffer:1024*1024})
        if(result.status!==0)throw Error('Migration boundary CLI failed: '+(result.stderr||result.stdout).slice(0,1000))
        lastStderr=result.stderr
        return JSON.parse(result.stdout)
      }
      const sql=text=>{
        const result=spawnSync(exe('psql'),['-X','-h','127.0.0.1','-p',String(port),'-U','brain_test','-d','backend_framework_boundary','-At','-v','ON_ERROR_STOP=1','-c',text],{encoding:'utf8',timeout:10000,maxBuffer:256*1024})
        if(result.status!==0)throw Error('Migration boundary owned SQL failed')
        return result.stdout.trim()
      }
      cli(['new','migration-boundary','--json'],dir)
      const app=join(dir,'migration-boundary')
      cli(['add','resource','sample','--fields','title:str!','--json'],app)
      cli(['backend','migrate','--json'],app)
      const previous=()=>sql('ALTER TABLE public._brain_users DROP COLUMN display_name, DROP COLUMN picture_url; ALTER TABLE public._brain_auth_challenges DROP CONSTRAINT _brain_auth_challenges_audience_check, DROP COLUMN audience; DROP TABLE public._brain_vitals_daily, public._brain_vitals_capacity, public._brain_data_backfills, public._brain_backfill_identity; DELETE FROM public._brain_migrations WHERE version IN (27,28,29,30)')
      const check=(report,applied,after)=>{
        const state=report.frameworkMigration
        if(state?.phase!=='before-apply'||state.installed!==26||JSON.stringify(state.pending?.map(item=>item.version))!==JSON.stringify([27,28,29,30])||state.applied!==applied||state.installedAfter!==after||state.pendingAfter?.length!==(applied?0:4))throw Error('Framework migration result must distinguish its plan and observed outcome')
      }
      previous()
      check(cli(['backend','approve-contract','--json'],app),false,26)
      if(sql('SELECT max(version) FROM public._brain_migrations')!=='26')throw Error('Read-only framework preview applied SQL')
      const applied=cli(['backend','approve-contract','--apply','--json'],app)
      check(applied,true,30)
      if(applied.applied!==false||!lastStderr.includes('MIGRATION_ROLLBACK_BOUNDARY'))throw Error('Framework-only apply needs a separate outcome and advance warning')
      const repeated=cli(['backend','approve-contract','--apply','--json'],app).frameworkMigration
      if(repeated.applied!==false||repeated.installed!==30||repeated.installedAfter!==30||repeated.pendingAfter.length!==0)throw Error('Repeated framework apply must report no advancement')
      previous()
      writeFileSync(join(app,'server/resources/sample/schema.jsbrain'),"<meta server>\nexport default { title: 'str!', note: 'str' }\n")
      const refused=spawnSync(binary,['backend','migrate','--json'],{cwd:app,env:migrationEnv,encoding:'utf8',timeout:30000,maxBuffer:65536})
      if(refused.status===0||!((refused.stdout||'')+(refused.stderr||'')).includes('even optional additions require explicit approval'))throw Error('Optional contract additions must explain their explicit approval boundary')
      check(cli(['backend','approve-contract','--only','sample','--json'],app),false,26)
      check(cli(['backend','approve-contract','--only','sample','--apply','--json'],app),true,30)
      console.log('Backend CLI framework migration smoke: PASS (read-only preview, framework-only apply, explicit outcome, idempotence and selected-resource apply)')
    }
    brain(['new','app','--json'],dir)
    const app=join(dir,'app')
    brain(['add','job','deliver','--fields','message:str!','--json'],app)
    writeFileSync(join(app,'server/jobs/deliver/handler.jsbrain'),"<meta server>\nexport default async function handle(ctx) { const digest = await hashText(ctx.payload.message); if (digest.length !== 64 || dateAdd(0, 1, 'day') !== 86400000) throw new Error('Native helpers failed'); return digest }\n")
    const migration = brain(['backend','migrate','--json'],app)
    if (!Number.isInteger(migration.frameworkMigration?.target) || migration.frameworkMigration.rollbackAfterApply?.minimumKnownMigration !== migration.frameworkMigration.target || migration.frameworkMigration.rollbackAfterApply.schemaDowngradeSupported !== false) throw Error('Migrate must report the framework rollback boundary')
    const migrationPreview = brain(['backend','approve-contract','--json'],app)
    if (migrationPreview.applied || migrationPreview.frameworkMigration?.installed !== migration.frameworkMigration.target || migrationPreview.frameworkMigration?.pending?.length !== 0) throw Error('Contract preview must expose the current framework migration boundary without applying changes')
    const queued=brain(['jobs','enqueue','deliver','--payload','{"message":"Hello"}','--key','smoke-event','--json'],app)
    assertId(queued.id)
    const duplicate=brain(['jobs','enqueue','deliver','--payload','{"message":"Hello"}','--key','smoke-event','--json'],app)
    if(queued.id!==duplicate.id) throw Error('CLI idempotency key created a duplicate job')
    // Queue liveness: the waiting job is visible with no queue named, and doctor says nothing is running it, until a worker is.
    const queueRow=()=>brain(['jobs','stats'],app).queues.find(row=>row.queue==='deliver')
    const doctorSays=code=>{
      const started=Date.now()
      // Doctor includes browser tests and a production build; use the existing registry-smoke doctor budget.
      const result=spawnSync(binary,['doctor','--json'],{cwd:app,env:cliEnv,encoding:'utf8',timeout:240000,maxBuffer:8*1024*1024})
      console.log(JSON.stringify({event:'backend-doctor-probe',elapsedMs:Date.now()-started,status:result.status,errorCode:result.error?.code??null,signal:result.signal??null,stdoutBytes:Buffer.byteLength(result.stdout||'')}))
      return hasDoctorAdvice(result,code)
    }
    const waiting=queueRow()
    if(waiting?.pending!==1||waiting.workers!==0||waiting.declared!==true||typeof waiting.oldestWaitSeconds!=='number') throw Error('`brain jobs stats` did not report the waiting queue: '+JSON.stringify(waiting))
    if(!doctorSays('B0765')) throw Error('`brain doctor` did not warn B0765 for a queue with no worker')
    const unnamed=spawnSync(binary,['jobs','work'],{cwd:app,env:cliEnv,encoding:'utf8',timeout:20000})
    if(unnamed.status===0||!unnamed.stderr.includes('declared queues: deliver')) throw Error('`brain jobs work` without a queue must list the declared queues')
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
      if(queueRow()?.workers!==1) throw Error('`brain jobs stats` did not count the running worker')
      if(doctorSays('B0765')) throw Error('`brain doctor` still warned B0765 with a worker running')
      console.log('Backend CLI queue smoke: PASS (compiled handler, leased delivery, idempotency, acknowledgement)')
    } finally {
      await terminateOwned(test,'SIGINT')
    }
    mkdirSync(join(app,'server/auth'),{recursive:true})
    writeFileSync(join(app,'server/auth/roles.jsbrain'),"export default { roles: ['admin','user','editor'], signup_role: 'user' }\n")
    mkdirSync(join(app,'server/webhooks'),{recursive:true})
    writeFileSync(join(app,'server/api-keys.jsbrain'),"export default { enabled: true, dailyQuota: 100 }\n")
    writeFileSync(join(app,'server/webhooks/config.jsbrain'),"export default { encryptionKeys: ['BRAIN_WEBHOOK_ENCRYPTION_KEY'], inbound: { smoke: { queue: 'deliver', signingKeys: ['BRAIN_WEBHOOK_SMOKE_KEY'] } } }\n")
    brain(['backend','migrate','--json'],app)
    let backendURL='',googleSetup,line='',captured=0
    const authEnv={...cliEnv,BRAIN_WEBHOOK_ENCRYPTION_KEY:'11'.repeat(32),BRAIN_WEBHOOK_SMOKE_KEY:'22'.repeat(32),PUBLIC_SITE_URL:'https://app.example.test/project/app',BRAIN_MAIL_RESEND_KEY:'',BRAIN_MAIL_FROM:'',GOOGLE_CLIENT_ID:'synthetic-client-id-0001.apps.googleusercontent.com',BRAIN_JWT_SECRET:'synthetic-cli-jwt-signing-key-over-32-bytes',BRAIN_GCS_BUCKET:''}
    test=spawn(binary,['backend','serve','--port','0','--origin','https://app.example.test'],{cwd:app,env:authEnv,stdio:['ignore','pipe','pipe'],detached:true})
    test.once('error',()=>{})
    const capture=(chunk)=>{
      captured+=chunk.length
      if(captured>65536){try{process.kill(-test.pid,'SIGTERM')}catch{};return}
      line+=chunk.toString('utf8')
      let end
      while((end=line.indexOf('\n'))!==-1) {
        const text=line.slice(0,end);line=line.slice(end+1)
        try {const event=JSON.parse(text);if(event.event==='backend-starting'&&/^http:\/\/127\.0\.0\.1:\d+$/.test(event.url)) backendURL=event.url;if(event.event==='google-origin-setup') googleSetup=event.setup}catch{}
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
      if(!googleSetup || JSON.stringify(googleSetup.authorizedJavaScriptOrigins)!==JSON.stringify(['https://app.example.test']) || googleSetup.providerVerified!==false) throw Error('Google origin startup hints must use the selected browser origin without claiming provider verification')
      const http=async(path,body,cookie)=>{
        const response=await fetch(backendURL+path,{method:body===undefined?'GET':'POST',headers:{Origin:'https://app.example.test',Accept:'application/json',...(body!==undefined?{'Content-Type':'application/json'}:{}),...(cookie?{Cookie:cookie}:{})},body:body===undefined?undefined:JSON.stringify(body),signal:AbortSignal.timeout(3000),redirect:'manual'})
        const reader=response.body.getReader();let length=0,parts=[]
        try{for(;;){const {done,value}=await reader.read();if(done)break;length+=value.length;if(length>512*1024)throw Error('Auth CLI smoke response exceeds its bound');parts.push(Buffer.from(value))}}finally{await reader.cancel().catch(()=>{})}
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

      // A successful idle shutdown must exit itself, rather than merely passing cleanup after the harness used KILL.
      process.kill(-test.pid, 'SIGTERM')
      await waitExit(test, 5000)
      if (running(test) || test.exitCode !== 0) throw Error('Owned native backend did not exit cleanly after SIGTERM; forced termination is not a passing shutdown')
      let listening = false
      try { const response = await fetch(backendURL + '/health', { signal: AbortSignal.timeout(500) }); await response.text(); listening = true } catch {}
      if (listening) throw Error('Owned native backend listener survived shutdown')
      console.log('Backend CLI shutdown smoke: PASS (SIGTERM, process exit, closed listener; no forced KILL)')
    } finally {await terminateOwned(test,'SIGINT')}
    await shutdownUnderSQLLock(binary, cliEnv, port)
  }
  if(process.exitCode===0||process.exitCode===undefined)for(const name of selectedGates) {
    const binary=process.env.BRAIN_BIN||join(process.env.CARGO_TARGET_DIR||join(project,'target'),'debug/brain')
    const result=await phase(process.execPath,[join(project,'scripts/gates',name+'.mjs')],{...process.env,BRAIN_BIN:binary,BRAIN_GATE_DATABASE_URL:`postgres://brain_test@127.0.0.1:${port}/postgres`,PG_BIN:pg},12*60*1000,'Release gate '+name)
    if(result!==0){process.exitCode=result;throw Error('Release gate failed: '+name)}
  }
} catch(e) {console.error(e.message);process.exitCode=1}
finally {await stop()}

function assertId(id){if(typeof id!=="string"||!/^[0-9a-f-]{36}$/.test(id))throw Error("CLI enqueue returned no UUID")}

// Exercise the whole process, not only pool.close(): a proven SQL waiter must not survive the HTTP drain deadline.
async function shutdownUnderSQLLock(binary, cliEnv, postgresPort) {
  const database = 'backend_shutdown'
  const pgArgs = ['-X','-h','127.0.0.1','-p',String(postgresPort),'-U','brain_test','-d',database,'-At','-v','ON_ERROR_STOP=1']
  const sql = statement => {
    const result = spawnSync(exe('psql'), [...pgArgs,'-c',statement], {encoding:'utf8',timeout:2000,maxBuffer:65536})
    if (result.error || result.status !== 0) throw Error('Owned shutdown SQL probe failed')
    return result.stdout.trim()
  }
  run(exe('createdb'), ['-h','127.0.0.1','-p',String(postgresPort),'-U','brain_test',database])
  const env = {...cliEnv,DATABASE_URL:`postgres://brain_test@127.0.0.1:${postgresPort}/${database}`,PUBLIC_SITE_URL:'https://example.test'}
  const app = join(dir,'shutdown-app')
  const cli = (args,cwd) => {
    const result = spawnSync(binary,args,{cwd,env,encoding:'utf8',timeout:20000,maxBuffer:65536})
    if (result.error || result.status !== 0) throw Error('Owned shutdown fixture CLI failed: '+(result.stderr || '').slice(0,1000))
  }
  cli(['new','shutdown-app','--json'],dir)
  cli(['add','resource','items','--fields','title:str!','--json'],app)
  writeFileSync(join(app,'server/resources/items/policy.jsbrain'),"<meta server>\nexport default { read: ['public'], create: ['user'], update: ['owner'], delete: ['owner'] }\n")
  const readyDelayMs = 250, drainMs = 500, exitMarginMs = 3000
  writeFileSync(join(app,'server/platform.jsbrain'),`<meta server>\nexport default { shutdown: { readyDelay: '${readyDelayMs}ms', drain: '${drainMs}ms' } }\n`)
  cli(['backend','migrate','--json'],app)
  const reservation = net.createServer()
  await new Promise((ok,no)=>reservation.once('error',no).listen(0,'127.0.0.1',ok))
  const httpPort = reservation.address().port
  await new Promise(ok=>reservation.close(ok))
  const url = `http://127.0.0.1:${httpPort}`
  let locker, suspendedDatabasePID, lockOutput = '', lockBytes = 0
  const requestAbort = new AbortController()
  let pending
  const startBackend = async () => {
    test = spawn(binary,['backend','serve','--port',String(httpPort),'--origin','https://example.test'],{cwd:app,env,stdio:['ignore','pipe','pipe'],detached:true})
    const child = test
    let output = '', bytes = 0
    child.once('error',()=>{})
    const capture = chunk => { bytes += chunk.length; if (bytes <= 65536) output += chunk.toString('utf8') }
    child.stdout.on('data',capture); child.stderr.on('data',capture)
    const until = Date.now()+15000
    while (Date.now()<until) {
      if (!running(child) || bytes>65536) throw Error('Owned shutdown backend exited or exceeded its output limit: '+output.slice(-2000))
      try {
        const response = await fetch(url+'/health/ready',{signal:AbortSignal.timeout(300)})
        await response.body?.cancel()
        if (response.status===200) return {child,output:()=>output}
      } catch {}
      await sleep(50)
    }
    throw Error('Owned shutdown backend did not become ready')
  }
  try {
    const active = await startBackend()
    locker = spawn(exe('psql'),pgArgs,{stdio:['pipe','pipe','pipe'],detached:true})
    auxiliary.add(locker); locker.once('error',()=>{}); locker.stdin.on('error',()=>{})
    locker.stdout.on('data',chunk=>{lockBytes+=chunk.length;if(lockBytes<=4096)lockOutput+=chunk.toString('utf8')})
    locker.stderr.on('data',chunk=>{lockBytes+=chunk.length})
    locker.stdin.write("BEGIN; LOCK TABLE public._brain_res_items IN ACCESS EXCLUSIVE MODE; SELECT 'LOCK_READY:' || pg_backend_pid();\n")
    let lockerPID
    const lockDeadline = Date.now()+5000
    while (Date.now()<lockDeadline) {
      if (!running(locker) || lockBytes>4096) throw Error('Owned SQL locker exited or exceeded its output limit')
      const match = lockOutput.match(/LOCK_READY:(\d+)/)
      if (match) { lockerPID=Number(match[1]); break }
      await sleep(20)
    }
    if (!lockerPID) throw Error('Owned SQL lock was not acquired')
    let settled = false
    pending = fetch(url+'/api/items',{headers:{Accept:'application/json'},signal:AbortSignal.any([requestAbort.signal,AbortSignal.timeout(15000)])})
      .then(async response=>{await response.body?.cancel();return response.status},()=>null).finally(()=>{settled=true})
    let blockedPID, applicationName
    const blockedDeadline = Date.now()+5000
    while (Date.now()<blockedDeadline) {
      if (settled) throw Error('Shutdown HTTP request finished before waiting on the SQL lock')
      const value = sql(`SELECT json_build_object('pid',a.pid,'application',a.application_name) FROM pg_stat_activity a JOIN pg_locks l ON l.pid=a.pid WHERE a.datname='${database}' AND a.backend_type='client backend' AND a.wait_event_type='Lock' AND l.relation='public._brain_res_items'::regclass AND NOT l.granted AND ${lockerPID}=ANY(pg_blocking_pids(a.pid))`)
      if (value) {const waiter=JSON.parse(value);blockedPID=waiter.pid;applicationName=waiter.application;break}
      await sleep(20)
    }
    if (!blockedPID || settled) throw Error('Shutdown fixture did not establish a live HTTP request waiting on its owned SQL lock')
    // The native pool's application_name identifies all its connections, including the idle ones.
    if (!/^brain:[A-Za-z0-9_.:-]+:http$/.test(applicationName)) throw Error('Blocked request was not owned by the native HTTP pool')
    const postmasterPID = Number(readFileSync(join(dir,'data/postmaster.pid'),'utf8').split('\n')[0])
    if (!Number.isSafeInteger(blockedPID) || blockedPID<=1 || blockedPID===postmasterPID || blockedPID===active.child.pid) throw Error('SQL waiter is not a distinct owned database session')
    // Only this owned cluster's observed waiter is suspended. Otherwise lock_timeout=1s could hide an unbounded
    // pool close by releasing the connection before our process-exit margin; an unresponsive DB must not do that.
    if (settled) throw Error('Shutdown HTTP request finished before suspending its owned SQL waiter')
    process.kill(blockedPID,'SIGSTOP')
    suspendedDatabasePID = blockedPID
    suspendedOwnedDatabase.add(blockedPID)
    if (settled || sql(`SELECT count(*) FROM pg_stat_activity a JOIN pg_locks l ON l.pid=a.pid WHERE a.pid=${blockedPID} AND a.datname='${database}' AND a.application_name='${applicationName}' AND a.backend_type='client backend' AND a.wait_event_type='Lock' AND l.relation='public._brain_res_items'::regclass AND NOT l.granted AND ${lockerPID}=ANY(pg_blocking_pids(a.pid))`)!=='1') throw Error('Suspended database session no longer owns the observed HTTP lock wait')
    const begin = Date.now()
    process.kill(active.child.pid,'SIGTERM')
    await waitExit(active.child,readyDelayMs+drainMs+exitMarginMs)
    const elapsedMs = Date.now()-begin
    if (running(active.child) || active.child.signalCode || ![0,2].includes(active.child.exitCode)) {
      console.error(JSON.stringify({event:'shutdown-deadline-failed',elapsedMs,pid:active.child.pid,blockedDatabasePID:blockedPID,requestPending:!settled,exitCode:active.child.exitCode,signal:active.child.signalCode}))
      console.error(active.output().slice(-8192))
      const threads = spawnSync('ps',[...(process.platform==='darwin'?['-M']:['-T']),'-p',String(active.child.pid)],{encoding:'utf8',timeout:2000,maxBuffer:16384})
      if (threads.stdout) console.error(threads.stdout)
      if (process.platform==='darwin' && running(active.child)) {
        const sample = join(dir,'shutdown-sample.txt')
        spawnSync('sample',[String(active.child.pid),'1','-file',sample],{encoding:'utf8',timeout:5000,maxBuffer:4096})
        try {console.error(readFileSync(sample,'utf8').slice(0,131072))} catch {}
      }
      throw Error('SQL-blocked backend did not exit itself within its shutdown budget; forced termination is not success')
    }
    const events = active.output().split('\n').flatMap(line=>{try{return [JSON.parse(line)]}catch{return []}})
    const draining = events.find(event=>event.event==='backend-draining')
    if (draining?.readyDelayMs!==readyDelayMs || draining.drainMs!==drainMs || !events.some(event=>event.code==='DRAIN_TIMEOUT')) throw Error('SQL-blocked shutdown did not report its declared drain deadline')
    // main.rs maps a runtime backend error to exit 2. Accept it only for this diagnosed incomplete pool cleanup.
    if (active.child.exitCode===2 && !events.some(event=>event.event==='backend-shutdown-incomplete' && event.code==='DRAIN_TIMEOUT' && event.phase==='pool')) throw Error('SQL-blocked shutdown exited with an unrelated backend error')
    process.kill(suspendedDatabasePID,'SIGCONT')
    suspendedOwnedDatabase.delete(suspendedDatabasePID)
    suspendedDatabasePID = undefined
    const connectionDeadline = Date.now()+2000
    let connections
    do {
      connections = Number(sql(`SELECT count(*) FROM pg_stat_activity WHERE datname='${database}' AND application_name IN ('${applicationName}','${applicationName.replace(/:http$/,':worker')}')`))
      if (connections===0) break
      await sleep(25)
    } while (Date.now()<connectionDeadline)
    if (connections!==0) throw Error('Native backend Postgres connections survived process shutdown')
    const released = net.createServer()
    await new Promise((ok,no)=>released.once('error',no).listen(httpPort,'127.0.0.1',ok))
    await new Promise(ok=>released.close(ok))
    locker.stdin.end('ROLLBACK;\n')
    await waitExit(locker,2000)
    if (running(locker) || locker.exitCode!==0) throw Error('Owned SQL locker did not release its transaction cleanly')
    auxiliary.delete(locker)
    await startBackend()
    const response = await fetch(url+'/api/items',{headers:{Accept:'application/json'},signal:AbortSignal.timeout(2000)})
    await response.body?.cancel()
    if (response.status!==200) throw Error('Restarted backend could not serve its resource on the same port')
    process.kill(test.pid,'SIGTERM')
    await waitExit(test,readyDelayMs+drainMs+exitMarginMs)
    if (running(test) || test.exitCode!==0) throw Error('Restarted idle backend did not exit cleanly without forced termination')
    console.log(`Backend CLI SQL-blocked shutdown smoke: PASS (observed SQL waiter, unresponsive owned DB connection, SIGTERM exit in ${elapsedMs} ms, closed connections and listener, same-port restart; no forced KILL)`)
  } finally {
    if (suspendedDatabasePID) {try {process.kill(suspendedDatabasePID,'SIGCONT')} catch {};suspendedOwnedDatabase.delete(suspendedDatabasePID)}
    requestAbort.abort()
    if (pending) await pending
    await terminateOwned(test)
    if (locker) {await terminateOwned(locker);auxiliary.delete(locker)}
  }
}
