import {test,after} from 'node:test'
import assert from 'node:assert/strict'
import {createServer as http} from 'node:http'
import {mkdtempSync,mkdirSync,writeFileSync,rmSync,copyFileSync} from 'node:fs'
import {tmpdir} from 'node:os'
import {join} from 'node:path'
import {createServer} from '../server/server.mjs'
const dist=mkdtempSync(join(tmpdir(),'brain-google-bridge-'));mkdirSync(join(dist,'_server'))
writeFileSync(join(dist,'package.json'),'{"type":"module"}')
for(const name of ['brain.js','brain-form.js','brain-schema.js'])copyFileSync(new URL('../'+name,import.meta.url),join(dist,'_server',name))
const user={id:'00000000-0000-4000-8000-000000000002',email:'google@example.test',role:'user',verified:true}
const nonce='a'.repeat(64),flags='Path=/; Secure; HttpOnly; SameSite=Lax'
const seen=[];let status=200
const backend=http((req,res)=>{
 const chunks=[];req.on('data',chunk=>chunks.push(chunk));req.on('end',()=>{
  seen.push({path:req.url,headers:req.headers,body:Buffer.concat(chunks).toString()})
  if(status!==200){res.writeHead(status,{'content-type':'application/json'});res.end('{"error":"private provider detail"}');return}
  if(req.url==='/_brain/auth/google/challenge'){res.writeHead(200,{'content-type':'application/json','set-cookie':[`__Host-brain-google=${nonce}; ${flags}; Max-Age=300`]});res.end(JSON.stringify({client_id:'synthetic.apps.googleusercontent.com',nonce,expires_in:300}));return}
  res.writeHead(200,{'content-type':'application/json','set-cookie':[`__Host-brain-access=synthetic-access; ${flags}; Max-Age=300`,`__Host-brain-refresh=synthetic-refresh; ${flags}; Max-Age=600`,`__Host-brain-google=; ${flags}; Max-Age=0`]})
  res.end(JSON.stringify({user:{...user,password_hash:'private-hash'},access_token:'private-access-marker',refresh_token:'private-refresh-marker'}))
 })
})
await new Promise(resolve=>backend.listen(0,'127.0.0.1',resolve))
writeFileSync(join(dist,'_server/account.mjs'),`export async function challenge(ctx){return ctx.backend.google.challenge()};export async function google(ctx,values){return ctx.backend.google.login(values)};export const $forms=[];`)
writeFileSync(join(dist,'_server/manifest.json'),JSON.stringify({base:'/',routes:[],rpc:{account:'_server/account.mjs'}}))
const frontend=createServer({dist,backendURL:`http://127.0.0.1:${backend.address().port}`,log(){}})
await new Promise(resolve=>frontend.listen(0,'127.0.0.1',resolve));const base=`http://127.0.0.1:${frontend.address().port}`
after(async()=>{for(const server of [frontend,backend])await new Promise(resolve=>{server.close(resolve);server.closeAllConnections()});rmSync(dist,{recursive:true,force:true})})
const rpc=(name,body,cookie='')=>fetch(`${base}/_r/account/${name}`,{method:'POST',headers:{origin:base,'x-brain-rpc':'1','content-type':'application/json',...(cookie?{cookie}:{})},body:JSON.stringify(body)})
test('the challenge binds its nonce to this browser with the HttpOnly cookie and hands the page only the public values',async()=>{
 status=200
 const response=await rpc('challenge',[])
 assert.equal(response.status,200)
 assert.deepEqual(await response.json(),{client_id:'synthetic.apps.googleusercontent.com',nonce,expires_in:300})
 const set=response.headers.getSetCookie()
 assert.equal(set.length,1);assert.match(set[0],/^__Host-brain-google=/);assert.match(set[0],/HttpOnly/)
 assert.match(response.headers.get('cache-control'),/no-store/)
 assert.equal(seen.at(-1).path,'/_brain/auth/google/challenge');assert.equal(seen.at(-1).headers.origin,base)
})
test('the login sends the nonce cookie of this browser only, relays the session and clears the nonce',async()=>{
 status=200
 const response=await rpc('google',[{credential:'synthetic.id.token'}],`__Host-brain-google=${nonce}; unrelated=private-cookie`)
 assert.equal(response.status,200);assert.deepEqual(await response.json(),{user})
 const request=seen.at(-1)
 assert.equal(request.path,'/_brain/auth/google');assert.deepEqual(JSON.parse(request.body),{credential:'synthetic.id.token'})
 assert.equal(request.headers.cookie,`__Host-brain-google=${nonce}`)
 assert.equal(response.headers.getSetCookie().length,3)
})
test('no nonce cookie, a malformed one, or a bad credential shape never reaches the backend',async()=>{
 status=200;const before=seen.length
 for(const [body,cookie] of [[{credential:'x'},''],[{credential:'x'},'__Host-brain-google=not-a-nonce'],[{credential:'x',email:'a'},`__Host-brain-google=${nonce}`],[{credential:'x'.repeat(16385)},`__Host-brain-google=${nonce}`]]){
  const response=await rpc('google',[body],cookie);assert.ok(response.status===401||response.status===422,String(response.status));assert.equal(response.headers.get('set-cookie'),null)
 }
 assert.equal(seen.length,before)
})
test('a backend refusal is a generic denial and an outage stays an outage, without provider text',async()=>{
 for(const [from,to] of [[401,401],[403,401],[429,429],[503,503]]){
  status=from;const response=await rpc('google',[{credential:'synthetic'}],`__Host-brain-google=${nonce}`)
  assert.equal(response.status,to);assert.equal(response.headers.get('set-cookie'),null);assert.ok(!(await response.text()).includes('private'))
 }
 status=503;const response=await rpc('challenge',[]);assert.equal(response.status,503)
})
