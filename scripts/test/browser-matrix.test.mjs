// Complementary cross-engine proof; Viewer remains the primary installed browser integration.
import {test} from 'node:test'
import assert from 'node:assert/strict'
import {createServer} from 'node:http'
import {readFile} from 'node:fs/promises'
import {pathToFileURL} from 'node:url'
const modulePath=process.env.BRAIN_PLAYWRIGHT_MODULE
const html=`<!doctype html><meta charset="utf-8"><title>Native browser matrix</title><main>
<div id="counter"></div><div id="kit"></div><div id="public"></div></main>
<script type="module">
import {mount} from '/brain.js';import Counter from '/counter.brain.js';import Form from '/kit/form.brain.js';import {urlState,urlDialog} from '/brain-url.js';
const stopCounter=mount(Counter,document.querySelector('#counter'),{label:'Count',start:1});
const stopKit=mount(Form,document.querySelector('#kit'));
const stopPublic=mount(()=>{
 window.matrixTab=urlState('overview',{key:'matrix-tab'});
 const host=document.createElement('section');host.innerHTML='<button id="reviews">Choose reviews</button><dialog id="dialog"><button id="close-dialog">Close dialog</button></dialog>';
 host.querySelector('#reviews').onclick=()=>{window.matrixTab.v='reviews'};
 const dialog=host.querySelector('dialog');host.querySelector('#close-dialog').onclick=()=>dialog.close();urlDialog(dialog,'matrix-modal');return host;
},document.querySelector('#public'));
window.saved=false;const observer=new MutationObserver(()=>{if(document.body.textContent.includes('Saved person@example.test'))window.saved=true});observer.observe(document.body,{subtree:true,childList:true,characterData:true});
window.matrixStop=()=>{observer.disconnect();stopPublic();stopKit();stopCounter()};window.matrixReady=true;
</script>`
for(const engine of ['chromium','firefox','webkit']) test(`compiled counter/form and shareable native state work in ${engine}`,{skip:!modulePath,timeout:90000},async()=>{
 const playwright=await import(pathToFileURL(modulePath).href)
 const errors=[];let browser,context,documents=0
 const server=createServer(async(req,res)=>{
  try{
   const path=new URL(req.url,'http://localhost').pathname
   let content
   if(path==='/') {content=html;documents++;res.setHeader('content-type','text/html')}
   else {
    let file
    if(path==='/counter.brain.js')file=new URL('../../runtime/test/fixtures/compiled/counter.brain.js',import.meta.url)
    else if(/^\/kit\/(?:ui\/)?[a-z-]+(?:\.jsbrain)?\.brain\.js$/.test(path)||path==='/kit/toast.jsbrain.js')file=new URL('../../runtime/test/fixtures/kit/out/'+path.slice(5),import.meta.url)
    else if(/^\/(?:brain|brain-[a-z-]+)\.js$/.test(path))file=new URL('../../runtime/'+path.slice(1),import.meta.url)
    else {res.writeHead(404);res.end();return}
    content=(await readFile(file,'utf8')).replaceAll('__RT__','');res.setHeader('content-type','text/javascript')
   }
   res.setHeader('cache-control','no-store');res.end(content)
  }catch{res.writeHead(404);res.end()}
 })
 await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve))
 try{
  browser=await playwright[engine].launch({headless:true,timeout:30000})
  context=await browser.newContext();const page=await context.newPage();page.setDefaultTimeout(10000)
  page.on('pageerror',error=>errors.push(error.name))
  await page.goto(`http://127.0.0.1:${server.address().port}/?campaign=keep&matrix-modal=true`)
  await page.waitForFunction(()=>window.matrixReady&&document.querySelector('#dialog').open)
  assert.equal(await page.locator('#dialog').evaluate(node=>node.matches(':modal')),true)
  await page.locator('#close-dialog').click();await page.waitForFunction(()=>!new URL(location.href).searchParams.has('matrix-modal'))
  await page.locator('#counter button').click();assert.match(await page.locator('#counter').textContent(),/Count: 2/)
  await page.locator('form button[type=submit]').click();await page.waitForFunction(()=>document.querySelector('input[name=email]').getAttribute('aria-invalid')==='true')
  await page.locator('input[name=email]').fill('person@example.test');await page.locator('select[name=plan]').selectOption('Pro');await page.locator('input[name=terms]').check()
  await page.locator('form button[type=submit]').click();await page.waitForFunction(()=>window.saved)
  await page.locator('#reviews').click();await page.waitForFunction(()=>new URL(location.href).searchParams.get('matrix-tab')==='reviews')
  await page.evaluate(()=>history.back());await page.waitForFunction(()=>window.matrixTab.v==='overview')
  assert.equal(new URL(page.url()).searchParams.get('campaign'),'keep');assert.equal(documents,1)
  await page.evaluate(()=>window.matrixStop());assert.equal(await page.locator('#counter button').count(),0)
  assert.deepEqual(errors,[])
  console.log(JSON.stringify({engine,version:browser.version(),compiled:true,formValidation:true,urlHistory:true,documentRequests:documents,cleanup:true}))
 }finally{
  await context?.close();await browser?.close();await new Promise(resolve=>{server.close(resolve);server.closeAllConnections()})
 }
})
