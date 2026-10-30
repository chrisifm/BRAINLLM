import { test, beforeEach } from 'node:test'
import assert from 'node:assert/strict'
import { install, stats, MouseEvent } from '../shim/dom.js'

let doc = install()
const { sig, drv, fx, batch, tpl, insert, attr, cls, on, list, when, mount } = await import('./rt.js')
const { sibling } = await import('./rt.js')
await sibling('brain-dev.js') // This file asserts development diagnostic details; production privacy is tested separately.


beforeEach(() => {
  doc.body.textContent = ''
  stats.moves = 0
})
const click = el => el.dispatchEvent(new MouseEvent('click', { bubbles: true }))
const html = el => el.innerHTML

test('tpl clones the template every time', () => {
  const T = tpl('<p class="x">hello <b>world</b></p>')
  const a = T(), b = T()
  assert.notEqual(a, b)
  assert.equal(a.outerHTML, '<p class="x">hello <b>world</b></p>')
})

test('tpl with several roots returns a fragment; <!> is an anchor comment', () => {
  const f = tpl('<i>a</i><!><i>b</i>')()
  assert.equal(f.nodeType, 11)
  assert.equal(f.childNodes[1].nodeType, 8)
})

test('insert: reactive text reuses the Text node before the anchor', () => {
  const r = tpl('<p>[<!>]</p>')()
  const n = sig(1)
  insert(r, () => n.v, r.childNodes[1])
  const t = r.childNodes[1]
  assert.equal(html(r), '[1<!---->]')
  batch(() => (n.v = 2))
  assert.equal(html(r), '[2<!---->]')
  assert.equal(r.childNodes[1], t)
})

test('insert: switches between node, null, array and text, removing the previous one', () => {
  const r = doc.createElement('div')
  const v = sig(null)
  insert(r, () => v.v)
  assert.equal(html(r), '<!---->')
  batch(() => (v.v = tpl('<b>x</b>')()))
  assert.equal(html(r), '<b>x</b><!---->')
  batch(() => (v.v = ['a', tpl('<i>b</i>')()]))
  assert.equal(html(r), 'a<i>b</i><!---->')
  batch(() => (v.v = 'z'))
  assert.equal(html(r), 'z<!---->')
})

test('static insert creates no effect', () => {
  const r = doc.createElement('div')
  insert(r, 'hello')
  assert.equal(html(r), 'hello<!---->')
})

test('attr, cls, style: and properties', () => {
  const el = doc.createElement('input')
  const on1 = sig(true), val = sig('a')
  attr(el, 'title', () => (on1.v ? 'yes' : null))
  attr(el, 'disabled', () => on1.v)
  attr(el, 'value', () => val.v)
  attr(el, 'style:color', () => (on1.v ? 'red' : null))
  cls(el, 'hot', () => on1.v)
  assert.equal(el.getAttribute('title'), 'yes')
  assert.equal(el.getAttribute('disabled'), '')
  assert.equal(el.value, 'a')
  assert.equal(el.style.getPropertyValue('color'), 'red')
  assert.ok(el.classList.contains('hot'))
  batch(() => {
    on1.v = false
    val.v = 'b'
  })
  assert.equal(el.getAttribute('title'), null)
  assert.equal(el.hasAttribute('disabled'), false)
  assert.equal(el.value, 'b')
  assert.equal(el.style.getPropertyValue('color'), '')
  assert.ok(!el.classList.contains('hot'))
})

test('delegated on: one listener per type, correct currentTarget, stopPropagation stops', () => {
  const r = tpl('<div><button><span>x</span></button></div>')()
  doc.body.appendChild(r)
  const log = []
  on(r, 'click', e => log.push('div:' + e.currentTarget.tagName))
  const btn = r.firstChild
  on(btn, 'click', function (e) {
    log.push('btn:' + e.currentTarget.tagName + ':' + this.tagName)
  })
  click(btn.firstChild)
  assert.deepEqual(log, ['btn:BUTTON:BUTTON', 'div:DIV'])
  on(btn, 'click', e => (log.push('stop'), e.stopPropagation()))
  click(btn.firstChild)
  assert.deepEqual(log.slice(2), ['stop'])
  assert.equal(doc._l.bub.click.length, 1)
})

