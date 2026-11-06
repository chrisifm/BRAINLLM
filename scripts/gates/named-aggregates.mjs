// Real CLI and native HTTP: exact named, array and multi-field aggregates preserve their declared shapes.
import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { join } from 'node:path'
import { binary, brain, createDatabase, freePort, gate, own, ready, stop, write } from './lib.mjs'

await gate('named-aggregates', 3 * 60 * 1000, async directory => {
  const database = createDatabase('gate_named_aggregates')
  const origin = 'http://localhost:5192'
  const env = {
    DATABASE_URL: database, PUBLIC_SITE_URL: origin,
    BRAIN_MAIL_RESEND_KEY: '', GOOGLE_CLIENT_ID: '', GOOGLE_CLIENT_SECRET: '',
    APPLE_CLIENT_ID: '', APPLE_NATIVE_APP_IDS: '', LINKEDIN_CLIENT_ID: '', LINKEDIN_CLIENT_SECRET: '',
  }
  brain(['new', 'app', '--json'], directory, env)
  const app = join(directory, 'app')
  brain(['add', 'resource', 'products', '--fields', 'price:int stock:int category:str tags:list<str>', '--policy', 'public-read', '--json'], app, env)
  write(app, 'server/resources/products/schema.jsbrain', "<meta server>\nexport default { price:'decimal(10,2)',stock:'int',category:'str',tags:'list<str>' }\n")
  write(app, 'server/resources/products/query.jsbrain', `<meta server>
export default {
  aggregate: {
    totals: {sum:['price','stock'],avg:['price'],min:['price'],max:['stock']},
    by_category: {group:'category',sum:['price']},
    by_tag: {group:'tags',sum:['price']},
    category_tag: {group:['category','tags'],sum:['price']}
  }
}
`)
  brain(['backend', 'schema', '--json'], app, env)
  brain(['backend', 'migrate', '--env', 'development', '--json'], app, env)
  let native
  try {
    const port = await freePort(), base = `http://127.0.0.1:${port}`
    native = own(binary, ['backend', 'serve', '--env', 'development', '--port', String(port), '--origin', origin], app, env)
    await ready(base + '/health', native)
    const signup = await fetch(base + '/_brain/auth/signup', {
      method: 'POST', headers: { origin, 'content-type': 'application/json' },
      body: JSON.stringify({email:'aggregate@example.test',password:'synthetic-password-1234'}), signal: AbortSignal.timeout(10000),
    })
    const account = await signup.json()
    assert.equal(signup.status, 200)
    const id = account.user?.id ?? account.id
    assert.match(id, /^[0-9a-f-]{36}$/i, 'only the owned generated UUID enters the SQL fixture')
    const sql = `INSERT INTO _brain_res_products(id,owner,data) VALUES
      (gen_random_uuid(),'${id}','{"price":"10.25","stock":2,"category":"tools","tags":["red","round","round"]}'),
      (gen_random_uuid(),'${id}','{"price":"20.50","stock":3,"category":"tools","tags":["round"]}'),
      (gen_random_uuid(),'${id}','{"price":"30.75","stock":4,"category":"garden","tags":["red"]}');`
    const inserted = spawnSync(process.env.PG_BIN ? join(process.env.PG_BIN, 'psql') : 'psql', ['-X',database,'-v','ON_ERROR_STOP=1','-c',sql], {encoding:'utf8',timeout:10000,maxBuffer:65536})
    assert.equal(inserted.status, 0, inserted.stderr)
    const response = await fetch(base + '/api/products?aggregate=totals,by_category,by_tag,category_tag&limit=1', {signal:AbortSignal.timeout(10000)})
    const value = await response.json()
    assert.equal(response.status, 200)
    assert.equal(value.total, 3)
    assert.equal(value.items.length, 1, 'the aggregates cover the reader set rather than the paginated item count')
    assert.equal(value.aggregates.totals.count, 3)
    assert.equal(value.aggregates.totals.sum.price, '61.50')
    assert.equal(value.aggregates.totals.sum.stock, '9')
    assert.equal(value.aggregates.totals.avg.price, '20.5000')
    assert.equal(value.aggregates.totals.min.price, '10.25')
    assert.equal(value.aggregates.totals.max.stock, 4)
    assert.equal(value.aggregates.by_category.length, 2)
    const byTag = Object.fromEntries(value.aggregates.by_tag.map(row => [row.key,row]))
    assert.equal(byTag.round.count, 2, 'duplicate elements never multiply a row within its array group')
    assert.equal(byTag.round.sum.price, '30.75')
    assert.equal(byTag.red.count, 2)
    assert.equal(byTag.red.sum.price, '41.00')
    const composite = value.aggregates.category_tag
    assert.equal(composite.length, 3)
    assert.ok(composite.every(row => typeof row.key === 'object' && Object.keys(row.key).length === 2))
    const toolsRound = composite.find(row => row.key.category === 'tools' && row.key.tags === 'round')
    assert.equal(toolsRound.count, 2)
    assert.equal(toolsRound.sum.price, '30.75')
    for (const selected of [origin, 'http://localhost:5193', 'https://foreign.example.test', null]) {
      const headers = {'content-type':'application/json'}
      if (selected) headers.origin = selected
      const response = await fetch(base + '/_brain/auth/signup', {method:'POST',headers,body:'{}',signal:AbortSignal.timeout(10000)})
      const body = await response.json()
      const allowed = selected === origin || selected === 'http://localhost:5193'
      assert.equal(response.status, allowed ? 422 : 403)
      assert.equal(body.error?.code ?? body.code, allowed ? 'INVALID_INPUT' : 'ORIGIN_DENIED')
    }
    console.log('Named aggregates: exact decimal results, distinct array elements, object group keys, pagination independence and development origin boundaries')
  } finally {
    if (native) await stop(native.child)
  }
})
