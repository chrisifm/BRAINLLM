import { test } from 'node:test'
import assert from 'node:assert/strict'
import { mkdtempSync, mkdirSync, writeFileSync, copyFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { createServer } from '../server/server.mjs'

test('public files and prerendered pages precede dynamic SSR routes without exposing private modules', async () => {
  const dist = mkdtempSync(join(tmpdir(), 'brain-route-priority-'))
  let server
  const write = (file, value) => { mkdirSync(join(dist,file,'..'),{recursive:true}); writeFileSync(join(dist,file),value) }
  try {
    write('_server/manifest.json', JSON.stringify({v:1,core:'/_b/brain.js',rpc:{},routes:[
      {path:'/private',rx:'^/private/?$',view:'/_b/view.js',server:'_server/view.mjs',page:'_server/page.html',guards:['_server/guard.mjs']},
      {path:'/middleware-private',rx:'^/middleware-private/?$',view:'/_b/view.js',server:'_server/view.mjs',page:'_server/page.html',middleware:['_server/guard.mjs']},
      {path:'/guarded/:slug',rx:'^/guarded/(?<slug>[^/]+)/?$',view:'/_b/view.js',server:'_server/view.mjs',page:'_server/page.html',guards:['_server/guard.mjs']},
      {path:'/form-private',rx:'^/form-private/?$',view:'/_b/view.js',forms:true,page:'_server/page.html',guards:['_server/guard.mjs']},
      {path:'/:slug',rx:'^/(?<slug>[^/]+)/?$',view:'/_b/view.js',server:'_server/view.mjs',page:'_server/page.html'}
    ]}))
    mkdirSync(join(dist,'_b'),{recursive:true})
    copyFileSync(new URL('../brain.js',import.meta.url),join(dist,'_b/brain.js'))
    copyFileSync(new URL('../shim/dom.js',import.meta.url),join(dist,'_server/dom.js'))
    write('_b/view.js',"export default function View(props) { const el=document.createElement('main');el.textContent='dynamic:'+props.slug;return el }")
    write('_server/view.mjs',"export async function load(ctx) { return {slug:ctx.params.slug} }")
    write('_server/guard.mjs',"export default () => false")
    write('_server/page.html','<!doctype html><html><head><!--brain-head--></head><body><div id="brain-view"><!--brain:ssr--></div><script type="application/json" id="brain-props"><!--brain:props--></script></body></html>')
    write('about.html','<!doctype html><html><head><title>About</title></head><body><main>prerendered-about</main></body></html>')
    write('favicon.svg','<svg xmlns="http://www.w3.org/2000/svg"></svg>')
    write('verification','public-verification')
    write('private.html','stale-private-output')
    write('private.md','# Stale private Markdown')
    write('middleware-private.html','stale-middleware-output')
    write('guarded/item.html','stale-guarded-output')
    write('form-private.html','stale-form-output')
    server=createServer({dist,log(){}})
    await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve))
    const origin=`http://127.0.0.1:${server.address().port}`
    for (const [path, content] of [['/about','prerendered-about'],['/favicon.svg','<svg'],['/verification','public-verification']]) {
      const response=await fetch(origin+path)
      assert.equal(response.status,200,path)
      const body=await response.text()
      assert.ok(body.includes(content),`${path}: ${body}`)
      assert.ok(!body.includes('dynamic:'),path)
      const head=await fetch(origin+path,{method:'HEAD'})
      assert.equal(head.status,200);assert.equal(await head.text(),'')
    }
    assert.match(await (await fetch(origin+'/article')).text(),/dynamic:article/)
    const privateResponse=await fetch(origin+'/private')
    assert.equal(privateResponse.status,403,'a guarded route remains guarded even when a legacy public artifact collides')
    assert.ok(!(await privateResponse.text()).includes('stale-private-output'))
    assert.equal((await fetch(origin+'/middleware-private')).status,403,'middleware retains the same authority over a colliding file')
    const statuses=await Promise.all(['/guarded/item','/form-private'].map(async path=>(await fetch(origin+path)).status))
    statuses.push((await fetch(origin+'/private',{headers:{accept:'text/markdown'}})).status)
    assert.deepEqual(statuses,[403,403,403],'protected params, forms and Markdown copies cannot take a static fast path')
    assert.equal((await fetch(origin+'/_server/view.mjs')).status,404)
    const post=await fetch(origin+'/about',{method:'POST',headers:{origin,'content-type':'application/x-www-form-urlencoded'},body:''})
    assert.ok(!(await post.text()).includes('prerendered-about'),'POST never falls through to a static document')
  } finally {
    if(server)await new Promise(resolve=>{server.close(resolve);server.closeAllConnections()})
    rmSync(dist,{recursive:true,force:true})
  }
})
