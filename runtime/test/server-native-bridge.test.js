// The front/backend bridge: client identity, streamed files, declared routes and conditional requests.
import {test,after} from 'node:test'
import assert from 'node:assert/strict'
import {createServer as http,request} from 'node:http'
import {mkdtempSync,mkdirSync,writeFileSync,rmSync} from 'node:fs'
import {tmpdir} from 'node:os'
import {join} from 'node:path'
import {createServer,resolveClient,trustedProxies} from '../server/server.mjs'
const directory=mkdtempSync(join(tmpdir(),'brain-native-bridge-'))
mkdirSync(join(directory,'_server'))
writeFileSync(join(directory,'_server/manifest.json'),JSON.stringify({routes:[{path:'/pricing'}],rpc:{},nativeRoutes:[{path:'/payments/events',headers:['x-pay-signature']},{path:'/orders/{id}/quote',headers:[]}]}))
writeFileSync(join(directory,'pricing.html'),'<!doctype html><title>Pricing</title>')
const seen=[]
const MiB=1024*1024
let release=()=>{},closedEarly=()=>{}
const backend=http((req,res)=>{
  const entry={path:req.url,method:req.method,headers:req.headers,bytes:0,parts:[]};seen.push(entry)
  if(req.url==='/_brain/files/attachments/stalled'){
    res.writeHead(200,{'content-type':'application/pdf','content-length':String(25*MiB)})
    res.write(Buffer.alloc(64*1024,1))
    res.once('close',()=>closedEarly(res.writableFinished))
    release=()=>res.end(Buffer.alloc(25*MiB-64*1024,1))
    return
  }
  if(req.url==='/_brain/files/attachments/large'){
    res.writeHead(200,{'content-type':'application/pdf','content-length':String(25*MiB),'etag':'"f1"','accept-ranges':'bytes','content-security-policy':"default-src 'none'",'x-content-type-options':'nosniff','cache-control':'public, max-age=31536000, immutable','x-private-upstream':'never-relay'})
    res.end(req.method==='HEAD'?undefined:Buffer.alloc(25*MiB,2));return
  }
  if(req.url==='/_brain/files/attachments/ranged'){
    if(req.headers['if-none-match']==='"f1"'){res.writeHead(304,{etag:'"f1"'});res.end();return}
    res.writeHead(206,{'content-type':'application/pdf','content-range':'bytes 0-3/10','content-length':'4','accept-ranges':'bytes',etag:'"f1"'})
    res.end('abcd');return
  }
  req.on('data',chunk=>{entry.bytes+=chunk.length;if(entry.bytes<=2*MiB)entry.parts.push(chunk)})
  req.on('end',()=>{
    if(req.url==='/_brain/files/attachments/refused'){res.writeHead(413,{'content-type':'application/json'});res.end('{"code":"FILE_TOO_LARGE"}');return}
    if(req.url==='/api/items/1'&&req.headers['if-none-match']==='"r1"'){res.writeHead(304,{etag:'"r1"','cache-control':'public, max-age=60'});res.end();return}
    if(req.url==='/api/items/1'&&req.headers['if-match']==='"old"'){res.writeHead(412,{'content-type':'application/json'});res.end('{"code":"PRECONDITION_FAILED"}');return}
    if(req.url==='/api/items/1'){res.writeHead(200,{'content-type':'application/json',etag:'"r1"','cache-control':'public, max-age=60'});res.end('{}');return}
    if(req.url.startsWith('/payments/')||req.url.startsWith('/orders/')){res.writeHead(202,{'content-type':'application/json','x-shop-trace':'t1',link:'</next>; rel="next"','content-language':'en','cache-control':'public, max-age=5','set-cookie':'unrelated=never-relay'});res.end('{"ok":true}');return}
    res.writeHead(req.url==='/_brain/auth/me'?401:200,{'content-type':'application/json'});res.end('{"ok":true,"items":[]}')
  })
})
await new Promise(resolve=>backend.listen(0,'127.0.0.1',resolve))
const frontend=createServer({dist:directory,backendURL:`http://127.0.0.1:${backend.address().port}`,log(){}})
await new Promise(resolve=>frontend.listen(0,'127.0.0.1',resolve))
const port=frontend.address().port,base=`http://127.0.0.1:${port}`
after(async()=>{
  await new Promise(resolve=>{frontend.close(resolve);frontend.closeAllConnections()})
  await new Promise(resolve=>{backend.close(resolve);backend.closeAllConnections()})
  rmSync(directory,{recursive:true,force:true})
})
test('the connecting peer is appended after any client-supplied hops, so a browser header cannot set the identity',async()=>{
  await (await fetch(base+'/api/items')).text()
  assert.equal(seen.at(-1).headers['x-forwarded-for'],'127.0.0.1')
  await (await fetch(base+'/api/items',{headers:{'x-forwarded-for':'1.1.1.1'}})).text()
  assert.equal(seen.at(-1).headers['x-forwarded-for'],'1.1.1.1, 127.0.0.1','the right-most hop is the peer the gateway observed')
  await (await fetch(base+'/api/items',{headers:{'x-forwarded-for':'9'.repeat(5000)}})).text()
  assert.equal(seen.at(-1).headers['x-forwarded-for'],'127.0.0.1','an oversized chain is dropped, never relayed')
})
test('a declared webhook keeps its raw bytes and declared headers; other headers stay private',async()=>{
  const raw=Buffer.from([0x7b,0x20,0x20,0xff,0x00,0x0a,0x7d,0x20])
  const response=await fetch(base+'/payments/events?attempt=1',{method:'POST',headers:{'content-type':'application/octet-stream','x-pay-signature':'sha256=abc','x-private':'never-forward',cookie:'__Host-brain=s; other=never-forward'},body:raw})
  assert.equal(response.status,202)
  const request=seen.at(-1)
  assert.equal(request.path,'/payments/events?attempt=1')
  assert.deepEqual(Buffer.concat(request.parts),raw)
  assert.equal(request.headers['x-pay-signature'],'sha256=abc');assert.equal(request.headers['x-private'],undefined)
  assert.equal(request.headers.cookie,'__Host-brain=s');assert.equal(request.headers['x-forwarded-for'],'127.0.0.1')
  assert.equal(response.headers.get('x-shop-trace'),'t1');assert.equal(response.headers.get('link'),'</next>; rel="next"')
  assert.equal(response.headers.get('content-language'),'en');assert.equal(response.headers.get('cache-control'),'public, max-age=5')
  assert.deepEqual(response.headers.getSetCookie(),[])
})
test('declared path parameters match one segment; unknown paths and views never reach the backend',async()=>{
  assert.equal((await fetch(base+'/orders/42/quote',{method:'POST'})).status,202)
  const count=seen.length
  assert.equal((await fetch(base+'/orders/42/43/quote',{method:'POST'})).status,405)
  assert.equal((await fetch(base+'/not/declared',{method:'POST'})).status,405)
  assert.equal((await fetch(base+'/not/declared')).status,404)
  assert.equal(seen.length,count)
})
test('declared routes accept the 1 MiB contract ceiling and refuse more before reaching the backend',async()=>{
  assert.equal((await fetch(base+'/payments/events',{method:'POST',body:Buffer.alloc(MiB,1)})).status,202)
  assert.equal(seen.at(-1).bytes,MiB)
  const count=seen.length
  assert.equal((await fetch(base+'/payments/events',{method:'POST',body:Buffer.alloc(MiB+1,1)})).status,413)
  assert.equal(seen.length,count)
})
test('native batches keep the 1 MiB backend limit through the bridge while plain bodies keep 64 KiB',async()=>{
  assert.equal((await fetch(base+'/api/items/batch',{method:'POST',body:Buffer.alloc(512*1024,1)})).status,200)
  assert.equal((await fetch(base+'/api/items',{method:'POST',body:Buffer.alloc(65537,1)})).status,413)
})
test('a download above the buffered budget streams with its range, validator and safety headers',async()=>{
  const response=await fetch(base+'/_brain/files/attachments/large',{headers:{'x-private':'never-forward'}})
  assert.equal(response.status,200)
  for(const [name,value] of [['content-length',String(25*MiB)],['etag','"f1"'],['accept-ranges','bytes'],['content-security-policy',"default-src 'none'"],['x-content-type-options','nosniff'],['cache-control','public, max-age=31536000, immutable']]) assert.equal(response.headers.get(name),value,name)
  assert.equal(response.headers.get('x-private-upstream'),null)
  assert.equal((await response.arrayBuffer()).byteLength,25*MiB)
  const head=await fetch(base+'/_brain/files/attachments/large',{method:'HEAD'})
  assert.equal(head.status,200);assert.equal(head.headers.get('content-length'),String(25*MiB))
})
test('range and conditional headers reach the bucket and 206/304 come back',async()=>{
  const partial=await fetch(base+'/_brain/files/attachments/ranged',{headers:{range:'bytes=0-3','if-range':'"f1"'}})
  assert.equal(partial.status,206);assert.equal(partial.headers.get('content-range'),'bytes 0-3/10');assert.equal(await partial.text(),'abcd')
  assert.equal(seen.at(-1).headers.range,'bytes=0-3');assert.equal(seen.at(-1).headers['if-range'],'"f1"')
  const fresh=await fetch(base+'/_brain/files/attachments/ranged',{headers:{'if-none-match':'"f1"'}})
  assert.equal(fresh.status,304);assert.equal(fresh.headers.get('etag'),'"f1"')
  await (await fetch(base+'/api/items',{headers:{range:'bytes=0-1'}})).text()
  assert.equal(seen.at(-1).headers.range,undefined,'Range is a file header only')
})
test('the first bytes arrive before the backend finishes and a client disconnect destroys the upstream',async()=>{
  const closed=new Promise(resolve=>{closedEarly=resolve})
  const controller=new AbortController()
  const response=await fetch(base+'/_brain/files/attachments/stalled',{signal:controller.signal})
  assert.equal(response.status,200)
  const reader=response.body.getReader()
  const first=await reader.read()
  assert.ok(first.value.length>0,'bytes flow while the backend has not finished')
  controller.abort()
  assert.equal(await closed,false,'the upstream was destroyed before finishing')
})
test('uploads stream past 64 KiB, and a refusal from the bucket reaches the client',async()=>{
  const form=new FormData();form.append('file',new Blob([Buffer.alloc(300*1024,1)]),'notes.txt')
  const response=await fetch(base+'/_brain/files/attachments',{method:'POST',headers:{origin:base},body:form})
  assert.equal(response.status,200);assert.ok(seen.at(-1).bytes>300*1024)
  assert.match(seen.at(-1).headers['content-type'],/^multipart\/form-data/)
  const refused=await fetch(base+'/_brain/files/attachments/refused',{method:'POST',body:Buffer.alloc(1024,1)})
  assert.equal(refused.status,413);assert.equal((await refused.json()).code,'FILE_TOO_LARGE')
})
test('long file streams do not consume the eight API operations',async()=>{
  const closed=new Promise(resolve=>{closedEarly=resolve})
  const held=[];for(let i=0;i<9;i++){const controller=new AbortController();held.push([controller,await fetch(base+'/_brain/files/attachments/stalled',{signal:controller.signal})])}
  assert.ok(held.every(([,response])=>response.status===200))
  assert.equal((await fetch(base+'/api/items')).status,200)
  for(const [controller] of held)controller.abort()
  await closed
})
test('API validators and declared public lifetimes survive the bridge, so If-Match can refuse a lost update',async()=>{
  const read=await fetch(base+'/api/items/1')
  assert.equal(read.headers.get('etag'),'"r1"');assert.equal(read.headers.get('cache-control'),'public, max-age=60')
  const revalidated=await fetch(base+'/api/items/1',{headers:{'if-none-match':'"r1"'}})
  assert.equal(revalidated.status,304);assert.equal(revalidated.headers.get('cache-control'),'public, max-age=60')
  assert.equal((await fetch(base+'/api/items/1',{method:'PATCH',headers:{'if-match':'"old"'},body:'{}'})).status,412)
  const plain=await fetch(base+'/api/items');assert.equal(plain.headers.get('cache-control'),'no-store')
})

