import {test,after} from 'node:test'
import assert from 'node:assert/strict'
import {createServer as http} from 'node:http'
import {mkdtempSync,mkdirSync,writeFileSync,rmSync,copyFileSync} from 'node:fs'
import {tmpdir} from 'node:os'
import {join} from 'node:path'
import {createServer} from '../server/server.mjs'
const dist=mkdtempSync(join(tmpdir(),'brain-login-bridge-'));mkdirSync(join(dist,'_server'))
writeFileSync(join(dist,'package.json'),'{"type":"module"}')
for(const name of ['brain.js','brain-form.js','brain-schema.js'])copyFileSync(new URL('../'+name,import.meta.url),join(dist,'_server',name))
const user={id:'00000000-0000-4000-8000-000000000001',email:'native@example.test',role:'editor',verified:true}
const seen=[];let mode='jwt',frontendBase
const cookies=['__Host-brain-access=synthetic-access; Path=/; Secure; HttpOnly; SameSite=Lax; Max-Age=300','__Host-brain-refresh=synthetic-refresh; Path=/; Secure; HttpOnly; SameSite=Lax; Max-Age=604800']
const backend=http((req,res)=>{
 const chunks=[];req.on('data',chunk=>chunks.push(chunk));req.on('end',()=>{
  seen.push({path:req.url,headers:req.headers,body:Buffer.concat(chunks).toString()})
  if(mode==='captcha-failed'){res.writeHead(403,{'content-type':'application/json'});res.end('{"error":{"code":"CAPTCHA_FAILED"}}');return}
  if(mode==='captcha-down'){res.writeHead(503,{'content-type':'application/json'});res.end('{"error":{"code":"CAPTCHA_UNAVAILABLE"}}');return}
  if(mode==='grant'&&req.url==='/_brain/auth/signup'){res.writeHead(200,{'content-type':'application/json'});res.end(JSON.stringify({user,login_grant:'synthetic-login-grant'}));return}
  if(mode==='taken'){res.writeHead(422,{'content-type':'application/json'});res.end('{"error":{"code":"SIGNUP_FAILED"}}');return}
  if(mode==='unavailable'){res.writeHead(503,{'content-type':'application/json'});res.end('{"error":"private driver detail"}');return}
  const set=mode==='opaque'?['__Host-brain=synthetic-opaque; Path=/; Secure; HttpOnly; SameSite=Lax; Max-Age=604800']:mode==='insecure'?['__Host-brain-access=bad; Path=/']:cookies
  res.writeHead(200,{'content-type':'application/json','set-cookie':set})
  res.end(JSON.stringify({user:{...user,password_hash:'private-hash'},access_token:'private-access-marker',refresh_token:'private-refresh-marker'}))
 })
})
await new Promise(resolve=>backend.listen(0,'127.0.0.1',resolve))
writeFileSync(join(dist,'_server/account.mjs'),`export async function signup(ctx,values){return ctx.backend.signup(values)};export async function guarded(ctx,values,token){return ctx.backend.login(values,{captcha:token})};export async function other(ctx,values){await ctx.backend.signup(values);return ctx.backend.login({...values,email:'other@example.test'})};export async function join(ctx,values){await ctx.backend.signup(values);return ctx.backend.login(values)};export async function login(ctx,values){return {...await ctx.backend.login(values),redirect:'/account'}};export const $forms={login:{email:'email!',password:'str!'}};export const $server=['signup','guarded','other','join','login']`)
writeFileSync(join(dist,'_server/manifest.json'),JSON.stringify({base:'/app/',routes:[{path:'/login',forms:true}],rpc:{account:'_server/account.mjs'}}))
const frontend=createServer({dist,backendURL:`http://127.0.0.1:${backend.address().port}`,log(){}})
await new Promise(resolve=>frontend.listen(0,'127.0.0.1',resolve));frontendBase=`http://127.0.0.1:${frontend.address().port}`
after(async()=>{for(const server of [frontend,backend])await new Promise(resolve=>{server.close(resolve);server.closeAllConnections()});rmSync(dist,{recursive:true,force:true})})
const credentials={email:user.email,password:'synthetic-native-password'}
const rpc=extra=>fetch(frontendBase+'/app/_r/account/login',{method:'POST',headers:{origin:frontendBase,'x-brain-rpc':'1','content-type':'application/json',cookie:'unrelated=never-forward',authorization:'Bearer never-adopt',...extra},body:JSON.stringify([credentials]),redirect:'manual'})
test('one server RPC relays secure native JWT/opaque cookies and returns only verified public user fields',async()=>{
 for(const selected of ['jwt','opaque']){
  mode=selected;const before=seen.length,response=await rpc({'x-request-id':'native-login-reference'})
  assert.equal(response.status,200);assert.deepEqual(await response.json(),{user,redirect:'/account'})
  assert.equal(response.headers.getSetCookie().length,selected==='jwt'?2:1)
  assert.equal(seen.length,before+1);const request=seen.at(-1)
  assert.equal(request.path,'/_brain/auth/login');assert.deepEqual(JSON.parse(request.body),credentials)
  assert.equal(request.headers.origin,frontendBase);assert.equal(request.headers.authorization,undefined);assert.equal(request.headers.cookie,undefined)
  assert.equal(request.headers['x-request-id'],'native-login-reference');assert.match(response.headers.get('cache-control'),/no-store/)
 }
})
test('progressive form POST uses the same native login and returns a prefixed 303 with HttpOnly cookies',async()=>{
 mode='jwt'
 const response=await fetch(frontendBase+'/app/login',{method:'POST',headers:{origin:frontendBase,'content-type':'application/x-www-form-urlencoded'},body:new URLSearchParams({_brain:'account/login',...credentials}),redirect:'manual'})
 assert.equal(response.status,303);assert.equal(response.headers.get('location'),'/app/account');assert.equal(response.headers.getSetCookie().length,2)
})
test('native login fails closed before issuing cookies or exposing provider messages',async()=>{
 for(const failure of ['unavailable','insecure']){
  mode=failure;const response=await rpc();assert.equal(response.status,503);assert.equal(response.headers.get('set-cookie'),null)
  const body=await response.text();assert.ok(!body.includes('private')&&!body.includes('synthetic-native-password'))
 }
 const before=seen.length,response=await rpc({origin:'https://foreign.example'})
 assert.equal(response.status,403);assert.equal(seen.length,before)
})