test('handler runs in a batch: the DOM is updated when the event ends', () => {
  const r = tpl('<button><!></button>')()
  doc.body.appendChild(r)
  const n = sig(0)
  insert(r, () => n.v, r.firstChild)
  on(r, 'click', () => {
    n.v++
    n.v++
  })
  click(r)
  assert.equal(r.textContent, '2')
})

const Row = tpl('<li></li>')
function mkList(items) {
  const ul = doc.createElement('ul')
  doc.body.appendChild(ul)
  const renders = { n: 0 }
  list(ul, () => items.v, it => it.id, it => {
    renders.n++
    const li = Row()
    insert(li, () => it.v.t)
    return li
  })
  return { ul, renders, text: () => ul.childNodes.filter(n => n.nodeType === 1).map(n => n.textContent).join(',') }
}
const it = (id, t = String(id)) => ({ id, t })

test('list: initial render, add and remove', () => {
  const items = sig([it(1), it(2), it(3)])
  const L = mkList(items)
  assert.equal(L.text(), '1,2,3')
  batch(() => (items.v = [it(1), it(2), it(3), it(4)]))
  assert.equal(L.text(), '1,2,3,4')
  assert.equal(L.renders.n, 4)
  batch(() => (items.v = [it(1), it(3)]))
  assert.equal(L.text(), '1,3')
})

test('list: same key with a new object updates in place without re-render', () => {
  const items = sig([it(1, 'a'), it(2, 'b')])
  const L = mkList(items)
  const first = L.ul.firstChild
  batch(() => (items.v = [it(1, 'A'), it(2, 'b')]))
  assert.equal(L.text(), 'A,b')
  assert.equal(L.ul.firstChild, first)
  assert.equal(L.renders.n, 2)
})

test('list: swapping the ends moves only 2 rows (LIS)', () => {
  const ids = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]
  const items = sig(ids.map(i => it(i)))
  const L = mkList(items)
  const nodes = [...L.ul.childNodes]
  stats.moves = 0
  const sw = ids.slice()
  ;[sw[1], sw[8]] = [sw[8], sw[1]]
  batch(() => (items.v = sw.map(i => it(i))))
  assert.equal(L.text(), sw.join(','))
  assert.equal(stats.moves, 2)
  assert.equal(L.ul.childNodes[1], nodes[8])
})

test('list: reversing moves n-1 rows and keeps identity', () => {
  const items = sig([1, 2, 3, 4, 5].map(i => it(i)))
  const L = mkList(items)
  const n1 = L.ul.firstChild
  stats.moves = 0
  batch(() => (items.v = [5, 4, 3, 2, 1].map(i => it(i))))
  assert.equal(L.text(), '5,4,3,2,1')
  assert.equal(stats.moves, 4)
  assert.equal(L.ul.childNodes[4], n1)
})

test('list: removing a row disposes its effects', () => {
  const items = sig([it(1), it(2)])
  const ext = sig(0)
  let runs = 0
  const ul = doc.createElement('ul')
  list(ul, () => items.v, x => x.id, x => {
    fx(() => (ext.v, runs++))
    return Row()
  })
  assert.equal(runs, 2)
  batch(() => (items.v = [it(1)]))
  batch(() => (ext.v = 1))
  assert.equal(runs, 3)
})

test('list: a duplicate key is reported as a caught error', () => {
  const ul = doc.createElement('ul'), ce = console.error, seen = []
  console.error = e => seen.push(e.message)
  list(ul, () => [it(1), it(1)], x => x.id, () => Row())
  console.error = ce
  assert.match(seen.join(), /duplicate key/)
})

