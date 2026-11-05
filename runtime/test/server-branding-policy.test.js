// Attribution is per build/server instance, including synthesized sitemap indexes and byte-bounded children.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { mkdtempSync,mkdirSync,writeFileSync,rmSync } from 'node:fs'
import { join } from 'node:path'
import { tmpdir } from 'node:os'
import { createServer } from '../server/server.mjs'
const home='https://chrisifm.github.io/BRAINLLM/'
test('concurrent branded and explicitly unbranded instances keep sitemap and header policies independent',async()=>{
  const owned=[]
  try{
    for(const branding of [true,false]){
      const dist=mkdtempSync(join(tmpdir(),'brain-server-branding-'));mkdirSync(join(dist,'_server'))
      writeFileSync(join(dist,'index.html'),'<!doctype html><html><head><title>Example site</title></head><body><h1>Example site</h1></body></html>')
      writeFileSync(join(dist,'sitemap.xml'),'<?xml version="1.0" encoding="UTF-8"?>\n<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9"><url><loc>https://example.test/</loc></url></urlset>')
      writeFileSync(join(dist,'_server/list.mjs'),"export function sitemap(){return Array.from({length:6},(_,i)=>({id:i+'-'+'x'.repeat(300)}))}")
      writeFileSync(join(dist,'_server/manifest.json'),JSON.stringify({v:1,build:'example',compilerBranding:branding,site:'https://example.test',routes:[{path:'/items/:id',rx:'^/items/(?<id>[^/]+)/?$',server:'_server/list.mjs',sitemap:true}]}))
      const server=createServer({dist,sitemapBytes:1200,log(){}});owned.push({server,dist,branding})
      await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve))
    }
    await Promise.all(owned.map(async({server,branding})=>{
      const origin=`http://127.0.0.1:${server.address().port}`
      const reply=await fetch(origin+'/sitemap.xml'),index=await reply.text()
      assert.equal(reply.status,200);assert.ok(index.includes('<sitemapindex'))
      assert.equal(index.includes(home),branding)
      assert.equal(reply.headers.has('x-generated-by'),branding)
      const children=[...index.matchAll(/<loc>https:\/\/example.test([^<]+)<\/loc>/g)].map(match=>match[1])
      assert.ok(children.includes('/sitemap-static.xml')&&children.some(path=>path.startsWith('/sitemap-dynamic')))
      for(const path of children){
        const response=await fetch(origin+path),text=await response.text()
        assert.equal(response.status,200,path);assert.equal(text.includes(home),branding,path)
        assert.equal((text.match(/<\?xml/g)??[]).length,1)
        assert.ok(Buffer.byteLength(text)<=1200,path)
      }
    }))
    const dist=mkdtempSync(join(tmpdir(),'brain-static-branding-'))
    writeFileSync(join(dist,'index.html'),'<!doctype html><title>Example</title><h1>Example</h1>')
    writeFileSync(join(dist,'brain-build.json'),'{"schemaVersion":1,"compilerBranding":false}')
    const server=createServer({dist,log(){}});owned.push({server,dist});await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve))
    const response=await fetch(`http://127.0.0.1:${server.address().port}/`)
    assert.equal(response.status,200);assert.equal(response.headers.get('x-generated-by'),null);await response.text()
  }finally{for(const {server,dist}of owned){await new Promise(resolve=>{server.close(resolve);server.closeAllConnections()});rmSync(dist,{recursive:true,force:true})}}
})