const call=(fn,body)=>fetch(frontendBase+'/app/_r/account/'+fn,{method:'POST',headers:{origin:frontendBase,'x-brain-rpc':'1','content-type':'application/json'},body:JSON.stringify([body])})
test('ctx.backend.signup creates the account without a session, and login in the same request signs in',async()=>{
 mode='jwt'
 const before=seen.length,created=await call('signup',credentials)
 assert.equal(created.status,200);assert.deepEqual(await created.json(),{user});assert.equal(created.headers.getSetCookie().length,0)
 assert.equal(seen[before].path,'/_brain/auth/signup');assert.deepEqual(JSON.parse(seen[before].body),credentials);assert.equal(seen[before].headers.origin,frontendBase)
 const joined=await call('join',credentials)
 assert.equal(joined.status,200);assert.equal(joined.headers.getSetCookie().length,2)
})
test('ctx.backend.signup names the field: a refused address and a short password never reach a generic error',async()=>{
 mode='taken'
 const refused=await call('signup',credentials)
 assert.equal(refused.status,422);assert.deepEqual((await refused.json()).errors,{email:'This email cannot be used'})
 const before=seen.length,short=await call('signup',{...credentials,password:'short'})
 assert.equal(short.status,422);assert.ok((await short.json()).errors.password);assert.equal(seen.length,before)
 mode='jwt'
})

test('the captcha token reaches the backend from a server function or the page header, and a refusal is named',async()=>{
 mode='jwt'
 const send=(fn,args,headers={})=>fetch(frontendBase+'/app/_r/account/'+fn,{method:'POST',headers:{origin:frontendBase,'x-brain-rpc':'1','content-type':'application/json',...headers},body:JSON.stringify(args)})
 let response=await send('guarded',[credentials,'synthetic-server-token']);assert.equal(response.status,200);assert.equal(seen.at(-1).headers['x-brain-captcha'],'synthetic-server-token')
 response=await send('signup',[credentials],{'x-brain-captcha':'synthetic-page-token'});assert.equal(response.status,200);assert.equal(seen.at(-1).headers['x-brain-captcha'],'synthetic-page-token')
 const before=seen.length;response=await send('guarded',[credentials,'bad\ntoken']);assert.equal(response.status,403);assert.equal(seen.length,before,'a malformed token never leaves the server')
 mode='captcha-failed';response=await send('guarded',[credentials,'synthetic-server-token']);assert.equal(response.status,403);let body=await response.text();assert.ok(body.includes('captcha')&&!body.includes('B0740'),body)
 response=await send('signup',[credentials]);assert.equal(response.status,403);body=await response.text();assert.ok(body.includes('captcha')&&!body.includes('B0740'),body)
 mode='captcha-down';response=await send('guarded',[credentials,'synthetic-server-token']);assert.equal(response.status,503);assert.equal(response.headers.get('set-cookie'),null)
})
test('a signup grant signs in that same address once and stays out of the page',async()=>{
 mode='grant'
 const send=(fn,args)=>fetch(frontendBase+'/app/_r/account/'+fn,{method:'POST',headers:{origin:frontendBase,'x-brain-rpc':'1','content-type':'application/json'},body:JSON.stringify(args)})
 let response=await send('join',[credentials]);assert.equal(response.status,200);assert.equal(seen.at(-1).path,'/_brain/auth/login');assert.equal(seen.at(-1).headers['x-brain-login-grant'],'synthetic-login-grant')
 assert.ok(!(await response.text()).includes('synthetic-login-grant'))
 response=await send('signup',[credentials]);assert.ok(!(await response.text()).includes('synthetic-login-grant'))
 response=await send('other',[credentials]);assert.equal(seen.at(-1).path,'/_brain/auth/login');assert.equal(seen.at(-1).headers['x-brain-login-grant'],undefined,'a grant never travels with another address')
})
test('the backend proxy passes the captcha token and never an operator key',async()=>{
 mode='jwt'
 await fetch(frontendBase+'/app/_brain/auth/signup',{method:'POST',headers:{origin:frontendBase,'content-type':'application/json','x-brain-captcha':'synthetic-page-token','x-brain-operator-key':'synthetic-operator-key'},body:JSON.stringify(credentials)})
 assert.equal(seen.at(-1).headers['x-brain-captcha'],'synthetic-page-token');assert.equal(seen.at(-1).headers['x-brain-operator-key'],undefined)
})
