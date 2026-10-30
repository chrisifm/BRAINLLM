import {test, after} from 'node:test'
import assert from 'node:assert/strict'
import {EventEmitter} from 'node:events'
import {createServer} from 'node:http'
import {nativeAuthBridge} from '../server/server.mjs'
const seen=[]
let mode='normal'
const backend=createServer((req,res)=>{
  const chunks=[]
  req.on('data',chunk=>chunks.push(chunk))
  req.on('end',()=>{
    seen.push({method:req.method,path:req.url,headers:req.headers,body:Buffer.concat(chunks).toString()})
    if(mode==='denied'){res.writeHead(403,{'content-type':'application/json'});res.end('{"error":"private driver detail"}');return}
    if(mode==='oversize'){res.writeHead(200,{'content-type':'application/json'});res.end(JSON.stringify({value:'x'.repeat(2*1024*1024)}));return}
    res.writeHead(req.method==='DELETE'?204:200,{'content-type':'application/json'})
    res.end(req.method==='DELETE'?'':JSON.stringify({ok:true,items:[]}))
  })
})
await new Promise(resolve=>backend.listen(0,'127.0.0.1',resolve))
const bridge=nativeAuthBridge(`http://127.0.0.1:${backend.address().port}`)
after(async()=>{bridge.close();await new Promise(resolve=>{backend.close(resolve);backend.closeAllConnections()})})
function context(method='POST'){
  const req=new EventEmitter(),res=new EventEmitter()
  req.method=method;req.headers={host:'app.example.test',origin:'https://app.example.test',cookie:'__Host-brain-access=synthetic; unrelated=private',authorization:'Bearer never-forward'}
  res.writableEnded=false;res.destroyed=false
  return {req,res,resource:bridge.resource(req,res,'00000000-0000-4000-8000-000000000004','notes')}
}
const id='00000000-0000-4000-8000-000000000005'
test('server repositories use native policy, cookie identity and bounded JSON transport without a second database client',async()=>{
  const {resource}=context()
  assert.deepEqual(await resource.list({search:'hello world',limit:2,offset:4}),{ok:true,items:[]})
  assert.equal(seen.at(-1).path,'/api/notes?search=hello+world&limit=2&offset=4')
  assert.equal(seen.at(-1).headers.cookie,'__Host-brain-access=synthetic')
  assert.equal(seen.at(-1).headers.authorization,undefined)
  await resource.get(id);assert.equal(seen.at(-1).path,'/api/notes/'+id)
  await resource.create({title:'New'});assert.equal(seen.at(-1).method,'POST');assert.equal(seen.at(-1).body,'{"title":"New"}')
  assert.equal(seen.at(-1).headers.origin,'https://app.example.test')
  await resource.update(id,{title:'Changed'});assert.equal(seen.at(-1).method,'PUT')
  assert.equal(await resource.remove(id),null);assert.equal(seen.at(-1).method,'DELETE')
})
test('server resource validation rejects injection, unbounded inputs and writes from GET before network access',async()=>{
  const {resource}=context(),before=seen.length
  assert.throws(()=>resource.get('../auth/me'),/identifier/)
  assert.throws(()=>resource.list({limit:101}),/pagination/)
  assert.throws(()=>resource.list({unknown:true}),/options/)
  assert.throws(()=>bridge.resource(context().req,context().res,'ref','../auth'),/name/)
  await assert.rejects(resource.create({title:'x'.repeat(65536)}),/byte budget/)
  await assert.rejects(context('GET').resource.create({title:'Unsafe'}),/POST/)
  const closed=context();closed.res.writableEnded=true;await assert.rejects(closed.resource.list(),/ended/)
  assert.equal(seen.length,before)
})
test('upstream failures remain generic and responses have a separate two MiB server budget',async()=>{
  const {resource}=context()
  try {
    mode='denied';await assert.rejects(resource.list(),error=>error.status===403&&!error.message.includes('driver'))
    mode='oversize';await assert.rejects(resource.list(),error=>error.status===502)
  } finally {mode='normal'}
})
