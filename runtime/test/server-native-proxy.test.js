import {test,after} from 'node:test'
import assert from 'node:assert/strict'
import {createServer as http} from 'node:http'
import {mkdtempSync,mkdirSync,writeFileSync,rmSync} from 'node:fs'
import {tmpdir} from 'node:os'
import {join} from 'node:path'
import {createServer} from '../server/server.mjs'
const directory=mkdtempSync(join(tmpdir(),'brain-native-proxy-'))
mkdirSync(join(directory,'_server'))
writeFileSync(join(directory,'_server/manifest.json'),JSON.stringify({routes:[],rpc:{},base:'/app/'}))
const seen=[]
const backend=http((req,res)=>{
  const chunks=[]
  req.on('data',chunk=>chunks.push(chunk))
  req.on('end',()=>{
    seen.push({path:req.url,method:req.method,headers:req.headers,body:Buffer.concat(chunks).toString()})
    if(req.url==='/_brain/uploads/slow'){
      const timer=setTimeout(()=>{res.writeHead(200,{'content-type':'application/json'});res.end(JSON.stringify({ok:true,media:true}))},10100)
      res.once('close',()=>clearTimeout(timer));return
    }
    if(req.url==='/s/short'){res.writeHead(302,{location:'/app/destination'});res.end();return}
    res.writeHead(req.method==='POST'?201:200,{'content-type':'application/json','set-cookie':['__Host-brain-access=synthetic; Path=/; Secure; HttpOnly','unrelated=never-relay; Path=/'],'x-private-upstream':'never-relay'})
    res.end(JSON.stringify({ok:true}))
  })
})
await new Promise(resolve=>backend.listen(0,'127.0.0.1',resolve))
const frontend=createServer({dist:directory,backendURL:`http://127.0.0.1:${backend.address().port}`,log(){}})
await new Promise(resolve=>frontend.listen(0,'127.0.0.1',resolve))
const base=`http://127.0.0.1:${frontend.address().port}`
after(async()=>{
  await new Promise(resolve=>{frontend.close(resolve);frontend.closeAllConnections()})
  await new Promise(resolve=>{backend.close(resolve);backend.closeAllConnections()})
  rmSync(directory,{recursive:true,force:true})
})
test('one frontend origin proxies native APIs with bounded body, native cookies and reference continuity',async()=>{
  const rid='00000000-0000-4000-8000-000000000003'
  const response=await fetch(base+'/app/_brain/auth/google',{method:'POST',headers:{origin:base,'content-type':'application/json',cookie:'__Host-brain-google=nonce; __Host-brain-refresh=refresh; private_cookie=never-forward','x-request-id':rid,'x-forwarded-host':'untrusted.test',authorization:'Bearer synthetic'},body:'{"credential":"synthetic"}'})
  assert.equal(response.status,201);assert.deepEqual(await response.json(),{ok:true})
  const request=seen.at(-1)
  assert.equal(request.path,'/_brain/auth/google');assert.equal(request.headers.origin,base)
  assert.equal(request.headers['x-request-id'],rid);assert.equal(request.headers['x-forwarded-host'],undefined)
  assert.equal(request.headers.cookie,'__Host-brain-google=nonce; __Host-brain-refresh=refresh')
  assert.equal(request.body,'{"credential":"synthetic"}')
  assert.equal(response.headers.get('x-private-upstream'),null)
  assert.equal(response.headers.getSetCookie().length,1)
  assert.match(response.headers.get('cache-control'),/no-store/)
})
test('native CRUD and short links retain method/redirect semantics while JSON oversize never reaches backend',async()=>{
  const read=await fetch(base+'/app/api/articles?limit=2');assert.equal(read.status,200)
  assert.equal(seen.at(-1).path,'/api/articles?limit=2')
  const redirect=await fetch(base+'/app/s/short',{redirect:'manual'});assert.equal(redirect.status,302);assert.equal(redirect.headers.get('location'),'/app/destination')
  const count=seen.length
  const oversize=await fetch(base+'/app/api/articles',{method:'POST',headers:{origin:base,'content-type':'application/json'},body:'x'.repeat(65537)})
  assert.equal(oversize.status,413);assert.equal(seen.length,count)
})

test('media responses retain the native processing window instead of the shorter general API timeout',{timeout:15000},async()=>{
  const response=await fetch(base+'/app/_brain/uploads/slow',{signal:AbortSignal.timeout(14000)})
  assert.equal(response.status,200);assert.deepEqual(await response.json(),{ok:true,media:true})
})

test('signed provider ingress preserves only its native signature headers and exact JSON bytes without requiring Origin',async()=>{
  const payload='{"event":"created","label":"Native provider"}'
  const response=await fetch(base+'/app/_brain/webhooks/in/provider',{method:'POST',headers:{'content-type':'application/json','x-brain-webhook-id':'00000000-0000-4000-8000-000000000009','x-brain-webhook-timestamp':'1234567890','x-brain-webhook-signature':'a'.repeat(64),'x-provider-secret':'never-forward'},body:payload})
  assert.equal(response.status,201)
  const request=seen.at(-1);assert.equal(request.body,payload);assert.equal(request.headers.origin,undefined)
  assert.equal(request.headers['x-brain-webhook-id'],'00000000-0000-4000-8000-000000000009')
  assert.equal(request.headers['x-brain-webhook-timestamp'],'1234567890');assert.equal(request.headers['x-brain-webhook-signature'],'a'.repeat(64))
  assert.equal(request.headers['x-provider-secret'],undefined)
})
