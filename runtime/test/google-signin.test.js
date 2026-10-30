import { test } from 'node:test'
import assert from 'node:assert/strict'
import { install } from '../shim/dom.js'
const doc = install()
const { default: adapter } = await import('../templates/google-signin.js')
const flush = async () => { for (let n = 0; n < 8; n++) await Promise.resolve() }
const nonce = 'a'.repeat(64), client = '1234567890-test.apps.googleusercontent.com'
test('GIS uses backend nonce and configured public client, sends credential once and keeps tokens out of storage', async () => {
  const previous = { fetch: globalThis.fetch, google: globalThis.google, location: globalThis.location }
  let initialized, rendered = 0, reloaded = 0, mutations = 0, release
  const requests = [], states = []
  globalThis.google = { accounts: { id: { initialize(options) { initialized = options }, renderButton() { rendered++ } } } }
  globalThis.location = { reload() { reloaded++ } }
  globalThis.fetch = async (path, options) => {
    requests.push([path, options])
    if (path.endsWith('/me') || path.endsWith('/refresh')) return { ok: false, status: 401 }
    if (path.endsWith('/challenge')) return { ok: true, json: async () => ({ client_id: client, nonce }) }
    mutations++; await new Promise(resolve => { release = resolve })
    return { ok: true, json: async () => ({ user: { id: 'u' }, access_token: 'never-store-this-token' }) }
  }
  const element = doc.createElement('div'), widget = adapter.mount(element, { onState: state => states.push(state) })
  try {
    await flush(); assert.equal(initialized.client_id, client); assert.equal(initialized.nonce, nonce); assert.equal(rendered, 1)
    const one = initialized.callback({ credential: 'synthetic-google-credential' })
    await initialized.callback({ credential: 'duplicate' }); assert.equal(mutations, 1)
    release(); await one
    assert.equal(reloaded, 1); assert.equal(states.at(-1).signedIn, true)
    const sent = requests.find(([path]) => path.endsWith('/google'))[1]
    assert.deepEqual(JSON.parse(sent.body), { credential: 'synthetic-google-credential' })
    assert.equal(sent.credentials, 'same-origin')
    assert.ok(!JSON.stringify(states).includes('never-store-this-token'))
  } finally { widget.destroy(); Object.assign(globalThis, previous) }
})
test('adapter cleanup cancels owned requests and does not expose server messages or update a disposed widget', async () => {
  const previous = { fetch: globalThis.fetch, google: globalThis.google }
  let signal, release, initialized = 0
  globalThis.google = { accounts: { id: { initialize() { initialized++ }, renderButton() {} } } }
  globalThis.fetch = async (_, options) => { signal = options.signal; await new Promise(resolve => { release = resolve }); return { ok: false, json: async () => ({ message: 'private server stack and token' }) } }
  const states = [], element = doc.createElement('div'), widget = adapter.mount(element, { onState: state => states.push(state) })
  await flush(); const count = states.length
  widget.destroy(); assert.equal(signal.aborted, true); release(); await flush()
  assert.equal(states.length, count); assert.equal(initialized, 0)
  assert.ok(!JSON.stringify(states).includes('private server'))
  Object.assign(globalThis, previous)
})
test('already authenticated view avoids loading GIS and logout uses a fresh auth-boundary reload', async () => {
  const previous = { fetch: globalThis.fetch, location: globalThis.location }
  let reloaded = 0; const paths = [], states = []
  globalThis.location = { reload() { reloaded++ } }
  globalThis.fetch = async path => { paths.push(path); return { ok: true, json: async () => path.endsWith('/me') ? { user: { id: 'u' } } : { ok: true } } }
  const widget = adapter.mount(doc.createElement('div'), { onState: state => states.push(state) })
  try {
    await flush(); assert.deepEqual(paths, ['/_brain/auth/me']); assert.equal(states.at(-1).signedIn, true)
    widget.update({ action: 'logout', onState: state => states.push(state) }); await flush()
    assert.equal(paths.at(-1), '/_brain/auth/logout'); assert.equal(reloaded, 1)
  } finally { widget.destroy(); Object.assign(globalThis, previous) }
})

test('expired access refreshes once through the native endpoint without loading GIS or persisting tokens', async () => {
  const previous = { fetch: globalThis.fetch, location: globalThis.location }
  const paths = []; let reloads = 0
  globalThis.location = { reload() { reloads++ } }
  globalThis.fetch = async path => { paths.push(path);return path.endsWith('/me') ? {ok:false,status:401} : {ok:true,json:async()=>({user:{id:'u'},access_token:'private-access'})} }
  const widget = adapter.mount(doc.createElement('div'))
  try { await flush(); assert.deepEqual(paths,['/_brain/auth/me','/_brain/auth/refresh']);assert.equal(reloads,1) }finally{widget.destroy();Object.assign(globalThis,previous)}
})

test('native account requests keep the application hosting prefix and never use a foreign base',async()=>{
  const previous=globalThis.fetch,paths=[]
  const meta=doc.createElement('meta');meta.setAttribute('name','brain-base');meta.setAttribute('content','/portal/');doc.head.appendChild(meta)
  globalThis.fetch=async path=>{paths.push(path);return {ok:true,json:async()=>({user:{id:'u'}})}}
  let widget
  try {
    widget=adapter.mount(doc.createElement('div'));await flush();assert.deepEqual(paths,['/portal/_brain/auth/me']);widget.destroy();widget=null
    meta.setAttribute('content','https://foreign.example/')
    widget=adapter.mount(doc.createElement('div'));await flush();assert.equal(paths.length,1)
  } finally {widget?.destroy();meta.remove();globalThis.fetch=previous}
})
