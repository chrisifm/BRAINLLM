// Raw duplicate fields must not disappear through Node's req.headers normalization on a declared native route.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { createServer as http, request } from 'node:http'
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { createServer } from '../server/server.mjs'

test('declared single-value header ambiguity is rejected before forwarding without leaking capacity or combining legitimate lists',{timeout:15000},async()=>{
  const directory=mkdtempSync(join(tmpdir(),'brain-native-header-ambiguity-'))
  mkdirSync(join(directory,'_server'))
  writeFileSync(join(directory,'_server/manifest.json'),JSON.stringify({routes:[],rpc:{},nativeRoutes:[{path:'/events/verified',headers:['content-type','x-event-signature']},{path:'/events/verified-list',headers:['accept']}]}))
  const seen=[]
  const backend=http((req,res)=>{
    const chunks=[];req.on('data',chunk=>chunks.push(chunk));req.on('end',()=>{
      seen.push({headers:req.headers,rawHeaders:req.rawHeaders,body:Buffer.concat(chunks)})
      res.writeHead(200,{'Content-Type':'application/json'});res.end('{"ok":true}')
    })
  })
  let frontend
  try {
    await new Promise(resolve=>backend.listen(0,'127.0.0.1',resolve))
    frontend=createServer({dist:directory,backendURL:`http://127.0.0.1:${backend.address().port}`,log(){}})
    await new Promise(resolve=>frontend.listen(0,'127.0.0.1',resolve))
    const payload=Buffer.from([0,255,13,10,1])
    const call=(extra,path='/events/verified',accept=['application/json','text/plain'])=>new Promise((resolve,reject)=>{
      const req=request({host:'127.0.0.1',port:frontend.address().port,path,method:'POST',headers:[
        'Host',`127.0.0.1:${frontend.address().port}`,'Content-Length',String(payload.length),'Content-Type','application/octet-stream','x-event-signature','signature-one',
        ...accept.flatMap(value=>['Accept',value]),'Cookie','__Host-brain=synthetic-access','Cookie','__Host-brain-refresh=synthetic-refresh',...extra,
      ]},res=>{res.resume();res.once('end',()=>resolve(res.statusCode))})
      req.setTimeout(3000,()=>req.destroy(new Error('Owned raw-header request exceeded its deadline')))
      req.once('error',reject);req.end(payload)
    })
    assert.equal(await call([]),200)
    assert.deepEqual(seen[0].body,payload)
    assert.equal(seen[0].headers.accept,'application/json, text/plain')
    assert.equal(seen[0].headers.cookie,'__Host-brain=synthetic-access; __Host-brain-refresh=synthetic-refresh')
    const before=seen.length
    assert.equal(await call([],'/events/verified-list'),400,'a list-valued field explicitly declared for verification must also appear exactly once')
    assert.equal(seen.length,before)
    assert.equal(await call([],'/events/verified-list',['application/json, text/plain']),200,'a single declared header may contain its ordinary list syntax')
    const afterDeclaredList=seen.length
    for(let attempt=0;attempt<9;attempt++) {
      const duplicate=attempt%2===0?['CONTENT-TYPE','text/plain']:['X-Event-Signature','signature-two']
      assert.equal(await call(duplicate),400,'each genuine duplicate is rejected rather than hidden by req.headers')
      assert.equal(seen.length,afterDeclaredList,'no ambiguous request may reach the backend')
    }
    for(const duplicate of [['Authorization','Bearer first','authorization','Bearer second'],['X-API-Key','first-key','x-api-key','second-key']]) {
      assert.equal(await call(duplicate,'/api/items'),400,'ambiguous credentials are rejected even without a declared route')
      assert.equal(seen.length,afterDeclaredList)
    }
    assert.equal(await call([]),200,'more than eight refusals never consume the bridge admission slots')
    assert.equal(seen.length,afterDeclaredList+1)
    const types=seen.at(-1).rawHeaders.filter((_,index)=>index%2===0).filter(name=>name.toLowerCase()==='content-type')
    assert.equal(types.length,1,'common and declared allowlist entries do not manufacture duplicates')
  } finally {
    if(frontend)await new Promise(resolve=>{frontend.close(resolve);frontend.closeAllConnections()})
    await new Promise(resolve=>{backend.close(resolve);backend.closeAllConnections()})
    rmSync(directory,{recursive:true,force:true})
  }
})
