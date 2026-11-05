// Fixed framework causes are useful diagnostics; their allowlist is independent of backend availability.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { mkdtempSync,mkdirSync,writeFileSync,rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { createServer } from '../server/server.mjs'
test('owned causes survive privately while redirect failures are not backend outages',async()=>{
  const dist=mkdtempSync(join(tmpdir(),'brain-owned-causes-')),logs=[]
  const messages=['Invalid same-site redirect location','Invalid redirect status','Invalid final response status','Invalid configured redirect location','Native backend is unavailable','Native authentication response is unavailable','Static file validators are busy; retry shortly','Static file validator deadline exceeded; retry shortly','Static file changed; retry shortly','Static file changed while computing its validator']
  mkdirSync(join(dist,'_server'))
  writeFileSync(join(dist,'_server/manifest.json'),JSON.stringify({routes:[{path:'/cause/:id',rx:'^/cause/(?<id>\\d+)$',server:'_server/cause.mjs'}],redirects:{exact:{'/bad':['//private.example/secret',302],'/status':['/home',200]}}}))
  writeFileSync(join(dist,'_server/cause.mjs'),`import { HttpError } from ${JSON.stringify(new URL('../server/server.mjs',import.meta.url).href)};const messages=${JSON.stringify([...messages,'private application detail','Native authentication is unavailable: private application detail'])};export function load(ctx){throw new HttpError(500,messages[Number(ctx.params.id)])}`)
  const server=createServer({dist,log:(level,message,details)=>logs.push({level,message,...details})})
  try{
    await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve))
    const base=`http://127.0.0.1:${server.address().port}`
    for(let i=0;i<messages.length+2;i++){
      logs.length=0
      const reply=await fetch(base+'/cause/'+i,{headers:{accept:'application/json'}})
      assert.equal(reply.status,500)
      const body=await reply.text();assert.ok(!body.includes(messages[i]??'private application detail'))
      if(i<messages.length)assert.ok(logs.some(row=>row.cause===messages[i]),JSON.stringify(logs))
      else assert.ok(!JSON.stringify(logs).includes('private application detail'))
    }
    logs.length=0
    const reply=await fetch(base+'/bad',{redirect:'manual'})
    assert.equal(reply.status,500);await reply.text()
    assert.ok(logs.some(row=>row.message==='unhandled'&&row.cause==='Invalid same-site redirect location'))
    assert.ok(!logs.some(row=>row.message==='backend unavailable'))
    assert.ok(!JSON.stringify(logs).includes('private.example'))
  }finally{await new Promise(resolve=>{server.close(resolve);server.closeAllConnections()});rmSync(dist,{recursive:true,force:true})}
})