test('list: empty block and anchor between static siblings', () => {
  const r = tpl('<ul><li>h</li><!><li>f</li></ul>')()
  const items = sig([])
  list(r, () => items.v, x => x.id, x => {
    const li = Row()
    insert(li, () => x.v.t)
    return li
  }, r.childNodes[1], () => tpl('<li>empty</li>')())
  assert.equal(html(r), '<li>h</li><li>empty</li><!----><li>f</li>')
  batch(() => (items.v = [it(1), it(2)]))
  assert.equal(html(r), '<li>h</li><li>1<!----></li><li>2<!----></li><!----><li>f</li>')
  batch(() => (items.v = []))
  assert.equal(html(r), '<li>h</li><li>empty</li><!----><li>f</li>')
})

test('when: switches branches, disposes effects of the old branch and does not re-render if truthiness is unchanged', () => {
  const r = doc.createElement('div')
  const n = sig(1), inner = sig(0)
  let renders = 0, innerRuns = 0
  when(r, () => n.v > 0, () => {
    renders++
    fx(() => (inner.v, innerRuns++))
    return tpl('<b>pos</b>')()
  }, () => 'neg')
  assert.equal(html(r), '<b>pos</b><!---->')
  batch(() => (n.v = 2))
  assert.equal(renders, 1)
  batch(() => (n.v = -1))
  assert.equal(html(r), 'neg<!---->')
  batch(() => (inner.v = 1))
  assert.equal(innerRuns, 1)
  batch(() => (n.v = 5))
  assert.equal(renders, 2)
})

test('mount + dispose: hand-written counter against the API', () => {
  const T = tpl('<button><!>: <!> × 2 = <!></button>')
  function Counter($p) {
    const n = sig($p.start ?? 0)
    const dbl = drv(() => n.v * 2)
    const r = T(), a0 = r.firstChild, a1 = a0.nextSibling.nextSibling, a2 = a1.nextSibling.nextSibling
    on(r, 'click', () => {
      n.v++
    })
    cls(r, 'hot', () => n.v > 2)
    insert(r, () => $p.label ?? 'Clicks', a0)
    insert(r, () => n.v, a1)
    insert(r, () => dbl.v, a2)
    return r
  }
  const stop = mount(Counter, doc.body, { start: 1 })
  const b = doc.body.firstChild
  assert.equal(b.textContent, 'Clicks: 1 × 2 = 2')
  click(b)
  click(b)
  assert.equal(b.textContent, 'Clicks: 3 × 2 = 6')
  assert.ok(b.classList.contains('hot'))
  stop()
  assert.equal(doc.body.childNodes.length, 0)
})

test('#216 computed URL attributes get the base path (<meta name="brain-base">; __brainBase during SSR)', async () => {
  const m = document.createElement('meta')
  m.setAttribute('name', 'brain-base')
  m.setAttribute('content', '/app/')
  document.head.appendChild(m)
  const id = sig('hello-world')
  const a = document.createElement('a'), img = document.createElement('img'), f = document.createElement('form'), t = document.createElement('a')
  attr(a, 'href', () => `/posts/${id.v}`)
  attr(img, 'src', () => '/app/_b/logo.png') // already under the base
  attr(f, 'action', () => '//cdn.test/x') // protocol-relative: another host
  attr(t, 'title', () => '/posts/1') // not a URL attribute
  assert.equal(a.getAttribute('href'), '/app/posts/hello-world')
  assert.equal(img.getAttribute('src'), '/app/_b/logo.png')
  assert.equal(f.getAttribute('action'), '//cdn.test/x')
  assert.equal(t.getAttribute('title'), '/posts/1')
  id.v = 'next'
  await new Promise(r => setTimeout(r, 0))
  assert.equal(a.getAttribute('href'), '/app/posts/next')
  m.remove()
  const b = document.createElement('a')
  attr(b, 'href', () => '/posts/1')
  assert.equal(b.getAttribute('href'), '/posts/1') // no base: unchanged
  globalThis.__brainBase = '/site/'
  const c = document.createElement('a')
  attr(c, 'href', () => '/about')
  assert.equal(c.getAttribute('href'), '/site/about')
  delete globalThis.__brainBase
})

