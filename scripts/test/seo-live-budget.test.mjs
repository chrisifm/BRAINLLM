// Real CLI/network regression for bounded crawl inventory and same-origin sitemap traversal.
import {test} from 'node:test'
import assert from 'node:assert/strict'
import {createServer} from 'node:http'
import {spawn} from 'node:child_process'
import {mkdtempSync,rmSync} from 'node:fs'
import {tmpdir} from 'node:os'
import {join} from 'node:path'
const binary=process.env.BRAIN_BIN ?? new URL('../../target/debug/brain',import.meta.url).pathname
function run(args,cwd){return new Promise((resolve,reject)=>{const child=spawn(binary,args,{cwd,stdio:['ignore','pipe','pipe']});let stdout='',stderr='';const timer=setTimeout(()=>child.kill('SIGKILL'),10000);child.stdout.on('data',data=>{stdout+=data;if(stdout.length>1024*1024)child.kill('SIGKILL')});child.stderr.on('data',data=>{stderr=(stderr+data).slice(-65536)});child.once('error',error=>{clearTimeout(timer);reject(error)});child.once('exit',(code,signal)=>{clearTimeout(timer);resolve({code,signal,stdout,stderr})})})}
test('live crawl refuses incomplete oversized inventories and foreign sitemap targets before page requests',{timeout:60000},async t=>{
 const directory=mkdtempSync(join(tmpdir(),'brain-live-budget-'));t.after(()=>rmSync(directory,{recursive:true,force:true}))
 let mode='many',hits=0,foreignHits=0,origin
 const foreign=createServer((req,res)=>{foreignHits++;res.end('<urlset/>')})
 await new Promise(ok=>foreign.listen(0,'127.0.0.1',ok));t.after(async()=>{foreign.closeAllConnections();await new Promise(ok=>foreign.close(ok))})
 const foreignOrigin='http://127.0.0.1:'+foreign.address().port
 const server=createServer((req,res)=>{hits++;res.setHeader('content-type','application/xml');if(mode==='redirect'){res.writeHead(302,{location:foreignOrigin+'/sitemap.xml'});return res.end()}if(mode==='child')return res.end('<sitemapindex><sitemap><loc>'+foreignOrigin+'/child.xml</loc></sitemap></sitemapindex>');res.end('<urlset>'+Array.from({length:1001},(_,i)=>'<url><loc>'+origin+'/page-'+i+'</loc></url>').join('')+'</urlset>')})
 await new Promise(ok=>server.listen(0,'127.0.0.1',ok));origin='http://127.0.0.1:'+server.address().port;t.after(async()=>{server.closeAllConnections();await new Promise(ok=>server.close(ok))})
 for(const selected of ['many','redirect','child']){mode=selected;const before=hits;const result=await run(['seo','audit','--url',origin,'--sample','0','--delay-ms','0','--json'],directory);assert.equal(result.signal,null,result.stderr);assert.equal(result.code,2,result.stdout+result.stderr);const output=JSON.parse(result.stdout);assert.equal(output.ok,false);assert.match(output.error,selected==='many'?/1000/:/origin/);assert.equal(hits,before+1,'must not continue into an incomplete inventory or another target');assert.equal(foreignHits,0)}
 for(const args of [['--sample','1001'],['--delay-ms','10001'],['--path','//foreign.example/path']]){const before=hits;const result=await run(['seo','audit','--url',origin,...args,'--json'],directory);assert.equal(result.code,2,result.stdout+result.stderr);assert.equal(hits,before,'invalid explicit bounds must fail without HTTP')}
})
