// Own output extensions must execute in both Node module-hook implementations, not merely download correctly.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
const serverURL = new URL('../server/server.mjs', import.meta.url).href
const runtimeURL = new URL('../brain.js', import.meta.url).href
const shimURL = new URL('../shim/dom.js', import.meta.url).href
const fixture = `
import assert from 'node:assert/strict'
import module from 'node:module'
import { mkdtempSync,mkdirSync,writeFileSync,copyFileSync,rmSync } from 'node:fs'
import { fileURLToPath,pathToFileURL } from 'node:url'
import { join } from 'node:path'
import { tmpdir } from 'node:os'
if(process.env.TEST_FALLBACK==='1'){module.registerHooks=undefined;module.syncBuiltinESMExports()}
const {createServer}=await import(process.env.TEST_SERVER)
const root=mkdtempSync(join(tmpdir(),'brain-own-ssr-')),dist=join(root,'dist'),base=process.env.TEST_BASE
let server
try {
 mkdirSync(join(dist,'_b'),{recursive:true});mkdirSync(join(dist,'_server'))
 copyFileSync(fileURLToPath(process.env.TEST_RUNTIME),join(dist,'_b/core.jsbrain'))
 copyFileSync(fileURLToPath(process.env.TEST_SHIM),join(dist,'_server/dom.js'))
 writeFileSync(join(dist,'_b/helper.jsbrain'),'export const text="Own SSR proof";')
 writeFileSync(join(dist,'_b/view.jsbrain'),'import {tpl} from "'+base+'_b/core.jsbrain";import {text} from "./helper.jsbrain";export default function View(){return tpl("<h1>"+text+"</h1>")()}')
 writeFileSync(join(dist,'_b/dict.jsbrain'),'globalThis.__ownDictionaryLoaded=true;')
 writeFileSync(join(dist,'_server/load.mjs'),'export async function load(){return {ready:true}}')
 writeFileSync(join(dist,'_server/page.html'),'<html><head><title>Example</title></head><body><!--brain:ssr--><script id="brain-props" type="application/json"><!--brain:props--></script></body></html>')
 const route={view:base+'_b/view.jsbrain',i18n:base+'_b/dict.jsbrain',layouts:[],server:'_server/load.mjs',page:'_server/page.html'}
 writeFileSync(join(dist,'_server/manifest.json'),JSON.stringify({base,core:base+'_b/core.jsbrain',routes:[{...route,path:'/page',rx:'^/page$',stream:false},{...route,path:'/live',rx:'^/live$',stream:true}],rpc:{}}))
 server=createServer({dist,log(){},streamHoldMs:0})
 await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve))
 for(const path of ['page','live']) {
  const response=await fetch('http://127.0.0.1:'+server.address().port+base+path)
  const body=await response.text()
  assert.equal(response.status,200,body)
  assert.ok(body.includes('<h1>Own SSR proof</h1>'),body)
  assert.ok(response.headers.get('content-security-policy'))
 }
 assert.equal(globalThis.__ownDictionaryLoaded,true)
 const foreign=join(root,'foreign.jsbrain');writeFileSync(foreign,'export const example=1')
 await assert.rejects(import(pathToFileURL(foreign).href),{code:'ERR_UNKNOWN_FILE_EXTENSION'})
} finally {
 if(server)await new Promise(resolve=>{server.close(resolve);server.closeAllConnections()})
 rmSync(root,{recursive:true,force:true})
}
`
test('own chunks execute for SSR and streaming through synchronous and asynchronous hooks under root and base', { timeout: 45000 }, () => {
  for (const fallback of ['0', '1']) for (const base of ['/', '/site/']) {
    const result = spawnSync(process.execPath, ['--input-type=module', '--eval', fixture], { env: { ...process.env, TEST_SERVER: serverURL, TEST_RUNTIME: runtimeURL, TEST_SHIM: shimURL, TEST_BASE: base, TEST_FALLBACK: fallback }, encoding: 'utf8', timeout: 10000, maxBuffer: 128 * 1024 })
    assert.equal(result.status, 0, `${fallback} ${base}: ${result.stderr || result.error || result.stdout}`)
  }
})
