// Real CLI/SQL acceptance over backend-check's disposable cluster. No existing app or production database is used.
import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { randomUUID } from 'node:crypto'
import { readFileSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { binary, databaseURL, brain, createDatabase, gate, own, sleep, stop, write } from './lib.mjs'
const admin = new URL(databaseURL)
assert(['127.0.0.1','localhost','[::1]'].includes(admin.hostname) && admin.username === 'brain_test', 'Requires the owned loopback backend-check cluster')
const psql = process.env.PG_BIN ? join(process.env.PG_BIN,'psql') : 'psql'
const marker = 'private_fixture_' + randomUUID().replaceAll('-','')
function sql(url, text) {
  const result = spawnSync(psql, ['-X','-q','-A','-t','-v','ON_ERROR_STOP=1','-c',text], { env: {...process.env,PGHOST:url.hostname,PGPORT:url.port,PGUSER:decodeURIComponent(url.username),PGDATABASE:url.pathname.slice(1),PGPASSWORD:decodeURIComponent(url.password),PGOPTIONS:'',PGAPPNAME:'brain:backfill-gate'}, encoding:'utf8',timeout:10000,maxBuffer:1024*1024,stdio:['ignore','pipe','pipe'] })
  assert.equal(result.status,0,'Owned backfill fixture SQL failed (SQL and values omitted)');return result.stdout.trim()
}
await gate('computed-backfill',180000,async directory=>{
 const name='backfill_'+randomUUID().replaceAll('-','').slice(0,12);const url=new URL(createDatabase(name));let held,cloneName
 try {
  brain(['new','backfill-fixture','--static'],directory,{BRAIN_NO_DELEGATE:'1',DATABASE_URL:''})
  const app=join(directory,'backfill-fixture');const config=JSON.parse(readFileSync(join(app,'brain.json'),'utf8'));delete config.output;writeFileSync(join(app,'brain.json'),JSON.stringify(config))
  write(app,'server/resources/records/schema.jsbrain',"<meta server>\nexport default {label:{type:'str!'},amount:{type:'int'},code:{type:'str',unique:true}}\n")
  write(app,'server/resources/records/policy.jsbrain',"<meta server>\nexport default {api:false,audit:true}\n")
  const define=(job,compute='row.id')=>write(app,`server/backfills/${job}.jsbrain`,`<meta server>\nexport default {resource:'records',field:'code',compute:row=>${compute}}\n`)
  const invoke=(job,args=[],expected=0,environment='development',connection=url)=>{
   const result=spawnSync(binary,['data','backfill',job,'--env',environment,'--json',...args],{cwd:app,env:{...process.env,DATABASE_URL:connection.href,BRAIN_NO_DELEGATE:'1'},encoding:'utf8',timeout:35000,maxBuffer:1024*1024,stdio:['ignore','pipe','pipe']})
   const text=(result.stdout||'')+(result.stderr||'');assert(!text.includes(marker),'Backfill printed row data');assert(!text.includes(connection.href),'Backfill printed DATABASE_URL')
   if(result.status!==expected){let detail='safe error unavailable';try{detail=JSON.parse(result.stdout).error||detail}catch{};throw Error('Unexpected backfill exit '+result.status+' expected '+expected+': '+String(detail).slice(0,400))}
   let value;try{value=JSON.parse(result.stdout)}catch{throw Error('Backfill must return one JSON document')};return value
  }
  const approve=(job,request,args=[],expected=0,environment='development',connection=url)=>{write(app,'request.json',JSON.stringify(request));return invoke(job,['--apply','--request','request.json',...args],expected,environment,connection)}
  const snapshot=()=>sql(url,"SELECT jsonb_build_object('data',(SELECT coalesce(md5(string_agg(id::text||data::text||updated_at::text,',' ORDER BY id)),'') FROM public._brain_res_records),'checkpoints',(SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY name),'[]') FROM public._brain_data_backfills t),'audit',(SELECT count(*) FROM public._brain_audit_log))::text")
  const reset=()=>sql(url,"UPDATE public._brain_res_records SET data=(data-'code')||jsonb_build_object('label','"+marker+"')")
  define('codes')
  // Preview requires migration; it must not create either framework tables or a checkpoint.
  invoke('codes',[],1)
  assert.equal(sql(url,"SELECT count(*) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='public' AND c.relname LIKE '_brain_%'"),'0')
  brain(['backend','migrate','--env','development'],app,{DATABASE_URL:url.href,BRAIN_NO_DELEGATE:'1'})
  const owner=randomUUID();sql(url,`INSERT INTO public._brain_users(id,email,password_hash,role) VALUES('${owner}','owner@example.com','synthetic','admin')`)
  const ids=['00000000-0000-0000-0000-000000000000',...Array.from({length:5},(_,i)=>`00000000-0000-4000-8000-${String(i+1).padStart(12,'0')}`)]
  for(let i=0;i<ids.length;i++){const data={label:marker,amount:i+1};if(i===1)data.code='preserved';if(i===2)data.code=null;sql(url,`INSERT INTO public._brain_res_records(id,owner,data,created_at,updated_at) VALUES('${ids[i]}','${owner}','${JSON.stringify(data)}'::jsonb,'2020-01-01','2020-01-02')`)}
  let before=snapshot();const preview=invoke('codes',['--limit','2']);assert.equal(snapshot(),before);assert.equal(preview.applied,false);assert.equal(preview.sampledRows,2);assert(preview.request);assert(!JSON.stringify(preview).includes(ids[1]));
  let batch=approve('codes',preview.request,['--limit','2']);assert.equal(batch.updatedRows,1);assert.equal(batch.request.revision,1)
  if(process.env.BRAIN_GATE_PG_RESTART_REQUEST){
   const request=process.env.BRAIN_GATE_PG_RESTART_REQUEST;const identity=sql(url,'SELECT id FROM public._brain_backfill_identity');const boot=sql(url,'SELECT pg_postmaster_start_time()::text')
   writeFileSync(request,'requested',{mode:0o600});const end=Date.now()+15000
   while(readFileSync(request,'utf8')!=='ready'){assert(Date.now()<end,'Owned database supervisor restart deadline');await sleep(50)}
   assert.equal(sql(url,'SELECT id FROM public._brain_backfill_identity'),identity);assert.notEqual(sql(url,'SELECT pg_postmaster_start_time()::text'),boot)
  }
  before=snapshot();const stale=approve('codes',preview.request,['--limit','2']);assert.equal(stale.staleRequest,true);assert.equal(stale.updatedRows,0);assert.equal(snapshot(),before)
  // Each invocation is a new native process, proving durable resume rather than process-local state.
  while(!batch.complete)batch=approve('codes',batch.request,['--limit','2'])
  assert.equal(sql(url,"SELECT count(*) FROM public._brain_res_records WHERE data ? 'code' AND data->'code'<>'null'::jsonb"),'5')
  assert.equal(sql(url,`SELECT data->>'code' FROM public._brain_res_records WHERE id='${ids[0]}'`),ids[0],'Nil UUID row must not be silently skipped')
  assert.equal(sql(url,`SELECT data->>'code' FROM public._brain_res_records WHERE id='${ids[1]}'`),'preserved')
  assert.equal(sql(url,`SELECT data->'code' FROM public._brain_res_records WHERE id='${ids[2]}'`),'null')
  assert.equal(sql(url,"SELECT count(*) FROM public._brain_res_records WHERE updated_at<>'2020-01-02'::timestamptz"),'0')
  assert.equal(sql(url,"SELECT count(*) FROM public._brain_audit_log WHERE actor_kind='server' AND resource='records' AND fields=ARRAY['code']"),'4')
  before=snapshot();approve('codes',batch.request,['--limit','2']);assert.equal(snapshot(),before)
  define('nulls');const nullPreview=invoke('nulls',['--include-null']);approve('nulls',nullPreview.request,['--include-null']);assert.equal(sql(url,`SELECT data->>'code' FROM public._brain_res_records WHERE id='${ids[2]}'`),ids[2])
  // Rows inserted after preview belong to a later cohort, even when their UUID sorts after every original row.
  reset();define('cohort');const cohort=invoke('cohort');const later='ffffffff-ffff-4fff-8fff-ffffffffffff';sql(url,`INSERT INTO public._brain_res_records(id,owner,data) VALUES('${later}','${owner}','{"label":"${marker}","amount":1}'::jsonb)`);approve('cohort',cohort.request);assert.equal(sql(url,`SELECT data ? 'code' FROM public._brain_res_records WHERE id='${later}'`),'f');sql(url,`DELETE FROM public._brain_res_records WHERE id='${later}'`)
  define('bad_revision');const canonical=invoke('bad_revision');const negative={...canonical.request,revision:-1};before=snapshot();approve('bad_revision',negative,[],1);assert.equal(snapshot(),before)
  // Concurrent target writes are preserved; source calculations use the locked current row, never preview row values.
  reset();define('preserve');const preserve=invoke('preserve');sql(url,`UPDATE public._brain_res_records SET data=data||'{"code":"concurrent"}'::jsonb WHERE id='${ids[0]}'`);approve('preserve',preserve.request);assert.equal(sql(url,`SELECT data->>'code' FROM public._brain_res_records WHERE id='${ids[0]}'`),'concurrent')
  // A source row invalidated after preview aborts all writes and checkpoint initialization.
  reset();define('invalid','row.data.label.trim()');const invalid=invoke('invalid');sql(url,`UPDATE public._brain_res_records SET data=data||'{"label":7}'::jsonb WHERE id='${ids[1]}'`);before=snapshot();approve('invalid',invalid.request,[],1);assert.equal(snapshot(),before);reset()
  // Uniqueness failure after the first UPDATE rolls back rows, native audit events and checkpoint together.
  define('collision',"'same'");const collision=invoke('collision');before=snapshot();approve('collision',collision.request,[],1);assert.equal(snapshot(),before)
  define('drift');const drift=invoke('drift');define('drift',"row.id + '-changed'");before=snapshot();approve('drift',drift.request,[],1);assert.equal(snapshot(),before)
  define('environment');const selected=invoke('environment');before=snapshot();approve('environment',selected.request,[],1,'production');assert.equal(snapshot(),before)
  // Storage contract drift and unknown trigger side effects fail before any update.
  const schemaPath='server/resources/records/schema.jsbrain';const schemaBefore=readFileSync(join(app,schemaPath),'utf8')
  define('schema_change');const schemaPlan=invoke('schema_change');write(app,schemaPath,schemaBefore.replace("code:{type:'str',unique:true}","code:{type:'email',unique:true}"));before=snapshot();approve('schema_change',schemaPlan.request,[],1);assert.equal(snapshot(),before);write(app,schemaPath,schemaBefore)
  define('unknown_trigger');sql(url,"CREATE FUNCTION public._gate_custom() RETURNS trigger LANGUAGE plpgsql AS $$BEGIN RETURN NEW; END$$; CREATE TRIGGER _gate_custom BEFORE UPDATE ON public._brain_res_records FOR EACH ROW EXECUTE FUNCTION public._gate_custom()")
  before=snapshot();invoke('unknown_trigger',[],1);assert.equal(snapshot(),before);sql(url,'DROP TRIGGER _gate_custom ON public._brain_res_records; DROP FUNCTION public._gate_custom()')
  // A copied database also copies its identity UUID, but normalized connection/environment binding still differs.
  define('clone_control');const clonePlan=invoke('clone_control');cloneName=name+'_clone';sql(admin,`CREATE DATABASE "${cloneName}" TEMPLATE "${name}"`);const clone=new URL(url);clone.pathname='/'+cloneName;const cloneBefore=sql(clone,"SELECT count(*) FROM public._brain_data_backfills");approve('clone_control',clonePlan.request,[],1,'development',clone);assert.equal(sql(clone,"SELECT count(*) FROM public._brain_data_backfills"),cloneBefore)
  // Hold the first key: the backfill must fail boundedly instead of SKIP LOCKED advancing past that row.
  define('locked');const locked=invoke('locked');held=own(psql,['-X','-q','-c',`BEGIN; SELECT id FROM public._brain_res_records WHERE id='${ids[0]}' FOR UPDATE; SELECT pg_sleep(20); COMMIT`],directory,{PGHOST:url.hostname,PGPORT:url.port,PGUSER:url.username,PGDATABASE:name,PGAPPNAME:'brain:backfill-holder',PGOPTIONS:''})
  const deadline=Date.now()+5000;while(sql(url,"SELECT count(*) FROM pg_stat_activity WHERE application_name='brain:backfill-holder' AND wait_event='PgSleep'")!=='1'){assert(Date.now()<deadline,'Owned row-lock fixture readiness deadline');await sleep(50)}
  before=snapshot();approve('locked',locked.request,[],1);assert.equal(snapshot(),before);sql(url,"SELECT pg_cancel_backend(pid) FROM pg_stat_activity WHERE application_name='brain:backfill-holder'");await stop(held.child);held=null;approve('locked',locked.request)
  // Checkpoints survive a whole process restart; stale revisions cannot skip ahead or duplicate a committed batch.
  define('readonly');before=snapshot();const readonly=invoke('readonly');assert.equal(snapshot(),before);assert.equal(readonly.rowValuesCaptured,false)
  const saved=readFileSync(join(app,'server/resources/records/schema.jsbrain'),'utf8');write(app,'server/resources/records/schema.jsbrain',saved.replace("code:{type:'str',unique:true}","code:{type:'str',unique:true,access:'private'}"));invoke('readonly',[],1);write(app,'server/resources/records/schema.jsbrain',saved)
  // Metadata preflight rejects an oversized aggregate before transferring any document payload.
  const largeIds=Array.from({length:80},()=>randomUUID());for(const id of largeIds)sql(url,`INSERT INTO public._brain_res_records(id,owner,data,created_at,updated_at) VALUES('${id}','${owner}',jsonb_build_object('label',repeat('r',16384),'amount',1),'2020-01-01','2020-01-02')`)
  define('byte_budget');before=snapshot();const tooLarge=invoke('byte_budget',[],1);assert.match(tooLarge.error,/no documents were transferred/);assert.equal(snapshot(),before)
  const small=invoke('byte_budget',['--limit','10']);assert(small.documentBytes<=1048576);approve('byte_budget',small.request,['--limit','10'])
  sql(url,`DELETE FROM public._brain_res_records WHERE id=ANY(ARRAY[${largeIds.map(id=>"'"+id+"'").join(',')}]::uuid[])`)
  // Normal source gates recognize backfill files, without executing their compute functions or treating them as handlers.
  brain(['check'],app,{DATABASE_URL:url.href,BRAIN_NO_DELEGATE:'1'})
  brain(['build','--env','development','--site','https://example.com'],app,{DATABASE_URL:url.href,BRAIN_NO_DELEGATE:'1'})
  const doctor=spawnSync(binary,['doctor','--json'],{cwd:app,env:{...process.env,DATABASE_URL:url.href,BRAIN_NO_DELEGATE:'1'},encoding:'utf8',timeout:90000,maxBuffer:4*1024*1024});assert(!doctor.signal&&!doctor.error,'Owned doctor exceeded its bound');const diagnosis=JSON.parse(doctor.stdout);assert.equal(doctor.status,0,JSON.stringify({failed:diagnosis.failed,check:diagnosis.check?.errors,build:diagnosis.build?.ok}));assert.equal(diagnosis.ok,true,'Owned backfill app must pass doctor, not merely return JSON');assert.equal(diagnosis.check.errors,0);assert.equal(diagnosis.build.ok,true)
  console.log('computed-backfill: preview/read-only, preserve/null/nil UUID, typed invalid/unique rollback, atomic native audit/checkpoint, stale/resume/new process, concurrent target and locked row, source/env/restricted rejection, check/build/doctor verified')
 }finally{if(held)await stop(held.child);if(cloneName)sql(admin,`DROP DATABASE "${cloneName}" WITH (FORCE)`);sql(admin,`DROP DATABASE "${name}" WITH (FORCE)`)}
})
