// Real generated service/RPC/SSR execution against an owned loopback backend; no provider or credentials.
import assert from 'node:assert/strict'
import {createServer as httpServer} from 'node:http'
import {readFileSync} from 'node:fs'
import {join} from 'node:path'
import {pathToFileURL} from 'node:url'
const dist=process.env.BRAIN_CRUD_DIST
assert.ok(dist,'The Rust generated-resource fixture supplies its owned dist directory')
const manifest=JSON.parse(readFileSync(join(dist,'_server/manifest.json'),'utf8'))
const {createServer}=await import(pathToFileURL(join(dist,'server.mjs')).href)
let row,deny=false
const calls=[]
const id='00000000-0000-4000-8000-000000000002'
const owner='00000000-0000-4000-8000-000000000001'
const native=httpServer((req,res)=>{
 const chunks=[];let bytes=0
 req.setTimeout(5000,()=>req.destroy())
 req.on('data',chunk=>{bytes+=chunk.length;if(bytes>65536)req.destroy();else chunks.push(chunk)})
 req.on('end',()=>{
  const cookie=req.headers.cookie||''
  const admin=cookie.includes('synthetic-admin'),candidate=cookie.includes('synthetic-candidate')
  res.setHeader('content-type','application/json');res.setHeader('cache-control','no-store')
  if(req.url==='/_brain/auth/me'){
   if(!admin&&!candidate){res.writeHead(401);res.end('{"error":"Sign in"}');return}
   res.end(JSON.stringify({user:{id:owner,email:'synthetic@example.test',role:admin?'admin':'candidate',verified:true}}));return
  }
  if(calls.length>=128){res.writeHead(429);res.end('{}');return}
  calls.push({method:req.method,path:req.url,cookie,origin:req.headers.origin,authorization:req.headers.authorization})
  if(!admin||deny){res.writeHead(403);res.end('{"error":"private-driver-detail"}');return}
  const path=new URL(req.url,'http://fixture.test').pathname
  if(req.method==='POST'&&path==='/api/posts')row={id,owner,data:JSON.parse(Buffer.concat(chunks).toString())}
  else if(req.method==='PUT'&&path==='/api/posts/'+id)row={id,owner,data:JSON.parse(Buffer.concat(chunks).toString())}
  else if(req.method==='DELETE'&&path==='/api/posts/'+id){row=undefined;res.end('{"ok":true}');return}
  else if(req.method==='GET'&&path==='/api/posts'){res.end(JSON.stringify({items:row?[row]:[],limit:25,offset:0}));return}
  else if(req.method!=='GET'||path!=='/api/posts/'+id){res.writeHead(404);res.end('{}');return}
  if(!row){res.writeHead(404);res.end('{}');return}
  res.end(JSON.stringify(row))
 })
})
native.maxConnections=8
native.headersTimeout=5000
native.requestTimeout=5000
let app
const listen=server=>new Promise((ok,no)=>server.once('error',no).listen(0,'127.0.0.1',ok))
const close=server=>server?new Promise(ok=>{server.close(ok);server.closeAllConnections()}):Promise.resolve()
try{
 await listen(native)
 app=createServer({dist,backendURL:`http://127.0.0.1:${native.address().port}`,env:{},log:()=>{}})
 await listen(app)
 const base=`http://127.0.0.1:${app.address().port}`
 const findRpc=path=>{
  const route=manifest.routes.find(route=>route.path===path)
  assert.ok(route?.server,'Generated route has a compiled server module: '+path)
  const entry=Object.entries(manifest.rpc||{}).find(([,module])=>module===route.server)
  assert.ok(entry,'Generated route exposes its own RPC module: '+path)
  return entry[0]
 }
 const createHash=findRpc('/admin/posts/new')
 const editHash=findRpc('/admin/posts/:id/edit')
 const hiddenKey='__brainRecord'
 let hash=createHash
 const cookie='__Host-brain-access=synthetic-admin; unrelated=must-not-forward'
 const rpc=(name,args,auth=cookie)=>fetch(`${base}/_r/${hash}/${name}`,{method:'POST',headers:{'content-type':'application/json','x-brain-rpc':'1',origin:base,cookie:auth},body:JSON.stringify(args),signal:AbortSignal.timeout(5000)})
 for(const auth of ['', '__Host-brain-access=synthetic-candidate']){
  const response=await fetch(base+'/admin/posts',{headers:{cookie:auth},signal:AbortSignal.timeout(5000)})
  assert.equal(response.status,403)
  const before=calls.length
  const rejected=await rpc('saveRecord',[{title:'Denied',active:false,score:4.5,meta:'{}',[hiddenKey]:''}],auth)
  assert.equal(rejected.status,403)
  assert.equal(calls.length,before,'Denied roles cannot reach native mutations')
 }
 let response=await rpc('saveRecord',[{title:'First',active:false,score:4.5,meta:'{}',[hiddenKey]:''}])
 assert.equal(response.status,200,await response.clone().text())
 assert.equal((await response.json()).data.title,'First')
 assert.equal(calls.at(-1).method,'POST')
 assert.equal(calls.at(-1).cookie,'__Host-brain-access=synthetic-admin')
 assert.equal(calls.at(-1).authorization,undefined)
 assert.equal(calls.at(-1).origin,base)
 assert.equal(row.data.active,false)
 assert.equal(row.data.score,4.5)
 assert.deepEqual(row.data.meta,{})
 hash=editHash
 response=await fetch(base+'/admin/posts/'+id+'/edit',{headers:{cookie},signal:AbortSignal.timeout(5000)})
 assert.equal(response.status,200)
 const editor=await response.text()
 assert.ok(editor.includes('First'),'SSR load uses the generated service and the native row')
 assert.match(editor,/<input[^>]*type="number"[^>]*step="any"/,'Project Input accepts native decimal numbers')
 for(const input of editor.match(/<input\b[^>]*>/g)||[]) if(/type="(?:text|email)"/.test(input)) assert.ok(!/\bstep=/.test(input),'Text/email controls do not receive a numeric step')
 const before=calls.length
 response=await rpc('saveRecord',[{title:'Bad',active:false,owner:'forged',[hiddenKey]:''}])
 assert.equal(response.status,422)
 assert.equal(calls.length,before,'Unknown values fail validation before native transport')
 response=await rpc('saveRecord',[{title:'Updated',active:false,score:2.5,meta:'{"note":"Synthetic"}',[hiddenKey]:id}])
 assert.equal(response.status,200)
 assert.equal((await response.json()).data.title,'Updated')
 assert.equal(calls.at(-1).method,'PUT')
 const noMutation=calls.length
 response=await rpc('saveRecord',[{title:'Bad JSON',active:false,meta:'{broken',[hiddenKey]:id}])
 assert.equal(response.status,422)
 const invalid=await response.json();assert.equal(invalid.errors.meta,'Use a valid field value')
 assert.ok(!JSON.stringify(invalid).includes('SyntaxError'))
 response=await rpc('removeRecord',[{[hiddenKey]:id,confirmation:false}]);assert.equal(response.status,422)
 response=await rpc('saveRecord',[{title:'Bad number',active:false,score:'0x10',[hiddenKey]:id}]);assert.equal(response.status,422)
 response=await rpc('saveRecord',[{title:'Bad boolean',active:'false',[hiddenKey]:id}]);assert.equal(response.status,422)
 assert.equal(calls.length,noMutation)
 // Filters/pagination and detail are complete server-rendered views, with no client API fetch.
 response=await fetch(base+'/admin/posts?search=Updated&offset=25',{headers:{cookie},signal:AbortSignal.timeout(5000)})
 assert.equal(response.status,200);assert.ok((await response.text()).includes('Updated'))
 assert.match(calls.at(-1).path,/search=Updated/);assert.match(calls.at(-1).path,/offset=25/)
 response=await fetch(base+'/admin/posts/'+id,{headers:{cookie},signal:AbortSignal.timeout(5000)})
 assert.equal(response.status,200);assert.ok((await response.text()).includes('Updated'))
 // Native form POST uses the same generated service and strict scalar/object decoding without JS.
 response=await fetch(base+'/admin/posts/'+id+'/edit',{method:'POST',redirect:'manual',headers:{cookie,origin:base,'content-type':'application/x-www-form-urlencoded'},body:new URLSearchParams({_brain:editHash+'/saveRecord',[hiddenKey]:id,title:'No JS decimal',score:'7.25',meta:'{}'}),signal:AbortSignal.timeout(5000)})
 assert.equal(response.status,303,await response.clone().text())
 assert.equal(row.data.active,false);assert.equal(row.data.score,7.25);assert.equal(row.data.title,'No JS decimal')
 deny=true
 response=await rpc('saveRecord',[{title:'Blocked',active:false,[hiddenKey]:id}])
 assert.equal(response.status,403)
 assert.ok(!(await response.text()).includes('private-driver-detail'))
 deny=false
 response=await rpc('removeRecord',[{[hiddenKey]:id,confirmation:true}]);assert.equal(response.status,200)
 assert.equal(calls.at(-1).method,'DELETE');assert.equal(row,undefined)
 hash=createHash
 response=await rpc('saveRecord',[{title:'Native deletion',active:false,score:1.5,meta:'{}',[hiddenKey]:''}]);assert.equal(response.status,200)
 const nativeDelete=confirmation=>fetch(base+'/admin/posts/'+id+'/edit',{method:'POST',redirect:'manual',headers:{cookie,origin:base,'content-type':'application/x-www-form-urlencoded'},body:new URLSearchParams({_brain:editHash+'/removeRecord',[hiddenKey]:id,...confirmation}),signal:AbortSignal.timeout(5000)})
 response=await nativeDelete({});assert.equal(response.status,422);assert.ok(row,'Unconfirmed native delete cannot mutate')
 response=await nativeDelete({confirmation:'on'});assert.equal(response.status,303);assert.equal(row,undefined)
 assert.equal(calls.at(-1).method,'DELETE')
 console.log('Generated admin CRUD: PASS (compiled RPC, native policy, role guard, schema validation, SSR load, cookie minimization)')
}finally{await close(app);await close(native)}