test('#169 a write with an equal value (Object.is) runs nothing downstream, NaN included', async () => {
  const n = sig(NaN)
  let runs = 0
  fx(() => (n.v, runs++))
  n.v = NaN
  await new Promise(r => setTimeout(r, 0))
  assert.equal(runs, 1)
  n.v = 1
  await new Promise(r => setTimeout(r, 0))
  assert.equal(runs, 2)
})

test('QA #21 F3: aria-* (and data-*) booleans render "true"/"false"; HTML boolean attributes stay present/absent', async () => {
  const el = doc.createElement('button'), open = sig(false)
  attr(el, 'aria-expanded', () => open.v)
  attr(el, 'aria-pressed', () => !open.v)
  attr(el, 'data-on', () => open.v)
  attr(el, 'disabled', () => open.v)
  assert.deepEqual([el.getAttribute('aria-expanded'), el.getAttribute('aria-pressed'), el.getAttribute('data-on'), el.hasAttribute('disabled')], ['false', 'true', 'false', false])
  open.v = true
  await new Promise(r => setTimeout(r, 0))
  assert.deepEqual([el.getAttribute('aria-expanded'), el.getAttribute('aria-pressed'), el.getAttribute('disabled')], ['true', 'false', ''])
  attr(el, 'aria-label', () => null)
  assert.equal(el.hasAttribute('aria-label'), false) // null still removes
})

test('#94 B0213: javascript:, vbscript: and data: (not data:image/) in a bound URL are dropped and reported', () => {
  const errs = [], e0 = console.error
  console.error = e => errs.push(String((e && e.message) || e))
  const a = doc.createElement('a'), img = doc.createElement('img'), f = doc.createElement('form')
  attr(a, 'href', () => ' JavaScript:alert(1)')
  attr(img, 'src', () => 'data:image/png;base64,AAA')
  attr(f, 'action', () => 'data:text/html,<script>x</script>')
  const ok = doc.createElement('a')
  attr(ok, 'href', () => 'https://x.test/a?javascript:1')
  console.error = e0
  assert.equal(a.getAttribute('href'), '')
  assert.equal(img.getAttribute('src'), 'data:image/png;base64,AAA') // images stay
  assert.equal(f.getAttribute('action'), '')
  assert.equal(ok.getAttribute('href'), 'https://x.test/a?javascript:1')
  assert.equal(errs.filter(m => m.startsWith('B0213')).length, 2)
})

test('TOOLCHAIN #24: a delegated handler sees currentTarget as a data property (a getter would be cached in the shared event map and keep the first component alive)', () => {
  const b = doc.createElement('button')
  doc.body.appendChild(b)
  let seen = null, desc = null
  on(b, 'click', e => ((seen = e.currentTarget), (desc = Object.getOwnPropertyDescriptor(e, 'currentTarget'))))
  click(b)
  assert.equal(seen, b)
  assert.ok(desc && 'value' in desc && !desc.get)
  b.remove()
})

test('delegation: many on() calls for one type add one document listener, so a click runs each handler once (#288)', () => {
  const a = doc.createElement('button'), b = doc.createElement('button'), log = []
  doc.body.appendChild(a), doc.body.appendChild(b)
  on(a, 'click', () => log.push('a'))
  on(b, 'click', () => log.push('b'))
  on(a, 'click', () => log.push('a2')) // replaces the first handler of a
  click(a)
  click(b)
  assert.deepEqual(log, ['a2', 'b'])
})
