import {test} from 'node:test'
import assert from 'node:assert/strict'
const rt = await import('./rt.js')
const T = await rt.sibling('brain-threads.js')
const W = await rt.sibling('brain-worker.js')

test('capability queries and experimental worklets never imply unavailable browser support', async()=>{
  assert.equal(typeof T.threadCapabilities().wasm,'boolean')
  assert.equal(T.threadCapabilities().sharedMemory,false)
  await assert.rejects(T.loadWorklet('layout','/layout.js'),e=>e.code==='B0917')
  assert.equal(await T.loadWorklet('layout','/layout.js',{fallback:()=> 'native layout'}),'native layout')
  assert.throws(()=>T.sharedMemory(1024),e=>e.code==='B0917')
  assert.throws(()=>T.offscreen({}),e=>e.code==='B0917')
})
test('worklet loading uses explicit supplied audio context and rejects foreign module URLs',async()=>{
  globalThis.location=new URL('https://example.test/app/')
  const loads=[]
  const context={audioWorklet:{async addModule(...args){loads.push(args)}}}
  assert.deepEqual(await T.loadWorklet('audio','/audio.js',{context}),{available:true,kind:'audio'})
  assert.deepEqual(loads,[['https://example.test/audio.js',{credentials:'omit'}]])
  await assert.rejects(T.loadWorklet('audio','https://other.test/audio.js',{context}),e=>e.code==='B0917')
  delete globalThis.location
})
test('shared server ports preserve an exported RPC map and response transfer ownership',async()=>{
  const scope={onconnect:null}, replies=[], buffer=new ArrayBuffer(16)
  W.serve({buffer:()=>W.transfer(buffer,[buffer])},scope)
  let started=false
  const port={start(){started=true},postMessage(message,list){replies.push({message,list})}}
  scope.onconnect({ports:[port]})
  await port.onmessage({data:{id:1,name:'buffer',args:[]}})
  assert.equal(started,true)
  assert.equal(replies[0].message.ok,buffer)
  assert.deepEqual(replies[0].list,[buffer])
  assert.throws(()=>W.transfer(buffer,[buffer,buffer]),/distinct/)
})
test('message channels close both owned ports and shared memory remains bounded',()=>{
  const channel=T.messageChannel()
  assert.ok(channel.port1&&channel.port2)
  channel.close()
  globalThis.crossOriginIsolated=true
  assert.equal(T.sharedMemory(16).byteLength,16)
  assert.throws(()=>T.sharedMemory(8388609),e=>e.code==='B0917')
  delete globalThis.crossOriginIsolated
})
