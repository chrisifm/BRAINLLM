// Compiler-only plain stores must preserve the observable public store contract.
import test from 'node:test'
import assert from 'node:assert/strict'
import { install } from '../shim/dom.js'
const doc=install()
const { fx,mount,sibling }=await import('./rt.js')
const {store,plainStore,seen}=await sibling('brain-ch.js')
const tick=()=>new Promise(resolve=>setTimeout(resolve,0))

async function exercise(factory) {
  const initial={count:1,items:['one'],optional:undefined},events=[],effects=[]
  const observe=(kind,name,role,owner)=>{if(kind==='store')events.push([name,role,Boolean(owner)])}
  seen.add(observe)
  let state
  const host=doc.createElement('div')
  const stop=mount(()=>{
    state=factory(initial)
    fx(()=>effects.push([state.count,state.items.length]))
    return doc.createTextNode('Owned plain store fixture')
  },host)
  try {
    const descriptors=Object.fromEntries(['$set','$snap','$reset'].map(name=>{const d=Object.getOwnPropertyDescriptor(state,name);return [name,{enumerable:d.enumerable,configurable:d.configurable,writable:d.writable,type:typeof d.value}]}))
    state.$set({count:2,items:['one','two']});await tick()
    const snapshots=[state.$snap()]
    initial.count=7
    state.$reset();await tick();snapshots.push(state.$snap())
    stop();state.count=9;await tick()
    return {keys:Object.keys(state),descriptors,snapshots,effects,events}
  }finally{stop();seen.delete(observe)}
}
test('compiler plain store matches batching, reset, descriptors, ownership and inspector events',async()=>{
  assert.deepEqual(await exercise(init=>plainStore(init,'channels/session')),await exercise(init=>store(init,{},'channels/session')))
})
test('plain stores preserve serialization side effects and throw before reset writes',()=>{
  for(const factory of [init=>store(init),init=>plainStore(init)]){
    let encodes=0
    const initial={value:{toJSON(){encodes++;return 'serialized'}}}
    const state=factory(initial)
    assert.equal(encodes,1)
    state.$snap();assert.equal(encodes,1)
    state.$reset();assert.equal(encodes,2)
    const safe={safe:true};state.value=safe
    const circular={};circular.self=circular;initial.value=circular
    assert.throws(()=>state.$reset(),TypeError)
    assert.equal(state.value,safe,'Serialization failure must precede the write batch')
    assert.throws(()=>factory({value:circular}),TypeError)
    assert.throws(()=>factory(null),TypeError)
  }
})
test('plain helper never opens persistence or synchronization transports',()=>{
  const saved={storage:Object.getOwnPropertyDescriptor(globalThis,'localStorage'),broadcast:Object.getOwnPropertyDescriptor(globalThis,'BroadcastChannel'),socket:Object.getOwnPropertyDescriptor(globalThis,'WebSocket')}
  const fail=()=>{throw Error('A plain store cannot activate a transport')}
  try{
    Object.defineProperty(globalThis,'localStorage',{configurable:true,get:fail})
    Object.defineProperty(globalThis,'BroadcastChannel',{configurable:true,get:fail})
    Object.defineProperty(globalThis,'WebSocket',{configurable:true,get:fail})
    const state=plainStore({count:1})
    state.count=2;state.$set({count:3});state.$reset()
    assert.deepEqual(state.$snap(),{count:1})
  }finally{
    for(const [name,descriptor] of [['localStorage',saved.storage],['BroadcastChannel',saved.broadcast],['WebSocket',saved.socket]]){
      if(descriptor)Object.defineProperty(globalThis,name,descriptor)
      else delete globalThis[name]
    }
  }
})
