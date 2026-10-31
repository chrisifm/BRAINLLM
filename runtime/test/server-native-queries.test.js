// ctx.backend parity with the HTTP data-query contract (docs/backend/server-queries.md): filters, sort, cursor,
// include, aggregates, PATCH, slug redirects and field errors, all through the same bounded native transport.
import {test, after} from 'node:test'
import assert from 'node:assert/strict'
import {EventEmitter} from 'node:events'
import {createServer} from 'node:http'
import {nativeAuthBridge} from '../server/server.mjs'
const seen=[]
const backend=createServer((req,res)=>{
  const chunks=[]
  req.on('data',chunk=>chunks.push(chunk))
  req.on('end',()=>{
    seen.push({method:req.method,path:req.url,body:Buffer.concat(chunks).toString()})
    const json=(status,value)=>{res.writeHead(status,{'content-type':'application/json'});res.end(JSON.stringify(value))}
    if(req.url.startsWith('/api/articles/redirects/old-title'))return json(200,{slug:'new-title'})
    if(req.url.startsWith('/api/articles/redirects/'))return json(404,{schemaVersion:1,error:{code:'NOT_FOUND',fix:'',fields:null}})
    if(req.url.includes('price%5Bgte%5D=bad'))return json(422,{schemaVersion:1,error:{code:'INVALID_INPUT',fix:'Apply the field errors',fields:{'price[gte]':'Expected a decimal(10,2) string such as "19.50"'}}})
    if(req.method==='PATCH')return json(200,{id:'00000000-0000-4000-8000-000000000005',data:{stock:5},created_at:'2026-10-31T10:00:00.000Z',updated_at:'2026-10-31T10:05:00.000Z'})
    json(200,{items:[{id:'00000000-0000-4000-8000-000000000005',data:{},created_at:'2026-10-31T10:00:00.000Z',updated_at:'2026-10-31T10:05:00.000Z'}],limit:2,offset:0,total:1,next_cursor:'7b22'})
  })
})
await new Promise(resolve=>backend.listen(0,'127.0.0.1',resolve))
const bridge=nativeAuthBridge(`http://127.0.0.1:${backend.address().port}`)
after(async()=>{bridge.close();await new Promise(resolve=>{backend.close(resolve);backend.closeAllConnections()})})
function context(method='POST'){
  const req=new EventEmitter(),res=new EventEmitter()
  req.method=method;req.headers={host:'shop.example.test',origin:'https://shop.example.test',cookie:'__Host-brain-access=synthetic'}
  res.writableEnded=false;res.destroyed=false
  return bridge.backend(req,res,'00000000-0000-4000-8000-000000000004')
}
const id='00000000-0000-4000-8000-000000000005'
test('list forwards declared filters, sort, cursor, include and aggregates with the HTTP query syntax',async()=>{
  const products=context('GET').resource('products')
  const page=await products.list({filter:{category:{in:['tools','garden']},price:{between:['10.00','25.00']},stock:{gt:0},title:'Mug','shop.country':{eq:'MX'},released_on:{exists:true}},sort:['-price','title'],limit:2,include:['shop'],aggregate:['by_category'],count:['reviews']})
  assert.equal(page.next_cursor,'7b22')
  assert.equal(page.items[0].updated_at,'2026-10-31T10:05:00.000Z')
  const url=new URL(seen.at(-1).path,'http://x')
  assert.equal(url.pathname,'/api/products')
  assert.deepEqual([...url.searchParams],[['category[in]','tools,garden'],['price[between]','10.00,25.00'],['stock[gt]','0'],['title','Mug'],['shop.country','MX'],['released_on[exists]','true'],['limit','2'],['sort','-price,title'],['include','shop'],['aggregate','by_category'],['count','reviews']])
  await products.list({sort:'price',after:'7b22',limit:2})
  assert.deepEqual([...new URL(seen.at(-1).path,'http://x').searchParams],[['limit','2'],['sort','price'],['after','7b22']])
  await products.list()
  assert.equal(seen.at(-1).path,'/api/products')
  await products.get(id,{include:['shop']})
  assert.equal(seen.at(-1).path,'/api/products/'+id+'?include=shop')
})
test('invalid list options fail before any request',async()=>{
  const products=context('GET').resource('products'),before=seen.length
  assert.throws(()=>products.list({filter:{'price;drop':'1'}}),/filter/)
  assert.throws(()=>products.list({filter:{price:{like:'1'}}}),/operator/)
  assert.throws(()=>products.list({filter:{price:{gte:{nested:true}}}}),/value/)
  assert.throws(()=>products.list({filter:{category:{in:Array(51).fill('x')}}}),/value/)
  assert.throws(()=>products.list({sort:['a','b','c','d','e']}),/sort/)
  assert.throws(()=>products.list({sort:'price desc'}),/sort/)
  assert.throws(()=>products.list({cursor:'a',after:'b'}),/cursor/)
  assert.throws(()=>products.list({cursor:'x'.repeat(2049)}),/cursor/)
  assert.throws(()=>products.list({include:['../x']}),/include/)
  assert.throws(()=>products.get(id,{search:'x'}),/options/)
  assert.equal(seen.length,before)
})
test('patch sends only the changed fields and field errors reach the caller as errors',async()=>{
  const products=context().resource('products')
  const record=await products.patch(id,{stock:5,note:null})
  assert.equal(seen.at(-1).method,'PATCH');assert.equal(seen.at(-1).body,'{"stock":5,"note":null}')
  assert.equal(record.updated_at,'2026-10-31T10:05:00.000Z')
  await assert.rejects(context().resource('products').patch(id,{}),/at least one field/)
  await assert.rejects(context('GET').resource('products').list({filter:{price:{gte:'bad'}}}),error=>error.status===422&&error.errors['price[gte]'].startsWith('Expected a decimal'))
})
test('slugRedirect answers the current slug or null and never confirms unknown slugs',async()=>{
  const backend=context('GET')
  assert.equal(await backend.slugRedirect('articles','old-title'),'new-title')
  assert.equal(seen.at(-1).path,'/api/articles/redirects/old-title')
  assert.equal(await backend.resource('articles').slugRedirect('missing'),null)
  assert.throws(()=>backend.slugRedirect('articles','a/b'),/slug/)
  assert.throws(()=>backend.slugRedirect('../auth','x'),/name/)
})
