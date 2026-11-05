// Generated canonical patterns must retain the application base in both SSR paths.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { mkdtempSync, mkdirSync, writeFileSync, copyFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { createServer } from '../server/server.mjs'
test('normal and streamed SSR retain the app base in generated canonical and Open Graph patterns', { timeout: 15000 }, async () => {
  const dist=mkdtempSync(join(tmpdir(),'brain-i18n-base-'))
  const runtime=fileURLToPath(new URL('../',import.meta.url))
  const write=(path,text)=>{mkdirSync(join(dist,path,'..'),{recursive:true});writeFileSync(join(dist,path),text)}
  let server
  try {
    write('package.json','{"type":"module"}')
    mkdirSync(join(dist,'_b'),{recursive:true});copyFileSync(join(runtime,'brain.js'),join(dist,'_b/brain.js'))
    mkdirSync(join(dist,'_server'),{recursive:true});copyFileSync(join(runtime,'shim/dom.js'),join(dist,'_server/dom.js'))
    write('_b/view.js',`import{tpl}from'/_b/brain.js';export default p=>tpl('<h1>'+p.title+'</h1>')()`)
    write('_b/loading.js',`import{tpl}from'/_b/brain.js';export default()=>tpl('<p>Loading example</p>')()`)
    write('_server/view.mjs',`export async function load(){await new Promise(r=>setTimeout(r,300));return{title:'Loaded example'}}`)
    const routes=[]
    for (const stream of [false,true]) {
      const name=stream?'stream':'posts'
      const pattern=`https://example.test/app/es/${name}/:slug`
      write(`_server/pages/${name}.html`,`<!doctype html><html lang="es"><head><title>Example</title><link rel="canonical" href="${pattern}"><meta property="og:url" content="${pattern}"></head><body><div id="brain-view"><!--brain:ssr--></div><script type="application/json" id="brain-props"><!--brain:props--></script></body></html>`)
      routes.push({path:`/es/${name}/:slug`,rx:`^/es/${name}/([^/]+)/?$`,view:'/_b/view.js',layouts:[],server:'_server/view.mjs',page:`_server/pages/${name}.html`,locale:'es',stream,...(stream?{loading:'/_b/loading.js'}:{})})
    }
    write('_server/manifest.json',JSON.stringify({v:1,build:'base-proof',base:'/app/',core:'/_b/brain.js',rpc:{},routes}))
    server=createServer({dist,streamHoldMs:1,log(){}})
    await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve))
    for (const name of ['posts','stream']) {
      const response=await fetch(`http://127.0.0.1:${server.address().port}/app/es/${name}/sample`)
      const html=await response.text()
      assert.equal(response.status,200,html)
      const expected=`https://example.test/app/es/${name}/sample`
      assert.ok(html.includes(`rel="canonical" href="${expected}"`),html)
      assert.ok(html.includes(`property="og:url" content="${expected}"`),html)
      assert.ok(html.includes('Loaded example'),html)
      if(name==='stream') assert.ok(html.includes('<!--brain:loading-->'),'the slow route must actually stream')
    }
  } finally { if(server)await new Promise(resolve=>server.close(resolve));rmSync(dist,{recursive:true,force:true}) }
})
