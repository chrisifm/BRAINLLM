// brain/ui kit + form() from the REAL compiler output (fixtures/kit/out, regen.sh): Input/Select/Checkbox/Button
// with form validation, Tabs, virtualized Table and Toast on a bus.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { cpSync, mkdtempSync, readdirSync, readFileSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { pathToFileURL } from 'node:url'
import { install, Event, MouseEvent } from '../shim/dom.js'

const doc = install()
const { mount, base } = await import('./rt.js')
const src = new URL('./fixtures/kit/out/', import.meta.url)
const dir = mkdtempSync(join(tmpdir(), 'brain-kit-'))
cpSync(src, dir, { recursive: true })
for (const f of [...readdirSync(dir).map(n => join(dir, n)), ...readdirSync(join(dir, 'ui')).map(n => join(dir, 'ui', n))])
  if (f.endsWith('.js')) writeFileSync(f, readFileSync(f, 'utf8').replaceAll('__RT__/', base))
const { default: View } = await import(pathToFileURL(join(dir, 'form.brain.js')).href)

const tick = (ms = 0) => new Promise(r => setTimeout(r, ms))
const click = el => el.dispatchEvent(new MouseEvent('click', { bubbles: true, cancelable: true }))
const type = (el, v, ev = 'input') => {
  el.focus()
  el.value = v
  el.dispatchEvent(new Event(ev, { bubbles: true }))
}

test('kit form: errors on submit, focus on first invalid, live fix, busy without re-entry, toast', async () => {
  const host = doc.createElement('div')
  doc.body.appendChild(host)
  const stop = mount(View, host, {})
  const fm = host.querySelector('form')
  const email = fm.querySelector('input[name=email]'), sel = fm.querySelector('select'), chk = fm.querySelector('input[type=checkbox]')
  const btn = fm.querySelector('button[type=submit]')
  const errs = () => fm.querySelectorAll('.error').filter(p => !p.hidden && p.textContent).map(p => p.textContent)
  assert.equal(email.getAttribute('aria-label'), 'Email')
  assert.deepEqual(errs(), [])
  const submit = () => fm.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }))
  submit()
  await tick()
  assert.deepEqual(errs(), ['Required', 'Required', 'Required'])
  assert.equal(doc.activeElement, email)
  assert.equal(email.getAttribute('aria-invalid'), 'true')
  assert.ok(email.getAttribute('aria-describedby').includes('err'), 'Field links the error text')
  type(email, 'ada@')
  await tick()
  assert.deepEqual(errs()[0], 'Enter a valid email') // live once shown
  type(email, 'ada@x.dev')
  type(sel, 'Pro', 'change')
  chk.checked = true
  chk.dispatchEvent(new Event('change', { bubbles: true }))
  await tick()
  assert.deepEqual(errs(), [])
  assert.equal(email.hasAttribute('aria-invalid'), false)
  submit()
  submit() // double submit while busy
  await tick()
  assert.equal(btn.hasAttribute('disabled'), true)
  assert.equal(btn.getAttribute('aria-busy'), 'true')
  await tick(40)
  assert.equal(btn.hasAttribute('disabled'), false)
  const toasts = host.querySelectorAll('.toast span').map(s => s.textContent)
  assert.deepEqual(toasts, ['Saved ada@x.dev']) // one submit, one toast
  await tick(80)
  assert.equal(host.querySelectorAll('.toast').length, 0) // auto-dismissed
  stop()
  doc.body.removeChild(host)
})

test('kit Tabs and Table: tab switch with arrows, 300 rows virtualized, sort with aria-sort', async () => {
  const host = doc.createElement('div')
  doc.body.appendChild(host)
  const stop = mount(View, host, {})
  const tabs = host.querySelectorAll('[role=tab]')
  const panel = () => host.querySelector('[role=tabpanel]').textContent.trim()
  assert.equal(panel(), 'Overview panel')
  tabs[0].focus()
  tabs[0].dispatchEvent(new KeyboardEventLike('ArrowRight'))
  await tick()
  assert.equal(panel(), 'Details panel')
  assert.equal(tabs[1].getAttribute('aria-selected'), 'true')
  assert.equal(doc.activeElement, tabs[1])
  const rows = () => host.querySelectorAll('tbody tr').filter(r => !r.classList.contains('pad'))
  assert.ok(rows().length < 60, `virtualized: ${rows().length} rows in the DOM for 300`)
  assert.equal(host.querySelector('table').getAttribute('aria-rowcount'), '301')
  const score = host.querySelectorAll('th button')[2]
  click(score)
  click(score)
  await tick()
  assert.equal(score.parentNode.getAttribute('aria-sort'), 'descending')
  assert.equal(rows()[0].childNodes.filter(n => n.nodeType === 1)[2].textContent, '100')
  stop()
  doc.body.removeChild(host)
})

function KeyboardEventLike(key) {
  return Object.assign(new Event('keydown', { bubbles: true, cancelable: true }), { key })
}

