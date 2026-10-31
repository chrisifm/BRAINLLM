// Real loopback TCP transport. The backend below is a handshake fixture, not a substitute for Rust policy tests.
import {test} from 'node:test'
import assert from 'node:assert/strict'
import {createServer as http} from 'node:http'
import {connect} from 'node:net'
import {createHash,randomBytes} from 'node:crypto'
import {mkdtempSync,mkdirSync,writeFileSync,readFileSync,rmSync} from 'node:fs'
import {tmpdir} from 'node:os'
import {join} from 'node:path'
import {pathToFileURL} from 'node:url'
import {createServer} from '../server/server.mjs'
import {installNativeSockets} from '../server/native-sockets.mjs'

const TRUSTED='https://trusted-frontend.test'
const frame=value=>{const bytes=Buffer.from(value);assert.ok(bytes.length<126);return Buffer.concat([Buffer.from([0x81,bytes.length]),bytes])}
const pause=ms=>new Promise(resolve=>setTimeout(resolve,ms))
async function until(predicate,timeout=2000) {
  const deadline=Date.now()+timeout
  while (!predicate()) { if(Date.now()>=deadline) throw Error('Owned socket fixture exceeded its deadline'); await pause(5) }
}
async function fixture(t,options={}) {
  const {base='/app/',accept}=options
  const nativeSockets=Object.hasOwn(options,'nativeSockets')?options.nativeSockets:true
  const directory=mkdtempSync(join(tmpdir(),'brain-node-sockets-'))
  mkdirSync(join(directory,'_server'))
  const manifest={routes:[],rpc:{},base}
  if(Object.hasOwn(options,'manifestSockets'))manifest.nativeSockets=options.manifestSockets
  writeFileSync(join(directory,'_server/manifest.json'),JSON.stringify(manifest))
  const backendSockets=new Set(),frontendSockets=new Set(),clients=new Set(),seen=[],logs=[]
  let frontend
  const backend=http((req,res)=>{res.writeHead(200,{'content-type':'application/json'});res.end('{"http":true}')})
  // HTTP upgrade sockets allow half-open peers; this fixture completes its own write half after EOF.
  backend.on('connection',socket=>{backendSockets.add(socket);socket.on('error',()=>{});socket.once('end',()=>socket.end());socket.once('close',()=>backendSockets.delete(socket))})
  backend.on('upgrade',(request,socket,head)=>{
    seen.push({url:request.url,headers:request.headers,head,received:[]})
    const record=seen.at(-1)
    socket.on('data',chunk=>record.received.push(chunk))
    if (accept) return accept(request,socket,head,record)
    if (request.headers.origin!==TRUSTED || request.headers.authorization==='Bearer invalid') {
      socket.end('HTTP/1.1 401 Unauthorized\r\nContent-Length: 14\r\n\r\nprivate reason');return
    }
    const digest=createHash('sha1').update(request.headers['sec-websocket-key']+'258EAFA5-E914-47DA-95CA-C5AB0DC85B11').digest('base64')
    socket.write(`HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: ${digest}\r\nSec-WebSocket-Protocol: brain.v1\r\n\r\n`)
    socket.write(frame('ready'))
  })
  t.after(async()=>{
    for(const socket of clients) socket.destroy()
    frontend?.$brainSocketStop?.()
    const close=async server=>{
      if (!server?.listening) return
      const closed=new Promise(resolve=>server.close(resolve))
      server.closeAllConnections?.()
      for(const socket of backendSockets)socket.destroy()
      await closed
    }
    await close(frontend);await close(backend)
    rmSync(directory,{recursive:true,force:true})
  })
  await new Promise(resolve=>backend.listen(0,'127.0.0.1',resolve))
  frontend=createServer({dist:directory,backendURL:`http://127.0.0.1:${backend.address().port}`,nativeSockets,log:(...values)=>logs.push(values)})
  frontend.on('connection',socket=>{frontendSockets.add(socket);socket.once('close',()=>frontendSockets.delete(socket))})
  await new Promise(resolve=>frontend.listen(0,'127.0.0.1',resolve))
  const host=`127.0.0.1:${frontend.address().port}`
  async function open({path=(base==='/'?'':base.slice(0,-1))+'/_brain/socket',method='GET',origin=TRUSTED,protocol='brain.v1',cookie='__Host-brain-access=synthetic-access',authorization,extra=[],head=Buffer.alloc(0),streaming=false}={}) {
    const socket=connect({host:'127.0.0.1',port:frontend.address().port,allowHalfOpen:false})
    clients.add(socket);socket.on('error',()=>{});socket.once('close',()=>clients.delete(socket))
    let bytes=Buffer.alloc(0),total=0,closed=false
    socket.on('data',chunk=>{total+=chunk.length;if(streaming){const keep=chunk.subarray(0,Math.max(0,131072-bytes.length));if(keep.length)bytes=Buffer.concat([bytes,keep])}else if(bytes.length+chunk.length>131072)socket.destroy();else bytes=Buffer.concat([bytes,chunk])})
    socket.once('close',()=>{closed=true})
    await new Promise((resolve,reject)=>{socket.once('connect',resolve);socket.once('error',reject)})
    const key=randomBytes(16).toString('base64')
    const headers=[`${method} ${path} HTTP/1.1`,`Host: ${host}`,'Connection: Upgrade','Upgrade: websocket',`Sec-WebSocket-Key: ${key}`,'Sec-WebSocket-Version: 13',...(protocol===null?[]:[`Sec-WebSocket-Protocol: ${protocol}`]),...(origin===null?[]:[`Origin: ${origin}`]),...(cookie===null?[]:[`Cookie: ${cookie}`]),...(authorization===undefined?[]:[`Authorization: ${authorization}`]),...extra]
    socket.write(Buffer.concat([Buffer.from(headers.join('\r\n')+'\r\n\r\n'),head]))
    await until(()=>bytes.includes('\r\n\r\n')||closed)
    assert.ok(bytes.includes('\r\n\r\n'),'expected a bounded HTTP handshake response')
    const boundary=bytes.indexOf('\r\n\r\n')+4
    const response=bytes.subarray(0,boundary).toString()
    return {socket,status:Number(response.split(' ')[1]),response,body:()=>bytes.subarray(boundary),payloadBytes:()=>Math.max(0,total-boundary),closed:()=>closed}
  }
  return {frontend,backend,backendSockets,frontendSockets,seen,logs,open,url:`http://${host}`}
}

