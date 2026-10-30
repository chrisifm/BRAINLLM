// Actual native modal/history behavior in an owned disposable Chrome profile.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { createServer } from 'node:http'
import { readFile } from 'node:fs/promises'
import { launch, openPage } from '../../bench/lib/cdp.mjs'

test('public selection and native dialog restore direct links and back/forward without a document fetch', {timeout:60000}, async () => {
  let chrome, page, documents=0
  const files=Object.fromEntries(await Promise.all(['brain.js','brain-url.js'].map(async name=>[name,await readFile(new URL('../../runtime/'+name,import.meta.url))])))
  const server=createServer((req,res)=>{
    const name=req.url.split('?')[0].slice(1)
    res.setHeader('Cache-Control','no-store')
    res.setHeader('Content-Type',files[name]?'text/javascript':'text/html')
    if (req.headers['sec-fetch-dest']==='document') documents++
    res.end(files[name]||'<!doctype html><title>Public UI state proof</title><main id="app"></main>')
  })
  const deadline=async promise=>{let timer;try{return await Promise.race([promise,new Promise((_,reject)=>{timer=setTimeout(()=>reject(Error('UI state proof deadline')),10000)})])}finally{clearTimeout(timer)}}
  try {
    await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve))
    chrome=await deadline(launch())
    page=await deadline(openPage(chrome.port,`http://127.0.0.1:${server.address().port}/`))
    await deadline(page.send("Page.navigate",{url:`http://127.0.0.1:${server.address().port}/?campaign=keep&tab=details&modal=true`}))
    await deadline(page.evaluate("new Promise(r=>document.readyState==='complete'?r():addEventListener('load',r,{once:true}))"))
    const baselineDocuments=documents
    const direct=await deadline(page.evaluate(`(async()=>{
      const {mount}=await import('/brain.js'),{urlState,urlDialog}=await import('/brain-url.js');
      globalThis.proof={};
      proof.stop=mount(()=>{
        proof.tab=urlState('overview',{key:'tab'});
        const dialog=document.createElement('dialog');dialog.innerHTML='<button autofocus>Close</button>';
        proof.dialog=dialog;urlDialog(dialog,'modal');return dialog;
      },document.querySelector('#app'));
      await new Promise(r=>setTimeout(r,30));
      return {tab:proof.tab.v,open:proof.dialog.open,modal:proof.dialog.matches(':modal')};
    })()`))
    assert.deepEqual(direct,{tab:'details',open:true,modal:true})
    const changed=await deadline(page.evaluate(`(async()=>{
      proof.dialog.close();await new Promise(r=>setTimeout(r,30));
      proof.tab.v='reviews';await new Promise(r=>setTimeout(r,30));
      return {tab:new URL(location.href).searchParams.get('tab'),modal:new URL(location.href).searchParams.has('modal'),campaign:new URL(location.href).searchParams.get('campaign')};
    })()`))
    assert.deepEqual(changed,{tab:'reviews',modal:false,campaign:'keep'})
    const back=await deadline(page.evaluate(`(async()=>{
      await new Promise(r=>{addEventListener('popstate',r,{once:true});history.back()});await new Promise(r=>setTimeout(r,30));
      const first={tab:proof.tab.v,open:proof.dialog.open};
      await new Promise(r=>{addEventListener('popstate',r,{once:true});history.back()});await new Promise(r=>setTimeout(r,30));
      return {first,second:{tab:proof.tab.v,open:proof.dialog.open,modal:proof.dialog.matches(':modal')}};
    })()`))
    assert.deepEqual(back,{first:{tab:'details',open:false},second:{tab:'details',open:true,modal:true}})
    const forward=await deadline(page.evaluate(`(async()=>{
      await new Promise(r=>{addEventListener('popstate',r,{once:true});history.forward()});await new Promise(r=>setTimeout(r,30));
      const state={tab:proof.tab.v,open:proof.dialog.open};proof.stop();return state;
    })()`))
    assert.deepEqual(forward,{tab:'details',open:false})
    // Browser favicon requests are not document navigations.
    assert.equal(documents,baselineDocuments,"UI-only history must not fetch another document")
  } finally {page?.close();chrome?.close();await new Promise(resolve=>server.close(resolve))}
})