test('#168 kit ErrorState: message and action per kind; Retry only where it helps; auto-retry when back online', async () => {
  const { default: ErrorState } = await import(pathToFileURL(join(dir, 'ui', 'error-state.brain.js')).href)
  const show = (error, retry) => {
    const host = doc.createElement('div')
    doc.body.appendChild(host)
    mount(ErrorState, host, { error, retry })
    return host
  }
  let n = 0
  const retry = () => n++
  const off = show({ kind: 'offline' }, retry)
  assert.match(off.textContent, /You are offline\./)
  click(off.querySelector('button'))
  assert.equal(n, 1)
  const h403 = show({ kind: 'http', status: 403, message: 'admins only' }, retry)
  assert.match(h403.textContent, /You do not have permission: admins only/)
  assert.equal(h403.querySelector('button'), null) // retrying a 403 does not help
  const h401 = show({ kind: 'http', status: 401 }, retry)
  assert.match(h401.querySelector('a').getAttribute('href'), /^\/login/)
  const h500 = show({ kind: 'http', status: 500, id: 'rid-9' }, retry)
  assert.match(h500.textContent, /Something went wrong on our side\.[\s\S]*Reference: rid-9/)
  assert.ok(h500.querySelector('button'))
  assert.equal(h500.firstChild.getAttribute('role'), 'alert')
  assert.equal(show({ kind: 'aborted' }, retry).textContent.trim(), '') // a cancelled request shows nothing
  // back online → the offline one retries by itself
  const ls = new Map()
  globalThis.addEventListener = (t, f) => ls.set(t, f)
  globalThis.removeEventListener = t => ls.delete(t)
  n = 0
  show({ kind: 'network' }, retry)
  ls.get('online')()
  assert.equal(n, 1)
})

test('#82 layout primitives: token-only spacing (gap="3" → var(--space-3)), flow and widths without writing CSS', async () => {
  const load = async n => (await import(pathToFileURL(join(dir, 'ui', n + '.brain.js')).href)).default
  const [Stack, Row, Grid, Center, Container, Sidebar, Spacer] = await Promise.all(['stack', 'row', 'grid', 'center', 'container', 'sidebar', 'spacer'].map(load))
  const put = (C, props) => {
    const h = doc.createElement('div')
    doc.body.appendChild(h)
    mount(C, h, props)
    return h.firstElementChild || h.firstChild
  }
  const st = (el, k) => el.style.getPropertyValue(k)
  assert.equal(st(put(Stack, {}), 'gap'), 'var(--space-3)')
  assert.equal(st(put(Stack, { gap: '0', align: 'center' }), 'gap'), '0')
  const row = put(Row, { gap: '5', justify: 'between', wrap: false })
  assert.deepEqual([st(row, 'gap'), st(row, 'justify-content'), row.classList.contains('nowrap')], ['var(--space-5)', 'space-between', true])
  assert.equal(st(put(Grid, { cols: 3 }), 'grid-template-columns'), 'repeat(3, minmax(0, 1fr))')
  assert.equal(st(put(Grid, {}), 'grid-template-columns'), 'repeat(auto-fit, minmax(min(16rem, 100%), 1fr))')
  assert.deepEqual([st(put(Center, {}), 'max-width'), st(put(Center, {}), 'padding-inline')], ['60ch', 'var(--space-4)'])
  assert.equal(st(put(Container, { size: 'sm' }), 'max-width'), '40rem')
  assert.equal(st(put(Container, { size: 'nope' }), 'max-width'), '72rem') // unknown size → lg
  const sb = put(Sidebar, { side: 'right', width: '20rem' })
  assert.ok(sb.classList.contains('right'))
  assert.equal(st(sb.querySelector('.side'), 'flex-basis'), '20rem')
  const sp = put(Spacer, {})
  assert.ok(sp.classList.contains('grow'))
  assert.equal(st(put(Spacer, { size: '2' }), 'flex-basis'), 'var(--space-2)')
})

test('QA #153 LangSwitch (no state of its own): links per language, the one-time hint closes by hiding itself', async () => {
  const { default: LangSwitch } = await import(pathToFileURL(join(dir, 'ui', 'lang-switch.brain.js')).href)
  doc.documentElement.setAttribute('lang', 'en')
  globalThis.location = new URL('https://site.test/about')
  doc.head.innerHTML = '<link rel="alternate" hreflang="en" href="/about"><link rel="alternate" hreflang="nl" href="/nl/over">'
  const ls = new Map()
  globalThis.localStorage = { getItem: k => ls.get(k) ?? null, setItem: (k, v) => ls.set(k, v) }
  const nav0 = Object.getOwnPropertyDescriptor(globalThis, 'navigator')
  Object.defineProperty(globalThis, 'navigator', { configurable: true, value: { languages: ['nl'] } })
  const h = doc.createElement('div')
  doc.body.appendChild(h)
  mount(LangSwitch, h, {})
  assert.deepEqual([...h.querySelectorAll('ul a')].map(a => a.getAttribute('hreflang')), ['en', 'nl'])
  const hint = h.querySelector('.hint')
  assert.ok(hint && !hint.hasAttribute('hidden'))
  click(hint.querySelector('button'))
  assert.ok(hint.hasAttribute('hidden'))
  assert.equal(ls.get('brain:lang-suggested'), '1') // remembered: not offered again
  if (nav0) Object.defineProperty(globalThis, 'navigator', nav0)
  doc.head.innerHTML = ''
})