test('one opt-in subpath tunnel preserves canonical Origin and native credentials without forwarding unrelated headers',{timeout:8000},async t=>{
  const app=await fixture(t)
  const early=Buffer.from([0x89,0x80,1,2,3,4]) // A masked empty protocol Ping; the transport does not parse it.
  const client=await app.open({cookie:'private_cookie=secret; __Host-brain-access=synthetic-access; __Host-brain-refresh=private-refresh; __Host-brain-google=private-google',authorization:'Bearer synthetic-bearer',extra:['X-Forwarded-Host: attacker.test','Sec-WebSocket-Extensions: permessage-deflate'],head:early})
  assert.equal(client.status,101)
  assert.match(client.response,/Sec-WebSocket-Protocol: brain\.v1/)
  await until(()=>client.body().equals(frame('ready')))
  const request=app.seen[0]
  assert.equal(request.url,'/_brain/socket');assert.equal(request.headers.origin,TRUSTED)
  assert.equal(request.headers.cookie,'__Host-brain-access=synthetic-access')
  assert.equal(request.headers.authorization,'Bearer synthetic-bearer')
  assert.equal(request.headers['sec-websocket-protocol'],'brain.v1')
  assert.equal(request.headers['sec-websocket-extensions'],undefined)
  assert.equal(request.headers['x-forwarded-host'],undefined)
  await until(()=>Buffer.concat([request.head,...request.received]).equals(early))
  const health=await fetch(app.url+'/app/_health')
  assert.equal(health.status,200);assert.equal((await health.json()).ok,true)
})

test('root installation works and invalid Bearer credentials never become a cookie-only handshake',{timeout:8000},async t=>{
  const app=await fixture(t,{base:'/'})
  assert.equal((await app.open()).status,101)
  const rejected=await app.open({authorization:'Bearer invalid'})
  assert.equal(rejected.status,401);assert.equal(rejected.body().length,0)
  assert.equal(app.seen.at(-1).headers.authorization,'Bearer invalid')
  assert.equal(app.seen.at(-1).headers.cookie,'__Host-brain-access=synthetic-access')
})

