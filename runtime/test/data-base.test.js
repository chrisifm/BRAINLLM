import {test} from 'node:test'
import assert from 'node:assert/strict'
import {install} from '../shim/dom.js'
const document=install(),{sibling}=await import('./rt.js'),{api,http}=await sibling('brain-data.js')
const meta=document.createElement('meta');meta.setAttribute('name','brain-base');meta.setAttribute('content','/app/');document.head.appendChild(meta)
const requests=[]
globalThis.fetch=async(url,options)=>{requests.push({url,body:options.body});return new Response(JSON.stringify({ok:true}),{headers:{'content-type':'application/json','cache-control':'no-store'}})}

test('app API paths respect hosting prefixes and foreign API bases remain absolute',async()=>{
  http({base:'',retry:0})
  await api('/records',{get:{}}).get();assert.equal(requests.at(-1).url,'/app/records')
  http({base:'https://api.example.test/v1',retry:0})
  await api('/records',{get:{}}).get();assert.equal(requests.at(-1).url,'https://api.example.test/v1/records')
  http({base:'//api.example.test/v2',retry:0})
  await api('/records',{get:{}}).get();assert.equal(requests.at(-1).url,'//api.example.test/v2/records')
})
test('native account endpoints cannot inherit an external API base',async()=>{
  http({base:'https://api.example.test/v1',retry:0})
  await api('/_brain/auth/password/request',{post:{}}).post({email:'synthetic@example.test'})
  assert.equal(requests.at(-1).url,'/app/_brain/auth/password/request')
})
