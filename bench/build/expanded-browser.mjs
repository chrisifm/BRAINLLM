// Real Chrome CRUD checks and initial-route payload capture for expanded.mjs outputs.
import {spawn} from 'node:child_process'
import {createServer} from 'node:http'
import {readFileSync,writeFileSync,existsSync,mkdtempSync,rmSync,statSync} from 'node:fs'
import {join,resolve,sep,extname} from 'node:path'
import {gzipSync} from 'node:zlib'
import {once} from 'node:events'
const work=resolve(process.env.BENCH_WORK),out=resolve(process.env.BENCH_OUT)
const sleep=ms=>new Promise(r=>setTimeout(r,ms))
const owned=new Set()
function launch(command,args,extra={}){const p=spawn(command,args,{detached:true,stdio:['ignore','pipe','ignore'],...extra});owned.add(p);return p}
async function stop(p){if(!p)return;try{process.kill(-p.pid,'SIGTERM')}catch{};await sleep(300);try{process.kill(-p.pid,'SIGKILL')}catch{};if(p.exitCode===null)await Promise.race([once(p,'exit').catch(()=>{}),sleep(1000)]);owned.delete(p)}
for(const sig of ['SIGTERM','SIGINT'])process.once(sig,()=>{for(const p of owned)try{process.kill(-p.pid,'SIGKILL')}catch{};process.exit(130)})
async function until(fn,label,ms=10000){const deadline=Date.now()+ms;while(Date.now()<deadline){if(await fn())return;await sleep(25)}throw new Error(`Timed out: ${label}`)}
const profile=mkdtempSync(join(work,'chrome-'))
const chrome=launch(process.env.BRAIN_CHROME||'/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',['--headless=new','--remote-debugging-port=0',`--user-data-dir=${profile}`,'--use-mock-keychain','--password-store=basic','--no-first-run','--no-default-browser-check','--disable-background-networking','about:blank'])
chrome.stdout.resume()
let page,currentServer,currentProcess
try{
 await until(()=>existsSync(join(profile,'DevToolsActivePort')),'Chrome start')
 const port=Number(readFileSync(join(profile,'DevToolsActivePort'),'utf8').split('\n')[0])
 async function open(){
  const t=await(await fetch(`http://127.0.0.1:${port}/json/new?about:blank`,{method:'PUT',signal:AbortSignal.timeout(5000)})).json()
  const ws=new WebSocket(t.webSocketDebuggerUrl)
  await new Promise((r,j)=>{const timer=setTimeout(()=>j(new Error('WebSocket open deadline')),5000);ws.onopen=()=>{clearTimeout(timer);r()};ws.onerror=e=>{clearTimeout(timer);j(e)}})
  let id=0;const pending=new Map(),events=[]
  ws.onmessage=e=>{const m=JSON.parse(e.data);if(m.id){const p=pending.get(m.id);if(p){clearTimeout(p.timer);pending.delete(m.id);m.error?p.reject(new Error(m.error.message)):p.resolve(m.result)}}else if(events.length<10000)events.push(m)}
  const send=(method,params={})=>new Promise((resolve,reject)=>{if(pending.size>=32)return reject(new Error('CDP pending limit'));const k=++id,timer=setTimeout(()=>{pending.delete(k);reject(new Error(`CDP deadline: ${method}`))},5000);pending.set(k,{resolve,reject,timer});ws.send(JSON.stringify({id:k,method,params}))})
  const evaluate=async expression=>{const r=await send('Runtime.evaluate',{expression,awaitPromise:true,returnByValue:true,userGesture:true});if(r.exceptionDetails)throw new Error(r.exceptionDetails.exception?.description||r.exceptionDetails.text);return r.result.value}
  const close=async()=>{try{await send('Target.closeTarget',{targetId:t.id})}catch{};ws.close();for(const p of pending.values()){clearTimeout(p.timer);p.reject(new Error('closed'))}pending.clear()}
  await send('Page.enable');await send('Runtime.enable');await send('Network.enable');await send('Network.setCacheDisabled',{cacheDisabled:true})
  return{send,evaluate,events,close}
 }
 const cases=[['brain','/dashboard','dist'],['react','/','dist'],['svelte','/','dist'],['vue','/','dist'],['angular','/','dist/browser'],['astro','/','dist'],['react-webpack','/','dist']]
 const selected=process.env.BENCH_ONLY?cases.filter(c=>process.env.BENCH_ONLY.split(',').includes(c[0])):cases
 const report={schemaVersion:1,date:new Date().toISOString(),chrome:null,results:[]}
 for(const [id,route,dist] of selected){
  console.log(`Browser parity: ${id}`)
  let origin
  if(id==='brain'){
   currentProcess=launch(join(work,'brain-bin'),['start','--port','0'],{cwd:join(work,id),env:{...process.env,BRAIN_NODE:process.execPath}})
   let text='';currentProcess.stdout.on('data',b=>{if(text.length<8192)text+=b.toString()})
   await until(()=>/http:\/\/127\.0\.0\.1:\d+/.test(text),'BRAINLLM server start')
   origin=text.match(/http:\/\/127\.0\.0\.1:\d+/)[0]
  }else{
   const root=resolve(work,id,dist)
   currentServer=createServer((q,r)=>{try{const path=resolve(root,'.'+decodeURIComponent(new URL(q.url,'http://localhost').pathname));if(path!==root&&!path.startsWith(root+sep)){r.writeHead(403).end();return}const file=statSync(path).isDirectory()?join(path,'index.html'):path;const type={'.html':'text/html','.js':'text/javascript','.css':'text/css','.svg':'image/svg+xml'}[extname(file)]||'application/octet-stream';r.writeHead(200,{'Content-Type':type,'Cache-Control':'no-store'}).end(readFileSync(file))}catch{r.writeHead(404).end()}})
   currentServer.listen(0,'127.0.0.1');await once(currentServer,'listening');origin=`http://127.0.0.1:${currentServer.address().port}`
  }
  await until(async()=>{try{return(await fetch(origin+route,{signal:AbortSignal.timeout(1000)})).ok}catch{return false}},'HTTP ready')
  page=await open();report.chrome??=await page.send('Browser.getVersion')
  await page.send('Page.navigate',{url:origin+route})
  await until(()=>page.evaluate("document.querySelectorAll('tbody tr').length===2 && document.body.textContent.includes('Ada Lovelace')"),'initial rows').catch(async e=>{console.error(JSON.stringify({id,errors:page.events.filter(e=>e.method==='Runtime.exceptionThrown'),body:await page.evaluate('document.body.innerText')}));throw e})
  await sleep(350)
  const resources=[]
  for(const e of page.events){if(e.method!=='Network.responseReceived')continue;const p=e.params;if(!['Document','Stylesheet','Script'].includes(p.type)||p.response.status!==200||!p.response.url.startsWith(origin))continue;const body=await page.send('Network.getResponseBody',{requestId:p.requestId});const data=Buffer.from(body.body,body.base64Encoded?'base64':'utf8');resources.push({path:new URL(p.response.url).pathname,type:p.type,rawBytes:data.length,gzipBytes:gzipSync(data,{level:9}).length})}
  const steps=[]
  const check=async(label,expr)=>{await until(()=>page.evaluate(expr),label);steps.push(label)}
  const click=async(text,scope='document')=>page.evaluate(`(()=>{const el=[...${scope}.querySelectorAll('button')].find(e=>e.textContent.trim()===${JSON.stringify(text)});if(!el)throw Error('button missing');el.click();return true})()`)
  const fill=async(selector,value)=>page.evaluate(`(()=>{const el=document.querySelector(${JSON.stringify(selector)});if(!el)throw Error('input missing');Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(el,${JSON.stringify(value)});el.dispatchEvent(new Event('input',{bubbles:true}));el.dispatchEvent(new Event('change',{bubbles:true}));return true})()`)
  await fill('input[aria-label="Search users"]','Ada');await check('search filters users',"document.querySelectorAll('tbody tr').length===1 && document.querySelector('tbody').textContent.includes('Ada')")
  await fill('input[aria-label="Search users"]','zz-missing');await check('empty search state',"document.querySelector('tbody').textContent.includes('No results for this search.')")
  await fill('input[aria-label="Search users"]','');await click('New');await check('new form validation',"!!document.querySelector('form button[disabled]')")
  await fill('form input:not([type])','Grace Hopper');await fill('form input[type=email]','grace@example.com');await click('Save','document.querySelector("form")');await check('create user',"document.querySelectorAll('tbody tr').length===3 && document.querySelector('tbody').textContent.includes('Grace Hopper')")
  await page.evaluate("[...document.querySelectorAll('tbody tr')].find(r=>r.textContent.includes('Grace Hopper')).querySelector('button').click();true")
  await check('edit form populated',"document.querySelector('form input')?.value==='Grace Hopper'")
  await fill('form input:not([type])','Grace Updated');await click('Save','document.querySelector("form")');await check('update user',"document.querySelector('tbody').textContent.includes('Grace Updated') && !document.querySelector('form')")
  await click('New');await check('cancel form visible',"!!document.querySelector('form')");await click('Cancel','document.querySelector("form")');await check('cancel preserves rows',"!document.querySelector('form') && document.querySelectorAll('tbody tr').length===3")
  await page.evaluate("[...document.querySelectorAll('tbody tr')].find(r=>r.textContent.includes('Grace Updated')).querySelectorAll('button')[1].click();true")
  await check('delete user',"document.querySelectorAll('tbody tr').length===2 && !document.querySelector('tbody').textContent.includes('Grace Updated')")
  const exceptions=page.events.filter(e=>e.method==='Runtime.exceptionThrown').map(e=>e.params.exceptionDetails.text)
  if(exceptions.length)throw new Error(`${id}: runtime exceptions ${exceptions.join(',')}`)
  report.results.push({id,route,steps,exceptions,resources,routeRawBytes:resources.reduce((n,r)=>n+r.rawBytes,0),routeGzipBytes:resources.reduce((n,r)=>n+r.gzipBytes,0),note:'Initial successful Document/Stylesheet/Script responses after data readiness; gzip level9 per body; images and later resources excluded'})
  writeFileSync(join(out,'browser.json'),JSON.stringify(report,null,2)+'\n')
  await page.close();page=null
  if(currentServer){currentServer.closeAllConnections();await new Promise(r=>currentServer.close(r));currentServer=null}
  await stop(currentProcess);currentProcess=null
 }
}finally{
 if(page)await page.close()
 if(currentServer){currentServer.closeAllConnections();await new Promise(r=>currentServer.close(r))}
 await stop(currentProcess);await stop(chrome);rmSync(profile,{recursive:true,force:true})
}