test('methods, paths, query tokens, Origin and protocol ambiguity are rejected before upstream admission',{timeout:8000},async t=>{
  const app=await fixture(t)
  for(const options of [{method:'POST'},{path:'/_brain/socket'},{path:'/app/_brain/socket?token=never-log'},{path:'/app/_brain/socket?'},{origin:null},{origin:'null'},{origin:'https://trusted-frontend.test/path'},{protocol:null},{protocol:'brain.v1, other'},{extra:['Sec-WebSocket-Protocol: brain.v1']},{extra:['Authorization: Bearer first','Authorization: Bearer second']},{extra:['Cookie: __Host-brain-access=second']}]) {
    const client=await app.open(options)
    assert.ok([400,403,405,431].includes(client.status),JSON.stringify(options))
    await until(client.closed)
  }
  assert.equal(app.seen.length,0)
  const output=JSON.stringify(app.logs)
  for(const secret of ['never-log','synthetic-access','Bearer first','Bearer second']) assert.equal(output.includes(secret),false)
  assert.ok(app.logs.every(([, ,details])=>/^NATIVE_SOCKET_/.test(details.code)&&typeof details.rid==='string'))
})

test('Rust remains the configured Origin authority and rejection details are not copied to the browser',{timeout:8000},async t=>{
  const app=await fixture(t)
  const rejected=await app.open({origin:'https://untrusted-frontend.test'})
  assert.equal(app.seen.at(-1).headers.origin,'https://untrusted-frontend.test')
  assert.equal(rejected.status,401);assert.equal(rejected.body().length,0)
  assert.equal(rejected.response.includes('private reason'),false)
})

test('invalid selected upstream protocol fails closed and closes the admitted TCP connection',{timeout:8000},async t=>{
  const app=await fixture(t,{accept(request,socket){
    const digest=createHash('sha1').update(request.headers['sec-websocket-key']+'258EAFA5-E914-47DA-95CA-C5AB0DC85B11').digest('base64')
    socket.write(`HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: ${digest}\r\nSec-WebSocket-Protocol: other\r\n\r\n`)
  }})
  const rejected=await app.open()
  assert.equal(rejected.status,502)
  await until(()=>rejected.closed()&&app.backendSockets.size===0).catch(error=>{error.message+=' '+JSON.stringify({clientClosed:rejected.closed(),backend:[...app.backendSockets].map(socket=>({readableEnded:socket.readableEnded,writableEnded:socket.writableEnded,destroyed:socket.destroyed}))});throw error})
})

test('shutdown closes pending handshakes and prevents new admission',{timeout:8000},async t=>{
  const app=await fixture(t,{accept(){}})
  const socket=connect({host:'127.0.0.1',port:app.frontend.address().port})
  t.after(()=>socket.destroy());socket.on('error',()=>{})
  await new Promise(resolve=>socket.once('connect',resolve))
  const key=randomBytes(16).toString('base64')
  socket.write(`GET /app/_brain/socket HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Key: ${key}\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Protocol: brain.v1\r\nOrigin: ${TRUSTED}\r\n\r\n`)
  await until(()=>app.seen.length===1)
  app.frontend.$brainSocketStop()
  await until(()=>socket.destroyed&&app.backendSockets.size===0)
  assert.equal((await app.open()).status,503)
})

test('unavailable loopback backend returns a generic bounded error without retaining credentials',{timeout:8000},async t=>{
  const app=await fixture(t)
  await new Promise(resolve=>app.backend.close(resolve))
  const response=await app.open({authorization:'Bearer synthetic-private-bearer'})
  assert.equal(response.status,502);assert.equal(response.body().length,0)
  await until(response.closed)
  const log=JSON.stringify(app.logs)
  assert.ok(log.includes('NATIVE_SOCKET_UPSTREAM_UNAVAILABLE'))
  assert.equal(log.includes('synthetic-private-bearer'),false)
  assert.equal(log.includes('ECONNREFUSED'),false)
})

test('a paused TCP client propagates backpressure and resumes without unbounded JavaScript buffering',{timeout:10000},async t=>{
  let source
  const app=await fixture(t,{accept(request,socket){
    source=socket
    const digest=createHash('sha1').update(request.headers['sec-websocket-key']+'258EAFA5-E914-47DA-95CA-C5AB0DC85B11').digest('base64')
    socket.write(`HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: ${digest}\r\nSec-WebSocket-Protocol: brain.v1\r\n\r\n`)
  }})
  const client=await app.open({streaming:true});assert.equal(client.status,101)
  client.socket.pause()
  // Reuse one bounded chunk; the fixture deliberately tests opaque TCP bytes, not WebSocket frame validation.
  const chunk=Buffer.alloc(16384,122),target=32*1024*1024
  let completed=false,written=0,writeError
  const writer=(async()=>{
    try {
      while(written<target) { await new Promise((resolve,reject)=>source.write(chunk,error=>error?reject(error):resolve()));written+=chunk.length }
      completed=true;source.end()
    } catch(error) {writeError=error}
  })()
  await pause(100)
  assert.equal(completed,false,'a paused receiver must stop this bounded writer before the full transfer completes')
  assert.ok(source.writableLength<=chunk.length)
  assert.ok([...app.frontendSockets].every(socket=>socket.writableLength<=262144))
  client.socket.resume()
  await until(()=>completed||writeError,5000)
  assert.equal(writeError,undefined)
  await writer
  await until(()=>client.payloadBytes()===target&&client.closed(),5000)
  assert.equal(client.payloadBytes(),target)
})

