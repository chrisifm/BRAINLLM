import { test } from 'node:test'
import assert from 'node:assert/strict'
import { install, MouseEvent } from '../shim/dom.js'

const doc = install()
const rt0 = await import('./rt.js')
const islandMod = await rt0.sibling('brain-island.js') // islands live outside the core and get it injected
const core = await import(rt0.base + 'brain.js')
const hydrate = r => islandMod.hydrate(r, core)
const chunk = new URL('./fixtures/counter.js', import.meta.url).href
const wait = () => new Promise(r => setTimeout(r, 20))
const click = el => el.dispatchEvent(new MouseEvent('click', { bubbles: true, cancelable: true }))

test('hydrate: island stays inert until the first event, then resumes and replays the event', async () => {
  doc.body.innerHTML =
    `<p>outside</p><button data-b="k3x9">n=4<!--b:0--><span>+</span></button>` +
    `<script type="b/state">{"c":"${chunk}","x":"Counter","p":{"start":4}}</script><div data-b="zz">scope only</div>`
  hydrate()
  hydrate() // idempotent (the router re-runs page scripts): a second call must not replay events twice
  const ssrBtn = doc.body.childNodes[1]
  assert.equal(ssrBtn.textContent, 'n=4+')
  click(doc.body.firstChild) // outside the island: does nothing
  click(doc.body.lastChild) // data-b without b/state: not an island
  await wait()
  assert.equal(doc.body.childNodes[1], ssrBtn)
  click(ssrBtn.lastChild) // first click on the <span> inside the island
  await wait()
  const btn = doc.body.childNodes[1]
  assert.notEqual(btn, ssrBtn)
  assert.equal(btn.textContent, 'n=5+') // mounted with p.start=4 and replayed the click
  assert.equal(doc.body.querySelectorAll('[type=b/state]').length, 0)
  click(btn.lastChild) // already live: goes through the normal delegate, synchronously
  assert.equal(btn.textContent, 'n=6+')
})

// --- #132: resuming must not lose what the user already did on the prerendered DOM ---
const typer = new URL('./fixtures/typer.js', import.meta.url).href
const { mount } = await import('./rt.js')
const { Event: Ev } = await import('../shim/dom.js')
// prerender the island like the build does (same markup + b/state), then return its root
const island = (c = typer) => {
  const tmp = doc.createElement('div')
  const { Typer } = globalThis.__TyperMod
  mount(Typer, tmp, {})
  doc.body.innerHTML = tmp.innerHTML.replace('<form>', '<form data-b="ty1">') + `<script type="b/state">{"c":"${c}","x":"Typer","p":{}}</script>`
  return doc.body.firstChild
}
globalThis.__TyperMod = await import(typer)

test('#132 typing in a sleeping island: intent events pass through, the text survives the resume, focus stays', async () => {
  const f = island()
  const ssrInput = f.firstChild
  const pd = new MouseEvent('pointerdown', { bubbles: true, cancelable: true })
  ssrInput.dispatchEvent(pd)
  const fi = new Ev('focusin', { bubbles: true })
  ssrInput.dispatchEvent(fi)
  ssrInput.focus()
  const kd = new Ev('keydown', { bubbles: true, cancelable: true })
  ssrInput.dispatchEvent(kd)
  assert.equal(pd.defaultPrevented || kd.defaultPrevented, false) // the browser still inserts the key
  assert.equal(doc.body.firstChild, f) // no resume yet: only the chunk is preloading
  ssrInput.value = 'Alpha' // what the key produced in the prerendered input
  ssrInput.dispatchEvent(new Ev('input', { bubbles: true }))
  await wait()
  const live = doc.body.firstChild
  assert.notEqual(live, f)
  assert.equal(live.firstChild.value, 'Alpha') // carried over
  assert.equal(globalThis.__typer.text.v, 'Alpha') // the replayed input reached the handler
  assert.equal(doc.activeElement, live.firstChild) // focus kept on the same field
  live.dispatchEvent(new Ev('submit', { bubbles: true, cancelable: true }))
  assert.match(live.lastChild.textContent, /^Alpha\|0/)
})

test('#132 first click on a sleeping island: pointerdown + focusin do not swap the button; the handler runs once', async () => {
  const f = island()
  const btn = f.childNodes[1]
  btn.dispatchEvent(new MouseEvent('pointerdown', { bubbles: true, cancelable: true }))
  btn.dispatchEvent(new Ev('focusin', { bubbles: true }))
  await wait() // a real click lands only after pointerup: the button must still be there by then
  assert.equal(doc.body.firstChild, f) // still the same button under the pointer
  click(btn)
  await wait()
  assert.equal(globalThis.__typer.clicks.v, 1)
  assert.match(doc.body.firstChild.lastChild.textContent, /\|1$/)
})

test('#131 a chunk that no longer exists reloads the page once per build, never in a loop', async () => {
  const ss = new Map()
  globalThis.sessionStorage = { getItem: k => ss.get(k) ?? null, setItem: (k, v) => ss.set(k, String(v)) }
  let reloads = 0
  globalThis.location = { reload: () => reloads++ }
  const meta = doc.createElement('meta')
  meta.setAttribute('name', 'brain-build')
  meta.setAttribute('content', 'b1')
  const gone = new URL('./fixtures/deleted-by-deploy.js', import.meta.url).href
  island(gone)
  doc.body.appendChild(meta)
  click(doc.body.firstChild.childNodes[1])
  await wait()
  island(gone)
  doc.body.appendChild(meta)
  click(doc.body.firstChild.childNodes[1])
  await wait()
  assert.equal(reloads, 1)
  assert.deepEqual([...ss.keys()], ['brain:reload:b1'])
  delete globalThis.location
})

test('core hydrate() loads brain-island.js on demand and hydrates the given root', async () => {
  const root = doc.createElement('div')
  root.innerHTML = `<button data-b="k3x9">n=1<!--b:0--><span>+</span></button><script type="b/state">{"c":"${chunk}","x":"Counter","p":{"start":1}}</script>`
  doc.body.appendChild(root)
  await rt0.hydrate(root)
  click(root.firstChild)
  await wait()
  assert.equal(root.firstChild.textContent, 'n=2+')
})

test('#94 B0214 (dev): a prerendered island whose HTML differs from the first client render is reported', async () => {
  const warns = [], w0 = console.warn
  console.warn = e => warns.push(String((e && e.message) || e))
  globalThis.__brainDev = true
  for (const shown of ['n=4', 'n=9']) {
    doc.body.innerHTML = `<button data-b="k3x9">${shown}<!--b:0--><span>+</span></button><script type="b/state">{"c":"${chunk}","x":"Counter","p":{"start":4}}</script>`
    hydrate()
    click(doc.body.firstChild.lastChild)
    await wait()
  }
  delete globalThis.__brainDev
  console.warn = w0
  assert.equal(warns.filter(m => m.startsWith('B0214')).length, 1, warns.join(' | ')) // only the n=9 one
  assert.match(warns.find(m => m.startsWith('B0214')), /server ".*n=9.*" · client ".*n=4.*"/)
})
