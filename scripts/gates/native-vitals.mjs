// Real selected binary, owned PostgreSQL, both native gateways, prefix-aware builds and operator reports.
import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { readFileSync, writeFileSync, existsSync } from 'node:fs'
import { join } from 'node:path'
import { pathToFileURL } from 'node:url'
import { request } from 'node:http'
import { binary, databaseURL, brain, createDatabase, freePort, gate, own, ready, sleep, stop, write } from './lib.mjs'
const admin = new URL(databaseURL)
assert(['127.0.0.1','localhost','[::1]'].includes(admin.hostname) && admin.username === 'brain_test', 'Requires the explicitly owned loopback backend-check cluster')
function sql(url, text) {
  const result = spawnSync(process.env.PG_BIN ? join(process.env.PG_BIN,'psql') : 'psql', ['-X','-q','-A','-t','-v','ON_ERROR_STOP=1','-c',text], { env: { ...process.env, PGHOST:url.hostname, PGPORT:url.port, PGUSER:decodeURIComponent(url.username), PGDATABASE:url.pathname.slice(1), PGPASSWORD:decodeURIComponent(url.password), PGOPTIONS:'' }, encoding:'utf8',timeout:10000,maxBuffer:65536,stdio:['ignore','pipe','pipe'] })
  assert.equal(result.status,0,'Owned vitals SQL probe failed (SQL and values omitted)');return result.stdout.trim()
}
function call(port, path, body, extra = {}) {
  return new Promise((ok,no) => {
    const outgoing = request({ host:'127.0.0.1',port,path,method:body===undefined?'GET':'POST',agent:false,headers:{host:'example.test',origin:'https://example.test',accept:'application/json','content-type':'application/json',...extra},timeout:8000 }, incoming => {
      let text='';incoming.setEncoding('utf8');incoming.on('data',chunk=>{text+=chunk;if(text.length>1024*1024)outgoing.destroy(Error('Bounded response exceeded'))});incoming.on('end',()=>ok({status:incoming.statusCode,body:text,headers:incoming.headers}))
    })
    outgoing.once('error',no);outgoing.once('timeout',()=>outgoing.destroy(Error('Owned vitals HTTP deadline')));outgoing.end(body===undefined?undefined:typeof body==='string'?body:JSON.stringify(body))
  })
}
await gate('native-vitals',180000,async directory => {
  const database = new URL(createDatabase('gate_native_vitals'))
  const backendPort = await freePort(), nodePort = await freePort(), devPort = await freePort()
  const backendURL = `http://127.0.0.1:${backendPort}`
  const env = { DATABASE_URL:database.href,BRAIN_BACKEND_URL:backendURL,PUBLIC_SITE_URL:'https://example.test',BRAIN_ANALYTICS:'off',BRAIN_SECRET:'synthetic-vitals-session-secret-0123456789',BRAIN_MAIL_RESEND_KEY:'',GOOGLE_CLIENT_ID:'',BRAIN_MAIL_FROM:'',BRAIN_JWT_SECRET:'synthetic-vitals-jwt-secret-'.repeat(4),BRAIN_NO_DELEGATE:'1' }
  brain(['new','performance-fixture','--static'],directory,env)
  const app=join(directory,'performance-fixture'), config=JSON.parse(readFileSync(join(app,'brain.json'),'utf8'))
  delete config.output;config.base='/app';config.analytics=false;config.vitals={enabled:true,routes:['/','/items/:id'],retentionDays:30};config.backend={origins:['https://example.test']}
  const save=()=>writeFileSync(join(app,'brain.json'),JSON.stringify(config))
  save()
  write(app,'.brain_env',`PUBLIC_SITE_URL=https://example.test\n[development]\nBRAIN_BACKEND_URL=${backendURL}\nBRAIN_SECRET=${env.BRAIN_SECRET}\nBRAIN_ANALYTICS=off\n`)
  write(app,'views/home.brain','<meta seo title="Performance example" description="A synthetic document used to verify optional anonymous performance estimates.">\n<script>export async function load() { return { ok: true } }</script>\n<main><h1>Performance example</h1></main>\n')
  brain(['backend','migrate','--env','development','--json'],app,env)
  assert.equal(sql(database,'SELECT max(version) FROM _brain_migrations'),'30')
  let backend=own(binary,['backend','serve','--port',String(backendPort),'--origin','https://example.test'],app,env)
  await ready(backendURL+'/health',backend)
  brain(['build','--env','development'],app,env)
  const dist=join(app,'dist'), manifest=JSON.parse(readFileSync(join(dist,'_server/manifest.json'),'utf8')), page=manifest.routes.find(route=>route.path==='/').page, html=readFileSync(join(dist,page),'utf8')
  assert(html.includes('startNativeVitals({base:"/app",routes:["/","/items/:id"]})'))
  assert(!html.includes(database.href));assert(!html.includes(backendURL));assert(existsSync(join(dist,'server.mjs')))
  const script=`import{createServer}from ${JSON.stringify(pathToFileURL(join(dist,'server.mjs')).href)};const server=createServer({dist:process.argv[1],env:{BRAIN_BACKEND_URL:process.env.BRAIN_BACKEND_URL,BRAIN_ANALYTICS:'off'},secret:'synthetic-vitals-session-secret-0123456789',log(){}});server.listen(Number(process.argv[2]),'127.0.0.1');process.on('SIGTERM',()=>{server.closeAllConnections();server.close()})`
  const node=own(process.execPath,['--input-type=module','-e',script,dist,String(nodePort)],app,env)
  await ready(`http://127.0.0.1:${nodePort}/_health`,node)
  const dev=own(binary,['run','--port',String(devPort),'--base','/app/','--no-inspect'],app,env)
  await ready(`http://127.0.0.1:${devPort}/app/@brain/ping`,dev)
  const payload={route:'/items/:id',consent:true,metrics:[{name:'TTFB',value:42},{name:'LCP',value:2400}]}
  for(const port of [backendPort,nodePort,devPort]) {
    const path=port===backendPort?'/_brain/vitals':'/app/_brain/vitals'
    const response=await call(port,path,payload,{cookie:'__Host-brain=unrelated-invalid-session',authorization:'Bearer unrelated-invalid-key'})
    assert.equal(response.status,204,'Anonymous collector must ignore unrelated credentials in each gateway')
    assert.equal(response.headers['set-cookie'],undefined)
    const denied=await call(port,path,{...payload,consent:false});assert.equal(denied.status,400);assert.equal(JSON.parse(denied.body).error.code,'VITALS_INVALID')
    const privatePath=await call(port,path,{...payload,route:'/items/private-identifier'});assert.equal(privatePath.status,400)
    const forbidden=await call(port,path,payload,{origin:'https://foreign.example.test'});assert.equal(forbidden.status,403)
    for(const signal of ['dnt','sec-gpc']) {const ignored=await call(port,path,payload,{[signal]:'1'});assert.equal(ignored.status,204)}
  }
  assert.equal(sql(database,'SELECT sum(count) FROM _brain_vitals_daily'),'6')
  assert.equal(sql(database,'SELECT rows FROM _brain_vitals_capacity'),'2')
  assert.equal(sql(database,'SELECT count(*) FROM _brain_analytics_visitors'),'0');assert.equal(sql(database,'SELECT count(*) FROM _brain_analytics_sessions'),'0')
  const snapshot=()=>sql(database,"SELECT jsonb_build_object('rows',(SELECT jsonb_agg(to_jsonb(d) ORDER BY day,route_pattern,metric) FROM _brain_vitals_daily d),'capacity',(SELECT rows FROM _brain_vitals_capacity),'quota',(SELECT hits FROM _brain_rate_windows WHERE scope='native-vitals'))::text")
  const before=snapshot(), report=JSON.parse(brain(['vitals','report','--env','development','--days','7','--json'],app,env))
  assert.equal(snapshot(),before);assert.equal(report.readOnly,true);assert.equal(report.percentileAvailable,false);assert.equal(report.duplicateBeaconsMayCountTwice,true);assert.equal(report.uniqueVisitorsAvailable,false)
  assert.equal(report.groups.length,2);assert(report.groups.every(row=>row.route==='/items/:id'&&row.count===3));assert(!JSON.stringify(report).includes('private-identifier'))
  // A SELECT-only operator role can report without migration or any write privilege.
  sql(database,"CREATE ROLE gate_vitals_reader LOGIN; GRANT USAGE ON SCHEMA public TO gate_vitals_reader; GRANT SELECT ON public._brain_vitals_daily TO gate_vitals_reader")
  const reader=new URL(database);reader.username='gate_vitals_reader';reader.password=''
  const readOnly=JSON.parse(brain(['vitals','report','--days','7','--json'],app,{...env,DATABASE_URL:reader.href}));assert.equal(readOnly.readOnly,true);assert.equal(snapshot(),before)
  const mcp=spawnSync(binary,['mcp','--profile','backend'],{cwd:app,env:{...process.env,...env,DATABASE_URL:reader.href},input:[{jsonrpc:'2.0',id:1,method:'initialize',params:{protocolVersion:'2025-06-18'}},{jsonrpc:'2.0',id:2,method:'tools/call',params:{name:'brain_vitals_report',arguments:{request:{days:7}}}}].map(value=>JSON.stringify(value)+'\n').join(''),encoding:'utf8',timeout:15000,maxBuffer:1024*1024})
  assert.equal(mcp.status,0,'Owned MCP report process must exit without mutation or a stalled child')
  const reply=mcp.stdout.trim().split('\n').map(line=>JSON.parse(line)).find(value=>value.id===2)
  assert.equal(reply.result.isError,false)
  const wrapped=reply.result.structuredContent??JSON.parse(reply.result.content[0].text)
  assert.equal(wrapped.exitCode,0);assert.equal(wrapped.report.readOnly,true);assert.equal(wrapped.report.groups.length,2);assert.equal(snapshot(),before,'MCP must preserve the same SELECT-only report contract')

  const unknown=await call(nodePort,'/_brain/vitals',payload);assert.equal(unknown.status,404,'base namespace is not widened')
  const large=await call(backendPort,'/_brain/vitals',JSON.stringify({...payload,padding:'x'.repeat(4096)}));assert.equal(large.status,413);assert.equal(JSON.parse(large.body).error.code,'VITALS_TOO_LARGE')
  await stop(backend.child);config.vitals=false;save();backend=own(binary,['backend','serve','--port',String(backendPort),'--origin','https://example.test'],app,env);await ready(backendURL+'/health',backend)
  assert.equal((await call(nodePort,'/app/_brain/vitals',payload)).status,404,'disabled native collector stays absent through the Node gateway')
  await stop(dev.child);await stop(node.child);await stop(backend.child)
  // Bootstrap never upgrades a portable/static app into Node output.
  config.vitals={enabled:true,routes:['/'],retentionDays:30};config.output='static';save()
  const invalid=spawnSync(binary,['build','--env','development','--json'],{cwd:app,env:{...process.env,...env},encoding:'utf8',timeout:30000,maxBuffer:65536});assert.notEqual(invalid.status,0)
  // Focused local acceptance may select the already-built integration executable. Normal release runs it separately.
  if(process.env.BRAIN_NATIVE_VITALS_TEST_BIN) {
    const result=spawnSync(process.env.BRAIN_NATIVE_VITALS_TEST_BIN,['vitals::','--include-ignored','--nocapture','--test-threads=1'],{env:{...process.env,BRAIN_BACKEND_TEST_URL:databaseURL},encoding:'utf8',timeout:90000,maxBuffer:65536});assert.equal(result.status,0,(result.stdout+result.stderr).slice(-4000))
  }
})
