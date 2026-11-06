// The public MCP tools delegate to the same bounded current-app CLI without shell or implicit mutations.
import test from 'node:test'
import assert from 'node:assert/strict'
import {mkdtempSync,mkdirSync,writeFileSync,readFileSync,readdirSync,rmSync} from 'node:fs'
import {tmpdir} from 'node:os'
import {join,resolve} from 'node:path'
import {spawn} from 'node:child_process'
import {createInterface} from 'node:readline'
import {createServer} from 'node:http'
const binary=process.env.BRAIN_BIN&&resolve(process.env.BRAIN_BIN)
test('operator MCP tools preserve CLI verdicts, current-app read-only behavior and explicit apply confirmation',{timeout:60000},async()=>{
 assert.ok(binary,'BRAIN_BIN must select the integrated binary')
 const root=mkdtempSync(join(tmpdir(),'brain-operator-mcp-'))
 writeFileSync(join(root,'brain.json'),'{"name":"operator-fixture","docs":false}')
 writeFileSync(join(root,'tokens.cssbrain'),'')
 mkdirSync(join(root,'dist'))
 const sitemap='<urlset><url><loc>https://example.com/keep</loc></url></urlset>'
 writeFileSync(join(root,'baseline.xml'),sitemap);writeFileSync(join(root,'dist/sitemap.xml'),sitemap)
 const before=readdirSync(root).sort(),requests=[]
 const server=createServer((req,res)=>{requests.push({cookie:req.headers.cookie,authorization:req.headers.authorization});res.writeHead(200,{'content-type':'application/json'});res.end(req.method==='HEAD'?undefined:'{"ok":true}')})
 await new Promise(ok=>server.listen(0,'127.0.0.1',ok))
 const env={...process.env,DATABASE_URL:''}
 const child=spawn(binary,['mcp'],{cwd:root,env,stdio:['pipe','pipe','pipe']})
 const pending=new Map();let id=0,size=0
 const closed=new Promise(ok=>child.once('close',ok)),reader=createInterface({input:child.stdout})
 reader.on('line',line=>{size+=line.length;if(size>2*1024*1024){child.kill('SIGKILL');return}const value=JSON.parse(line),waiter=pending.get(value.id);if(waiter){pending.delete(value.id);clearTimeout(waiter.timer);waiter.ok(value)}})
 child.stderr.on('data',()=>{})
 const rpc=(method,params)=>new Promise((ok,no)=>{const current=++id,timer=setTimeout(()=>{pending.delete(current);no(Error('Owned operator MCP response deadline'))},10000);pending.set(current,{ok,timer});child.stdin.write(JSON.stringify({jsonrpc:'2.0',id:current,method,params})+'\n')})
 const call=async(name,request)=>{const reply=await rpc('tools/call',{name,arguments:{request}});assert.ok(!reply.error,JSON.stringify(reply));return reply.result}
 const report=async(name,request)=>{const result=await call(name,request);assert.equal(result.isError,false,JSON.stringify(result));return result.structuredContent??JSON.parse(result.content[0].text)}
 try{
  await rpc('initialize',{protocolVersion:'2025-06-18'})
  const tools=(await rpc('tools/list',{})).result.tools.map(tool=>tool.name)
  for(const name of ['brain_db_audit','brain_backend_conform','brain_google_setup','brain_seo_diff','brain_vitals_report','brain_data_backfill','brain_edge'])assert.ok(tools.includes(name),name)
  const conform=await report('brain_backend_conform',{url:'http://127.0.0.1:'+server.address().port,paths:['/status']});assert.equal(conform.exitCode,0);assert.equal(conform.report.observations.length,2);assert.ok(requests.every(row=>row.cookie===undefined&&row.authorization===undefined))
  const google=await report('brain_google_setup',{});assert.equal(google.exitCode,0);assert.equal(google.report.readOnly,true);assert.equal(google.report.verification.networkRequests,0)
  const coverage=await report('brain_seo_diff',{from:'baseline.xml'});assert.equal(coverage.exitCode,0);assert.equal(coverage.report.missingCount,0)
  const edge=await report('brain_edge',{operation:'plan'});assert.equal(edge.exitCode,0);assert.equal(edge.report.schemaVersion,1)
  for(const[name,request]of [['brain_db_audit',{}],['brain_vitals_report',{}],['brain_data_backfill',{name:'labels',environment:'development'}]]){const value=await report(name,request);assert.equal(value.ok,false);assert.notEqual(value.exitCode,0,'an unavailable CLI observation must retain its nonzero verdict')}
  for(const[name,request]of [['brain_edge',{operation:'apply',provider:'cloudflare',request:'plan.json'}],['brain_data_backfill',{name:'labels',environment:'development',apply:true,request:'plan.json'}],['brain_db_audit',{limit:201}],['brain_seo_diff',{from:'../private.xml'}],['brain_google_setup',{secret:'private-value'}]])assert.equal((await call(name,request)).isError,true)
  assert.deepEqual(readdirSync(root).sort(),before,'read-only tools must not write cache, journals or client settings')
  assert.equal(readFileSync(join(root,'dist/sitemap.xml'),'utf8'),sitemap)
 }finally{
  child.stdin.end();const timer=setTimeout(()=>child.kill('SIGKILL'),1000);await closed;clearTimeout(timer);reader.close();for(const waiter of pending.values())clearTimeout(waiter.timer)
  server.closeAllConnections();await new Promise(ok=>server.close(ok));rmSync(root,{recursive:true,force:true})
 }
})
