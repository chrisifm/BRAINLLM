import assert from 'node:assert/strict'
import { install } from './dom.js'
const doc = install()
globalThis.location = {protocol:'https:',origin:'https://app.example.test'}
class Socket {
  static instances=[]
  constructor(url,protocol){this.url=url;this.protocol=protocol;this.readyState=0;this.bufferedAmount=0;Socket.instances.push(this)}
  send(text){assert.ok(['join','leave','ping'].includes(JSON.parse(text).type))}
  close(){this.readyState=3}
  message(value){this.onmessage?.({data:JSON.stringify(value)})}
}
globalThis.WebSocket=Socket
const {mount}=await import('./runtime/brain.js')
const {_liveStats}=await import('./runtime/brain-live.js')
const wait=ms=>new Promise(resolve=>setTimeout(resolve,ms))
for(const binding of ['const','let']){
  const {default:Component}=await import('./'+binding+'.mjs')
  const host=doc.createElement('div'),stop=mount(Component,host)
  try{
    await wait(280)
    const socket=Socket.instances.at(-1)
    assert.equal(socket.url,'wss://app.example.test/_brain/socket')
    assert.match(host.textContent,/connecting:false/)
    socket.readyState=1;socket.onopen({})
    socket.message({type:'ready',protocol:1,durable:false,resyncRequired:true})
    await wait(10);assert.match(host.textContent,/connecting:false/)
    socket.message({type:'resync',reason:'joined'})
    await wait(130);assert.match(host.textContent,/open:true/)
    socket.onclose({code:1008,reason:'Authentication expired'})
    await wait(10);assert.match(host.textContent,/auth-required:false/)
  }finally{stop()}
  await wait(5)
  assert.equal(_liveStats().owners,0);assert.equal(_liveStats().connection,0);assert.equal(_liveStats().timers,0)
}
console.log('Live compiled DOM: PASS (const/let/derived status, JOIN acknowledgement, auth closure, owner cleanup)')