test('a receiver that does not drain is stopped at the write deadline and releases both owned streams',{timeout:10000},async t=>{
  let source
  const app=await fixture(t,{accept(request,socket){
    source=socket
    const digest=createHash('sha1').update(request.headers['sec-websocket-key']+'258EAFA5-E914-47DA-95CA-C5AB0DC85B11').digest('base64')
    socket.write(`HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: ${digest}\r\nSec-WebSocket-Protocol: brain.v1\r\n\r\n`)
  }})
  const client=await app.open({streaming:true});assert.equal(client.status,101)
  client.socket.pause()
  const chunk=Buffer.alloc(16384,122)
  const writer=(async()=>{
    try { for(let bytes=0;bytes<32*1024*1024;bytes+=chunk.length) await new Promise((resolve,reject)=>source.write(chunk,error=>error?reject(error):resolve())) } catch {}
  })()
  await until(()=>app.logs.some(([, ,details])=>details.code==='NATIVE_SOCKET_WRITE_TIMEOUT')&&app.frontendSockets.size===0,6000)
  assert.equal(app.frontendSockets.size,0)
  client.socket.resume()
  await writer
  await until(()=>client.closed()&&app.backendSockets.size===0)
})

test('half-close and server.close both release upgraded sockets rather than waiting for HTTP-only cleanup',{timeout:8000},async t=>{
  const app=await fixture(t)
  const half=await app.open();assert.equal(half.status,101)
  half.socket.end()
  await until(()=>half.closed()&&app.backendSockets.size===0)
  const active=await app.open();assert.equal(active.status,101)
  const closed=new Promise(resolve=>app.frontend.close(resolve))
  await until(()=>active.closed()&&app.backendSockets.size===0)
  await closed
})

test('connection admission is finite and closeAllConnections also owns upgraded sockets',{timeout:10000},async t=>{
  const app=await fixture(t)
  const admitted=[]
  for(let i=0;i<32;i++){const client=await app.open();assert.equal(client.status,101);admitted.push(client)}
  assert.equal((await app.open()).status,503);assert.equal(app.seen.length,32)
  app.frontend.closeAllConnections()
  await until(()=>admitted.every(client=>client.closed())&&app.backendSockets.size===0)
  const replacement=await app.open()
  assert.equal(replacement.status,101,'closeAllConnections keeps its listener available for new upgrades')
  assert.equal((await fetch(app.url+'/_health')).status,200)
})

test('another Upgrade endpoint retains ownership while native path queries are still rejected',{timeout:8000},async t=>{
  const app=await fixture(t)
  let custom=0
  app.frontend.on('upgrade',(request,socket)=>{
    if(request.url!=='/app/custom-socket')return
    custom++
    const digest=createHash('sha1').update(request.headers['sec-websocket-key']+'258EAFA5-E914-47DA-95CA-C5AB0DC85B11').digest('base64')
    socket.end(`HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: ${digest}\r\nSec-WebSocket-Protocol: custom.v1\r\n\r\n`)
  })
  const other=await app.open({path:'/app/custom-socket',protocol:'custom.v1'})
  assert.equal(other.status,101);assert.equal(custom,1);assert.equal(app.seen.length,0)
  assert.equal((await app.open({path:'/app/_brain/socket?token=private'})).status,400)
  assert.equal(app.seen.length,0)
  assert.equal((await app.open()).status,101)
  assert.equal(app.seen.length,1)
})

