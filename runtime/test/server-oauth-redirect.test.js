// A native provider start is a narrow exception to same-site redirect protection, never a proxy-wide opt-out.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { createServer as http } from 'node:http'
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { createServer } from '../server/server.mjs'
test('only the native LinkedIn start relays its exact HTTPS authorization redirect', { timeout: 15000 }, async () => {
  const dist=mkdtempSync(join(tmpdir(),'brain-provider-redirect-'))
  const authorize='https://www.linkedin.com/oauth/v2/authorization'
  let location=authorize+'?response_type=code&client_id=synthetic-client&state=synthetic-state', status=303
  const backend=http((req,res)=>{res.writeHead(status,{location,'set-cookie':'__Host-brain-linkedin=synthetic-state; Path=/; Secure; HttpOnly; SameSite=Lax'});res.end()})
  let server, mounted
  try {
    mkdirSync(join(dist,'_server'));writeFileSync(join(dist,'_server/manifest.json'),'{"routes":[],"rpc":{}}')
    await new Promise(resolve=>backend.listen(0,'127.0.0.1',resolve))
    server=createServer({dist,backendURL:`http://127.0.0.1:${backend.address().port}`,log(){}})
    await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve))
    const origin=`http://127.0.0.1:${server.address().port}`
    const start='/_brain/auth/linkedin/start'
    for(const method of ['GET','HEAD']) {
      const response=await fetch(origin+start,{method,redirect:'manual'})
      assert.equal(response.status,303)
      assert.equal(response.headers.get('location'),location)
      assert.ok(response.headers.get('set-cookie').includes('__Host-brain-linkedin='))
      await response.arrayBuffer()
    }
    for(const candidate of [
      'http://www.linkedin.com/oauth/v2/authorization',
      'https://www.linkedin.com.attacker.example/oauth/v2/authorization',
      'https://www.linkedin.com@attacker.example/oauth/v2/authorization',
      'https://synthetic-user@www.linkedin.com/oauth/v2/authorization',
      'https://www.linkedin.com\\oauth\\v2\\authorization',
      authorize+'/other', authorize+'#fragment',
      'https://www.linkedin.com:444/oauth/v2/authorization',
    ]) {
      location=candidate
      const response=await fetch(origin+start,{redirect:'manual'})
      assert.equal(response.status,500,candidate)
      assert.equal(response.headers.get('location'),null,candidate)
      await response.arrayBuffer()
    }
    location=authorize
    for(const [path,method] of [[start,'POST'],[start+'/','GET'],['/_brain/auth/linkedin/callback','GET'],['/_brain/auth/google/start','GET'],['/_brain/auth/apple/start','GET'],['/_brain/auth/oidc/example/start','GET'],['/api/items','GET']]) {
      const response=await fetch(origin+path,{method,headers:method==='POST'?{origin}:{},redirect:'manual'})
      assert.equal(response.status,500,path+' '+method)
      assert.equal(response.headers.get('location'),null)
      await response.arrayBuffer()
    }
    status=302
    const invalidStatus=await fetch(origin+start,{redirect:'manual'})
    assert.equal(invalidStatus.status,500);await invalidStatus.arrayBuffer()
    location='/signed-in'
    const sameSite=await fetch(origin+'/_brain/auth/linkedin/callback',{redirect:'manual'})
    assert.equal(sameSite.status,302);assert.equal(sameSite.headers.get('location'),'/signed-in');await sameSite.arrayBuffer()
    status=303;location=authorize
    writeFileSync(join(dist,'_server/manifest.json'),'{"routes":[],"rpc":{},"base":"/app/"}')
    mounted=createServer({dist,backendURL:`http://127.0.0.1:${backend.address().port}`,log(){}})
    await new Promise(resolve=>mounted.listen(0,'127.0.0.1',resolve))
    const based=await fetch(`http://127.0.0.1:${mounted.address().port}/app${start}`,{redirect:'manual'})
    assert.equal(based.status,303);assert.equal(based.headers.get('location'),authorize);await based.arrayBuffer()
  } finally {
    if(mounted)await new Promise(resolve=>mounted.close(resolve))
    if(server)await new Promise(resolve=>server.close(resolve))
    await new Promise(resolve=>backend.close(resolve))
    rmSync(dist,{recursive:true,force:true})
  }
})
