import { test } from 'node:test'
import assert from 'node:assert/strict'
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, rmSync, existsSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { spawnSync } from 'node:child_process'
import { createServer as httpsServer, request as httpsRequest } from 'node:https'
import { createServer } from '../server/server.mjs'
import { createSentryTunnel } from '../server/sentry.mjs'

const asset = '/_b/01234567.js', id = '01234567-89ab-5def-8123-456789abcdef'
function fixture(t, enabled) {
  const dir = mkdtempSync(join(tmpdir(),'brain-sentry-server-'))
  t.after(() => rmSync(dir,{recursive:true,force:true}))
  mkdirSync(join(dir,'_server'))
  writeFileSync(join(dir,'_server/manifest.json'),JSON.stringify({v:1,routes:[],rpc:{},...(enabled ? {sentry:{assets:{[asset]:id}}}: {})}))
  return dir
}
test('unused Sentry accepts no requests and has no startup dependency; opt-in missing or invalid DSN prevents listening',async t=>{
  const unused=createServer({dist:fixture(t,false),env:{},log(){}})
  await new Promise(ok=>unused.listen(0,'127.0.0.1',ok))
  t.after(async()=>{unused.closeAllConnections();await new Promise(ok=>unused.close(ok))})
  const response=await fetch(`http://127.0.0.1:${unused.address().port}/_brain/sentry`,{method:'POST',body:'{}'})
  assert.equal(response.status,404)
  const dist=fixture(t,true)
  assert.throws(()=>createServer({dist,env:{},log(){}}),/private BRAIN_SENTRY_DSN/)
  writeFileSync(join(dist,'_server/start.mjs'),"import{writeFileSync}from'node:fs';writeFileSync(new URL('./started',import.meta.url),'unexpected');export default()=>null")
  writeFileSync(join(dist,'_server/manifest.json'),JSON.stringify({v:1,core:'_server/start.mjs',routes:[],rpc:{},sentry:{assets:{[asset]:id}}}))
  const broken=createServer({dist,warm:true,env:{BRAIN_SENTRY_DSN:'private invalid setup'},log(){}})
  const error=await new Promise(ok=>{broken.once('error',ok);broken.listen(0,'127.0.0.1')})
  assert.match(error.message,/Sentry startup failed/);assert.doesNotMatch(error.message,/private invalid setup/);assert.equal(broken.listening,false)
  await assert.rejects(broken.$brainWarm,/BRAIN_SENTRY_DSN/);assert.equal(existsSync(join(dist,'_server/started')),false)
  broken.closeAllConnections()
})
const openssl = spawnSync('openssl',['version'],{stdio:'ignore',timeout:3000}).status===0
test('owned TLS endpoint validates certificates, receives one fixed envelope and cannot redirect transport', {skip:!openssl,timeout:15000},async t=>{
  const dir=fixture(t,false), key=join(dir,'test-key.pem'), cert=join(dir,'test-cert.pem')
  const generated=spawnSync('openssl',['req','-x509','-newkey','rsa:2048','-nodes','-keyout',key,'-out',cert,'-days','1','-subj','/CN=localhost','-addext','subjectAltName=DNS:localhost'],{stdio:'ignore',timeout:10000})
  assert.equal(generated.status,0,'owned test certificate generation')
  const bodies=[], seenHeaders=[]
  const upstream=httpsServer({key:readFileSync(key),cert:readFileSync(cert)},(req,res)=>{
    let body='';req.on('data',data=>{body+=data});req.on('end',()=>{bodies.push(body);seenHeaders.push(req.headers);res.writeHead(307,{location:'https://other.example.com/api/123/envelope/'});res.end()})
  })
  await new Promise(ok=>upstream.listen(0,'127.0.0.1',ok))
  t.after(async()=>{upstream.closeAllConnections();await new Promise(ok=>upstream.close(ok))})
  const host='localhost:'+upstream.address().port
  const request=(url,options,callback)=>httpsRequest(url,{...options,ca:readFileSync(cert),lookup:(_host,options,done)=>options.all?done(null,[{address:'127.0.0.1',family:4}]):done(null,'127.0.0.1',4)},callback)
  const tunnel=createSentryTunnel({dsn:'https://'+'a'.repeat(32)+'@'+host+'/123',approvedHost:host,assets:{[asset]:id},request})
  t.after(()=>tunnel.close())
  // Exercise the transport through the actual Node request/response streams.
  const {createServer:httpServer}=await import('node:http')
  const server=httpServer((req,res)=>tunnel.handle(req,res,req.socket.remoteAddress))
  await new Promise(ok=>server.listen(0,'127.0.0.1',ok))
  t.after(async()=>{server.closeAllConnections();await new Promise(ok=>server.close(ok))})
  const response=await fetch(`http://127.0.0.1:${server.address().port}/_brain/sentry`,{method:'POST',headers:{origin:`http://127.0.0.1:${server.address().port}`,'content-type':'application/json',cookie:'private cookie'},body:JSON.stringify({schemaVersion:1,type:'Error',frames:[{asset,debugId:id,line:2,column:7}]})})
  assert.equal(response.status,202);assert.equal(bodies.length,1);assert.equal(seenHeaders[0].cookie,undefined)
  assert.doesNotMatch(bodies[0],/private cookie|other\.example|user|request/)
})