test('normal upstream EOF flushes the final queued TCP bytes before both sides close',{timeout:10000},async t=>{
  let source
  const app=await fixture(t,{accept(request,socket){
    source=socket
    const digest=createHash('sha1').update(request.headers['sec-websocket-key']+'258EAFA5-E914-47DA-95CA-C5AB0DC85B11').digest('base64')
    socket.write(`HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: ${digest}\r\nSec-WebSocket-Protocol: brain.v1\r\n\r\n`)
  }})
  const client=await app.open({streaming:true});assert.equal(client.status,101)
  client.socket.pause()
  const target=4*1024*1024
  source.end(Buffer.alloc(target,122))
  const consume=setInterval(()=>{while(client.socket.read()!==null);},1)
  try {await until(client.closed,6000)} finally {clearInterval(consume)}
  assert.equal(client.payloadBytes(),target)
  await until(()=>app.backendSockets.size===0)
})

test('native sockets are disabled by default and HTTP health remains available',{timeout:8000},async t=>{
  const app=await fixture(t,{nativeSockets:undefined})
  assert.equal(app.frontend.listenerCount('upgrade'),0)
  assert.equal((await fetch(app.url+'/_health')).status,200)
  assert.throws(()=>createServer({nativeSockets:true,env:{},log(){}}),/configured native backend/)
  assert.throws(()=>installNativeSockets(app.frontend,{target:'https://public.test',cookie:()=>''}),/loopback/)
})

test('compiled nativeSockets manifest opt-in activates the bridge without a manual runtime flag',{timeout:8000},async t=>{
  const app=await fixture(t,{manifestSockets:true,nativeSockets:undefined})
  assert.equal(app.frontend.listenerCount('upgrade'),1)
  assert.equal((await app.open()).status,101)
})

test('explicit runtime false overrides the compiled manifest while legacy HTTP remains available',{timeout:8000},async t=>{
  const app=await fixture(t,{manifestSockets:true,nativeSockets:false})
  assert.equal(app.frontend.listenerCount('upgrade'),0)
  assert.equal((await fetch(app.url+'/_health')).status,200)
  assert.equal(app.seen.length,0)
})

test('explicit runtime true remains available as a low-level override of a disabled manifest',{timeout:8000},async t=>{
  const app=await fixture(t,{manifestSockets:false,nativeSockets:true})
  assert.equal((await app.open()).status,101)
})

test('manifest and runtime socket flags require actual booleans and enabled manifests require a backend',t=>{
  const directory=mkdtempSync(join(tmpdir(),'brain-socket-manifest-'))
  t.after(()=>rmSync(directory,{recursive:true,force:true}))
  mkdirSync(join(directory,'_server'))
  const manifest=join(directory,'_server/manifest.json')
  for(const value of ['true','false',1,0,null,[],{}]) {
    writeFileSync(manifest,JSON.stringify({routes:[],rpc:{},nativeSockets:value}))
    // An explicit override must not hide a malformed private build manifest.
    assert.throws(()=>createServer({dist:directory,nativeSockets:false,env:{},log(){}}),/Manifest nativeSockets.*boolean/)
  }
  writeFileSync(manifest,JSON.stringify({routes:[],rpc:{},nativeSockets:true}))
  assert.throws(()=>createServer({dist:directory,env:{},log(){}}),/configured native backend/)
  for(const value of ['true',1,null])assert.throws(()=>createServer({dist:directory,nativeSockets:value,env:{},log(){}}),/nativeSockets.*boolean/)
})

test('both emitted production and development sidecar layouts load their private helper',{timeout:8000},async t=>{
  const directory=mkdtempSync(join(tmpdir(),'brain-socket-layout-'))
  t.after(()=>rmSync(directory,{recursive:true,force:true}))
  const server=readFileSync(new URL('../server/server.mjs',import.meta.url),'utf8')
  const helper=readFileSync(new URL('../server/native-sockets.mjs',import.meta.url),'utf8')
  for(const production of [false,true]) {
    const folder=join(directory,production?'dist':'sidecar')
    mkdirSync(join(folder,'_server'),{recursive:true})
    writeFileSync(join(folder,production?'_server/native-sockets.mjs':'native-sockets.mjs'),helper)
    writeFileSync(join(folder,'server.mjs'),production?server.replace("from './native-sockets.mjs'","from './_server/native-sockets.mjs'"):server)
    const loaded=await import(pathToFileURL(join(folder,'server.mjs')).href)
    assert.equal(typeof loaded.createServer,'function')
    if(production) {
      const server=loaded.createServer({dist:folder,log(){}})
      try {
        await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve))
        const response=await fetch(`http://127.0.0.1:${server.address().port}/_server/native-sockets.mjs`)
        assert.equal(response.status,404)
        assert.equal((await response.text()).includes('installNativeSockets'),false)
      } finally {await new Promise(resolve=>{server.close(resolve);server.closeAllConnections()})}
    }
  }
})
