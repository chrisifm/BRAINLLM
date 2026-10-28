// #285 hydration by adoption (brain-hydrate.js + brain-island.js): the island resumes on the server's own nodes.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { writeFileSync, mkdtempSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { pathToFileURL } from 'node:url'
import { install, MouseEvent } from '../shim/dom.js'

const doc = install()
const rt = await import('./rt.js')
const core = await import(rt.base + 'brain.js')
const H = await import(rt.base + 'brain-hydrate.js')
const island = await import(rt.base + 'brain-island.js')
const wait = (ms = 20) => new Promise(r => setTimeout(r, ms))
const click = el => el.dispatchEvent(new MouseEvent('click', { bubbles: true, cancelable: true }))

// what codegen emits in hydrate mode: fc/nx instead of .firstChild/.nextSibling
const dir = mkdtempSync(join(tmpdir(), 'brain-adopt-'))
const chunk = join(dir, 'isl.mjs')
writeFileSync(chunk, `import { tpl, insert, on, sig, list } from '${rt.base}brain.js'
import { fc, nx } from '${rt.base}brain-hydrate.js'
const t0 = tpl('<section data-b="isl1" class="isl"><h2>Title <!></h2><div class="scroll"><ul><!></ul></div><video class="v"></video><button class="b">+</button></section>')
const tLi = tpl('<li><!></li>')
export default function Isl(p) {
  const n = sig(p.start)
  const r = t0()
  const h2 = fc(r)
  insert(h2, () => n.v, nx(fc(h2)))
  const sc = nx(h2), ul = fc(sc)
  list(ul, () => [1, 2, 3], x => x, s => { const li = tLi(); insert(li, () => s.v * 10 + n.v, fc(li)); return li }, fc(ul))
  on(nx(nx(sc)), 'click', () => n.v++)
  return r
}
`)
const { default: Isl } = await import(pathToFileURL(chunk).href)

test('server render stamps templates (data-hk) and wraps dynamic regions in <!--[--> … <!--]-->', () => {
  const host = doc.createElement('div')
  const end = H.stamp(core._rt())
  const stop = core.mount(Isl, host, { start: 4 })
  end()
  const html = host.innerHTML
  stop()
  assert.match(html, /<section data-b="isl1" class="isl" data-hk="1">/)
  assert.match(html, /<h2>Title <!--\[-->4<!--\]--><\/h2>/)
  assert.match(html, /<li data-hk="\d">/)
})

test('hydration adopts the server nodes: same section, list rows and video (never moved), scroll kept; updates work', async () => {
  const host = doc.createElement('div')
  const end = H.stamp(core._rt())
  const stop = core.mount(Isl, host, { start: 4 })
  end()
  const html = host.innerHTML
  stop()
  doc.body.innerHTML = html + `<script type="b/state">{"c":"${pathToFileURL(chunk).href}","x":"default","p":{"start":4}}</script>`
  const sec = doc.body.querySelector('section'), video = sec.querySelector('video'), sc = sec.querySelector('.scroll'), li = sec.querySelectorAll('li')[1]
  sc.scrollTop = 120
  video.$playing = true
  let moves = 0
  const ins = sec.insertBefore.bind(sec)
  sec.insertBefore = (n, ref) => (n === video && moves++, ins(n, ref))
  island.hydrate(doc, core)
  click(sec.querySelector('.b')) // the first event resumes the island and is replayed
  await wait()
  const now = doc.body.querySelector('section')
  assert.ok(now.isSameNode ? now.isSameNode(sec) : now === sec, 'same section node')
  assert.equal(now.querySelector('video'), video)
  assert.equal(moves, 0) // a playing video is never detached and re-inserted
  assert.equal(video.$playing, true)
  assert.equal(now.querySelector('.scroll').scrollTop, 120)
  assert.equal(now.querySelectorAll('li')[1], li) // list rows adopted too
  assert.equal(now.querySelector('h2').textContent, 'Title 5') // the replayed click updated the adopted text
  assert.equal(now.querySelectorAll('li')[0].textContent, '15')
  assert.equal(doc.body.querySelectorAll('[type=b/state]').length, 0)
})

test('a mismatch (the server rendered something else) falls back to rendering fresh', async () => {
  doc.body.innerHTML = `<section data-b="isl1" data-hk="1"><p>stale</p></section><script type="b/state">{"c":"${pathToFileURL(chunk).href}","x":"default","p":{"start":1}}</script>`
  const old = doc.body.querySelector('section')
  island.hydrate(doc, core)
  click(old.querySelector('p'))
  await wait()
  const now = doc.body.querySelector('section')
  assert.notEqual(now, old)
  assert.equal(now.querySelector('h2').textContent, 'Title 1')
})
