// Compiled raw-segment constraints are checked before load, form and live sitemap work.
import { test } from 'node:test'
import { request } from 'node:http'
import assert from 'node:assert/strict'
import { copyFileSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { pathToFileURL } from 'node:url'
import { createServer } from '../server/server.mjs'

const segment = String.raw`(?=[\x21-\x2e\x30-\x5b\x5d-\x7e]{1,256}(?:/|(?![\s\S])))[\x32-\x39\x61-\x7a]{7}`
test('constrained SSR skips invalid loads and sitemaps; static, private paths and redirects retain priority', async () => {
  const dist = mkdtempSync(join(tmpdir(), 'brain-route-constraints-'))
  const write = (path, text) => { mkdirSync(join(dist, path, '..'), { recursive: true }); writeFileSync(join(dist, path), text) }
  mkdirSync(join(dist, '_b'), { recursive: true }); mkdirSync(join(dist, '_server'), { recursive: true })
  copyFileSync(new URL('../brain.js', import.meta.url), join(dist, '_b/core.js'))
  copyFileSync(new URL('../shim/dom.js', import.meta.url), join(dist, '_server/dom.js'))
  write('_b/view.js', "import {tpl} from '/app/_b/core.js';export default function View(){return tpl('<main>Record route</main>')()}")
  write('_server/view.mjs', "let seen=[];export function calls(){return [...seen]}export function load(ctx){seen.push(ctx.params.code);return {}};export function sitemap(){return [{code:'3abcdef'},{code:'zzz'},{code:'%32abcd'}]}")
  write('_server/page.html', '<!doctype html><html><head><title>Record</title></head><body><main id="brain-view"><!--brain:ssr--></main><script type="application/json" id="brain-props"><!--brain:props--></script></body></html>')
  write('4abcdef.html', '<h1>Static winner</h1>')
  write('favicon.svg', '<svg/>')
  write('sitemap.xml', '<?xml version="1.0"?><urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9"></urlset>')
  write('_server/manifest.json', JSON.stringify({v:1,rawRouting:true,build:'constraints',site:'https://example.com',base:'/app/',core:'/app/_b/core.js',routes:[{path:'/:code',rx:`^/(?<code>(?![_.])${segment})/?$`,view:'/app/_b/view.js',layouts:[],server:'_server/view.mjs',page:'_server/page.html',sitemap:true}],rpc:{},redirects:{exact:{'/old/exact':['/target',301]},patterns:[{re:`^/old/(${segment})/?$`,to:'/items/$1',status:308}]}}))
  const server = createServer({dist,log(){}})
  try {
    await new Promise(resolve => server.listen(0, '127.0.0.1', resolve))
    const origin = 'http://127.0.0.1:' + server.address().port
    const get = path => fetch(origin+path,{redirect:'manual',signal:AbortSignal.timeout(3000)})
    const loaded=await import(pathToFileURL(join(dist,'_server/view.mjs')).href)
    const rawGet = path => new Promise((resolve, reject) => {
      const outgoing = request({host:'127.0.0.1',port:server.address().port,path,method:'GET',agent:false}, incoming => { incoming.resume(); incoming.once('end', () => resolve(incoming.statusCode)) })
      outgoing.once('error',reject); outgoing.setTimeout(2000,()=>outgoing.destroy(Error('Owned raw routing test deadline'))); outgoing.end()
    })
    for (const path of ['/app/x/../3abcdef','/app/x/%2e%2e/3abcdef','/app/x/.%2e/3abcdef','/app/x/%2E./3abcdef','/app/x//../3abcdef','/app/x/../old/3abcdef','/app/\\3abcdef','/app/3abcdef#ignored','http://localhost/app/3abcdef']) assert.equal(await rawGet(path),403,path)
    for(const path of ['/app/zzz','/app/%32abcdef','/app/%33abcdef','/app/0abcdef','/app/'+ 'a'.repeat(257),'/app/_hidden','/app/.hidden','/app/a%2Fbcd']) assert.equal((await get(path)).status,404,path)
    assert.deepEqual(loaded.calls(),[])
    assert.match(await (await get('/app/3abcdef')).text(),/Record route/)
    assert.deepEqual(loaded.calls(),['3abcdef'])
    assert.match(await (await get('/app/4abcdef')).text(),/Static winner/)
    assert.deepEqual(loaded.calls(),['3abcdef'])
    assert.equal((await get('/app/favicon.svg')).status,200)
    assert.equal((await get('/app/_server/view.mjs')).status,404)
    let response=await get('/app/old/3abcdef/?ref=one')
    assert.deepEqual([response.status,response.headers.get('location')],[308,'/app/items/3abcdef?ref=one'])
    for(const path of ['/app/old/zzz','/app/old/%32abcdef']) assert.equal((await get(path)).status,404)
    response=await get('/app/old/exact');assert.deepEqual([response.status,response.headers.get('location')],[301,'/app/target'])
    const sitemap=await(await get('/app/sitemap.xml')).text()
    assert.match(sitemap,/3abcdef/);assert.doesNotMatch(sitemap,/zzz|%2532abcd/)
    assert.deepEqual(loaded.calls(),['3abcdef'])
  } finally {
    server.closeAllConnections(); await new Promise(resolve => server.close(resolve)); rmSync(dist,{recursive:true,force:true})
  }
})

// An unconfigured app retains its existing normalized static-file behavior.
test('raw routing rejection is explicit metadata and leaves legacy path dispatch unchanged',async()=>{
  const dist=mkdtempSync(join(tmpdir(),'brain-legacy-raw-routing-'))
  mkdirSync(join(dist,'_server'));writeFileSync(join(dist,'_server/manifest.json'),JSON.stringify({v:1,base:'/app/',routes:[],rpc:{}}))
  writeFileSync(join(dist,'3abcdef.html'),'<h1>Legacy static page</h1>')
  const server=createServer({dist,log(){}})
  try {
    await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve))
    const status=await new Promise((resolve,reject)=>{const outgoing=request({host:'127.0.0.1',port:server.address().port,path:'/app/x/%2e%2e/3abcdef',agent:false},incoming=>{incoming.resume();incoming.once('end',()=>resolve(incoming.statusCode))});outgoing.once('error',reject);outgoing.setTimeout(2000,()=>outgoing.destroy(Error('Owned legacy test deadline')));outgoing.end()})
    assert.equal(status,200)
  } finally {server.closeAllConnections();await new Promise(resolve=>server.close(resolve));rmSync(dist,{recursive:true,force:true})}
})
