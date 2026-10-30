// Actual official SQLite Wasm and OPFS. Supply an integrity-verified extracted official package directory.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { createServer } from 'node:http'
import { readFile } from 'node:fs/promises'
import { join } from 'node:path'
import { launch, openPage } from '../../bench/lib/cdp.mjs'
const fixture = process.env.BRAIN_SQLITE_FIXTURE_DIR

test('official SQLite Wasm uses preserved OPFS, atomic batches and one worker owner', {skip:!fixture, timeout:60000}, async () => {
  let chrome, page
  const adapter = await readFile(new URL('../../runtime/brain-sqlite.js', import.meta.url))
  const sdk = new Map()
  for (const name of ['index.mjs','sqlite3.wasm','sqlite3-opfs-async-proxy.js']) sdk.set('/vendor/'+name, await readFile(join(fixture,'package/dist',name)))
  const worker = `import initialize from '/vendor/index.mjs'; import {sqlite} from '/adapter.js';
    const db=sqlite(initialize,{namespace:'proof',name:'records'});
    onmessage=async ({data})=>{try{const allowed=['query','execute','batch','close','info'];if(!allowed.includes(data.method))throw Error('Invalid method');const value=await db[data.method](...data.args);postMessage({id:data.id,value})}catch(e){postMessage({id:data.id,error:{code:e.code,message:e.message}})}};`
  const server = createServer((req,res) => {
    res.setHeader('Cache-Control','no-store')
    res.setHeader('Content-Type',req.url.endsWith('.wasm')?'application/wasm':req.url.endsWith('.js')||req.url.endsWith('.mjs')?'text/javascript':'text/html')
    res.end(req.url==='/adapter.js'?adapter:req.url==='/worker.js'?worker:sdk.get(req.url)||'<!doctype html><title>SQLite proof</title>')
  })
  const deadline = async (promise, ms=10000) => { let timer; try{return await Promise.race([promise,new Promise((_,reject)=>{timer=setTimeout(()=>reject(Error('SQLite browser proof deadline')),ms)})])}finally{clearTimeout(timer)} }
  try {
    await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve))
    chrome=await deadline(launch()); page=await deadline(openPage(chrome.port,`http://127.0.0.1:${server.address().port}/`))
    const result=await deadline(page.evaluate(`(async()=>{
      let seq=0; const waits=new Map(); const w=new Worker('/worker.js',{type:'module'});
      w.onmessage=({data})=>{const p=waits.get(data.id);if(!p)return;waits.delete(data.id);data.error?p.reject(data.error):p.resolve(data.value)};
      const call=(method,...args)=>new Promise((resolve,reject)=>{const id=++seq;waits.set(id,{resolve,reject});w.postMessage({id,method,args})});
      try{
        await call('execute','CREATE TABLE entries(id INTEGER PRIMARY KEY, title TEXT NOT NULL)');
        await call('batch',[{sql:'INSERT INTO entries VALUES(?,?)',bind:[1,'First']},{sql:'INSERT INTO entries VALUES(?,?)',bind:[2,'Second']}]);
        const rows=await call('query','SELECT id,title FROM entries ORDER BY id');
        const contender=new Worker('/worker.js',{type:'module'});
        let exclusive=false;
        try {exclusive=await new Promise((resolve,reject)=>{
          contender.onmessage=({data})=>resolve(data.error?.code==='B0916');contender.onerror=reject;
          contender.postMessage({id:1,method:'query',args:['SELECT 1']});
        })}finally{contender.terminate()}

        let rolledBack=false;try{await call('batch',[{sql:'INSERT INTO entries VALUES(?,?)',bind:[3,'Third']},{sql:'INSERT INTO entries VALUES(?,?)',bind:[1,'Duplicate']}])}catch{rolledBack=true}
        const count=await call('query','SELECT COUNT(*) AS n FROM entries');
        let readonly=false;try{await call('query','DELETE FROM entries')}catch(e){readonly=e.code==='B0916'}
        let multiple=false;try{await call('execute','INSERT INTO entries VALUES(?,?); DELETE FROM entries',[3,'Third'])}catch(e){multiple=e.code==='B0916'}
        await call('close');w.terminate();
        const next=new Worker('/worker.js',{type:'module'});next.onmessage=({data})=>{const p=waits.get(data.id);if(!p)return;waits.delete(data.id);data.error?p.reject(data.error):p.resolve(data.value)};
        const reopened=(method,...args)=>new Promise((resolve,reject)=>{const id=++seq;waits.set(id,{resolve,reject});next.postMessage({id,method,args})});
        try{const persisted=await reopened('query','SELECT COUNT(*) AS n FROM entries');await reopened('close');return {rows,count,rolledBack,readonly,multiple,persisted,exclusive}}finally{next.terminate()}
      }finally{w.terminate()}
    })().catch(e=>{throw Error(JSON.stringify(e))})`),25000)
    assert.deepEqual(result.rows,[{id:1,title:'First'},{id:2,title:'Second'}])
    assert.equal(result.count[0].n,2); assert.equal(result.persisted[0].n,2)
    assert.equal(result.exclusive,true);assert.equal(result.rolledBack,true);assert.equal(result.readonly,true);assert.equal(result.multiple,true)
  } finally {page?.close();chrome?.close();await new Promise(resolve=>server.close(resolve))}
})