test('a project API key crosses the bridge: its identity, its refusal and its exemption from Origin are the backend\'s',async()=>{
  await (await fetch(base+'/api/items',{method:'POST',headers:{'x-api-key':'bk_live_0123456789ab_secret','content-type':'application/json'},body:'{}'})).text()
  assert.equal(seen.at(-1).headers['x-api-key'],'bk_live_0123456789ab_secret')
  assert.equal(seen.at(-1).headers.origin,undefined,'the bridge invents no Origin for a keyed call')
  await (await fetch(base+'/api/items',{headers:{authorization:'Bearer bk_live_0123456789ab_secret'}})).text()
  assert.equal(seen.at(-1).headers.authorization,'Bearer bk_live_0123456789ab_secret')
})
test('client address: same walk as the backend, a header counts only through a trusted proxy',()=>{
  const local=trustedProxies(['127.0.0.1','::1','10.0.0.0/8'])
  assert.equal(resolveClient('203.0.113.1','1.1.1.1',local),'203.0.113.1','an untrusted peer ignores the header')
  assert.equal(resolveClient('127.0.0.1','1.1.1.1',null),'127.0.0.1','nothing configured: the header is ignored')
  assert.equal(resolveClient('::ffff:127.0.0.1','1.1.1.1, 203.0.113.9',local),'203.0.113.9','left of the first untrusted hop is client-supplied')
  assert.equal(resolveClient('127.0.0.1','203.0.113.9, 10.0.0.2',local),'203.0.113.9','trusted hops are walked')
  assert.equal(resolveClient('127.0.0.1','10.0.0.3, 10.0.0.2',local),'10.0.0.3')
  assert.equal(resolveClient('127.0.0.1','garbage',local),'127.0.0.1','an unreadable hop stops the walk')
  assert.equal(resolveClient('127.0.0.1','',local),'127.0.0.1')
})
test('trustProxy refuses ranges wide enough to trust everyone, like the backend',()=>{
  for(const wide of ['0.0.0.0/0','0.0.0.0/1','128.0.0.0/1','::/0','8000::/1','2000::/15'])assert.throws(()=>trustedProxies([wide]),/too wide/,wide)
  assert.throws(()=>trustedProxies(['not-an-ip']),/IP address or CIDR/)
  for(const fine of ['127.0.0.1','::1','10.0.0.0/8','2a06:98c0::/29','fc00::/7'])trustedProxies([fine])
})
