// brain-form.js (INTERNALS §14): schema mini-language, blur/submit validation, automatic aria, busy, server errors.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { install, Event } from '../shim/dom.js'

const doc = install()
const rt = await import('./rt.js')
const { tpl, attr, on, insert, mount, sibling } = rt
const { form, rule, check } = await sibling('brain-form.js')
const tick = () => new Promise(r => setTimeout(r, 0))

test('rule + check: types, required, bounds, lists and messages', () => {
  const c = (r, v) => check(rule(r), v)
  assert.deepEqual(rule('str!<=80'), { t: 'str', req: true, list: false, min: null, max: 80 })
  assert.equal(c('str!', ''), 'Required')
  assert.equal(c('str', ''), '') // optional and empty: other checks skipped
  assert.equal(c('str<=3', 'abcd'), 'At most 3 characters')
  assert.equal(c('str>=1', 'x'), '')
  assert.equal(c('email!', 'a@b'), 'Enter a valid email')
  assert.equal(c('email', 'a@b.co'), '')
  assert.equal(c('num>=18', '17'), 'Must be at least 18')
  assert.equal(c('num<=5', 6), 'Must be at most 5')
  assert.equal(c('num', 'x1'), 'Enter a number')
  assert.equal(c('url', 'javascript:alert(1)'), 'Enter a valid URL')
  assert.equal(c('url', 'https://x.test/a'), '')
  assert.equal(c('date!', 'nope'), 'Enter a valid date')
  assert.equal(c('bool!', false), 'Required')
  assert.equal(c('[str]>=1', []), 'Choose at least 1')
  assert.equal(c('[email]', ['a@b.co', 'bad']), 'Item 2: Enter a valid email')
  assert.equal(c('[num]<=2', [1, 2, 3]), 'At most 2 items')
  assert.throws(() => rule('string!'.replace('string', 'str?')), /invalid rule/)
  assert.throws(() => c('colour', 'x'), /unknown type/)
})

// A form like the codegen emits: bind:value → attr + on:input writing f.v.x; on:submit={f.submit}
function Signup(f) {
  const r = tpl('<form><input name="email" aria-label="Email"><input name="age" aria-label="Age"><p id="E"><!></p><button>Go</button></form>')()
  const [email, age, p, btn] = r.childNodes
  attr(email, 'value', () => f.v.email)
  on(email, 'input', e => (f.v.email = e.target.value))
  attr(age, 'value', () => f.v.age)
  on(age, 'input', e => (f.v.age = e.target.value))
  insert(p, () => f.err.email, p.firstChild)
  attr(btn, 'disabled', () => f.busy)
  on(r, 'submit', e => f.submit(e))
  return r
}
const type = (el, v) => {
  el.focus()
  el.value = v
  el.dispatchEvent(new Event('input', { bubbles: true }))
}

test('errors show after blur, live while fixing; aria-invalid and aria-describedby follow the error', async () => {
  let f
  const host = doc.createElement('div')
  doc.body.appendChild(host)
  mount(() => ((f = form({ email: 'email!', age: 'num>=18' })), Signup(f)), host)
  const [email, , p] = host.firstChild.childNodes
  p.setAttribute('id', f.id('email')) // what <Field> renders
  type(email, 'a@')
  await tick()
  assert.equal(p.textContent, '') // not shown while typing the first time
  email.dispatchEvent(new Event('focusout', { bubbles: true }))
  await tick()
  assert.equal(p.textContent, 'Enter a valid email')
  assert.equal(email.getAttribute('aria-invalid'), 'true')
  assert.equal(email.getAttribute('aria-describedby'), f.id('email'))
  type(email, 'a@b.co')
  await tick()
  assert.equal(p.textContent, '')
  assert.equal(email.hasAttribute('aria-invalid'), false)
  assert.equal(email.hasAttribute('aria-describedby'), false)
  assert.equal(f.ok, true)
  doc.body.removeChild(host)
})

test('submit: validates all, focuses the first invalid control, converts values, no re-entry while busy', async () => {
  let f, calls = [], release
  const host = doc.createElement('div')
  doc.body.appendChild(host)
  mount(() => ((f = form({ email: 'email!', age: 'num>=18' }, { submit: v => (calls.push(v), new Promise(r => (release = r))) })), Signup(f)), host)
  const fm = host.firstChild
  const [email, age, , btn] = fm.childNodes
  const send = () => fm.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }))
  send()
  await tick()
  assert.equal(calls.length, 0)
  assert.equal(doc.activeElement, email) // found through form.elements even though never typed in
  assert.equal(f.err.email, 'Required')
  assert.equal(age.hasAttribute('aria-invalid'), false) // optional, empty
  type(email, 'a@b.co')
  type(age, '20')
  send()
  send() // double submit
  await tick()
  assert.equal(calls.length, 1)
  assert.deepEqual(calls[0], { email: 'a@b.co', age: 20 })
  assert.equal(f.busy, true)
  assert.equal(btn.hasAttribute('disabled'), true)
  release({ id: 1 })
  await tick()
  assert.equal(f.busy, false)
  assert.equal(btn.hasAttribute('disabled'), false)
  doc.body.removeChild(host)
})

test('server errors: returned or thrown {errors} mark fields; other failures set f.error; reset clears', async () => {
  let mode = 'ret'
  const f = form({ email: 'email!', name: 'str' }, {
    init: { email: 'a@b.co' },
    submit: async () => {
      if (mode === 'ret') return { errors: { email: 'Already taken' } }
      if (mode === 'throw') throw Object.assign(Error('422'), { errors: { name: 'Too short' } })
      throw Error('Network down')
    },
  })
  await f.submit()
  assert.equal(f.err.email, 'Already taken')
  mode = 'throw'
  await f.submit()
  assert.equal(f.err.name, 'Too short')
  mode = 'boom'
  await f.submit()
  assert.equal(f.error, 'Network down')
  f.reset()
  assert.equal(f.v.email, '')
  assert.equal(f.err.email, '') // untouched again
  assert.equal(f.error, '')
})

test('nested schema, lists, bool, cross-field validate and values()', async () => {
  const Address = { city: 'str!', zip: 'str<=5' }
  let sent
  const f = form(
    { name: 'str!', address: Address, tags: '[str]<=2', terms: 'bool!', pw: 'str!', pw2: 'str!' },
    { validate: v => (v.pw !== v.pw2 ? { pw2: 'Passwords do not match' } : null), submit: v => (sent = v) }
  )
  f.v.name = 'Ada'
  f.v.address.city = 'Puebla'
  f.v.tags = ['a']
  f.v.pw = 'x'
  f.v.pw2 = 'y'
  await f.submit()
  assert.equal(f.err.terms, 'Required')
  assert.equal(sent, undefined)
  f.v.terms = true
  await f.submit()
  assert.equal(f.err.pw2, 'Passwords do not match')
  f.v.pw2 = 'x'
  await f.submit()
  assert.deepEqual(sent, { name: 'Ada', address: { city: 'Puebla', zip: '' }, tags: ['a'], terms: true, pw: 'x', pw2: 'x' })
  f.v.address.zip = '123456'
  f.touch('address.zip')
  assert.equal(f.err.address.zip, 'At most 5 characters')
})

test('#156 a form posted without JS starts from the server state (values, field errors shown, form error)', () => {
  const save = Object.assign(async () => {}, { id: 'h1/save' })
  globalThis.__brainForms = { 'h1/save': { values: { email: 'ada@', age: 16 }, errors: { email: 'Enter a valid email' }, error: 'Fix the fields' } }
  const f = form({ email: 'email!', age: 'num>=18' }, { submit: save })
  globalThis.__brainForms = null
  assert.equal(f.v.email, 'ada@')
  assert.equal(f.v.age, 16)
  assert.equal(f.err.email, 'Enter a valid email') // already shown, no blur needed
  assert.equal(f.err.age, '') // not reported by the server: stays untouched
  assert.equal(f.error, 'Fix the fields')
  const other = form({ email: 'email!' }, { submit: Object.assign(async () => {}, { id: 'h1/other' }) })
  assert.equal(other.v.email, '') // another form on the page is not affected
})

test('#169 async check: runs after the field rule passes (blur and submit), stale answers ignored, blocks submit', async () => {
  const taken = new Set(['ada@x.test'])
  let sent = null, calls = 0
  const f = form({ email: 'email!' }, {
    check: { email: async v => (calls++, await tick(), taken.has(v) ? 'Taken' : '') },
    submit: async v => (sent = v),
  })
  f.v.email = 'bad'
  f.touch('email')
  assert.equal(f.err.email, 'Enter a valid email')
  assert.equal(calls, 0) // the field's own rule failed: no remote check
  f.v.email = 'ada@x.test'
  f.touch('email')
  assert.equal(f.checking, true)
  await tick()
  await tick()
  assert.equal(f.err.email, 'Taken')
  assert.equal(f.checking, false)
  await f.submit()
  assert.equal(sent, null) // blocked by the async check
  f.v.email = 'grace@x.test'
  await f.submit()
  assert.deepEqual(sent, { email: 'grace@x.test' })
  // stale: the value changes while the check is in flight
  f.v.email = 'ada@x.test'
  f.touch('email')
  f.v.email = 'new@x.test'
  await tick()
  await tick()
  assert.equal(f.err.email, '')
})

test('#169 dirty: true after a change, false after a successful submit or reset; a cancelled submit shows no error', async () => {
  let fail = null
  const f = form({ name: 'str' }, { init: { name: 'Ada' }, submit: async () => { if (fail) throw fail } })
  assert.equal(f.dirty, false)
  f.v.name = 'Grace'
  assert.equal(f.dirty, true)
  await f.submit()
  assert.equal(f.dirty, false)
  f.v.name = 'Linus'
  fail = Object.assign(new Error('The request was cancelled.'), { kind: 'aborted' })
  await f.submit()
  assert.equal(f.error, '') // aborted: not an error to show
  assert.equal(f.dirty, true)
  f.reset({ name: 'Ada' })
  assert.equal(f.dirty, false)
  assert.ok(globalThis.__brainDirty.has(f))
})

test('#299: while submit() runs the form is busy, so a navigation it starts on success is not asked about', async () => {
  let during = null
  const f = form({ name: 'str' }, { init: { name: 'Ada' }, submit: async () => (during = { dirty: f.dirty, busy: f.busy }) })
  f.v.name = 'Grace'
  await f.submit()
  assert.deepEqual(during, { dirty: true, busy: true }) // the leave guards skip it (dirty && !busy)
  assert.deepEqual([f.dirty, f.busy], [false, false])
})

test('#94 submit marks the form aria-busy while it works and clears it after', async () => {
  let release
  const f = form({ name: 'str' }, { submit: () => new Promise(r => (release = r)) })
  const fm = doc.createElement('form')
  const p = f.submit({ preventDefault() {}, target: fm })
  await tick()
  assert.equal(fm.getAttribute('aria-busy'), 'true')
  release()
  await p
  assert.equal(fm.hasAttribute('aria-busy'), false)
})

test('QA #157: with JS, form() matches the no-JS flow: a { redirect } navigates; a 5xx shows the generic message, a 4xx its own', async () => {
  const went = []
  globalThis.__brainNavigate = u => went.push(u)
  const ok = form({ name: 'str' }, { submit: async () => ({ redirect: '/about' }) })
  await ok.submit()
  assert.deepEqual(went, ['/about'])
  const ext = form({ name: 'str' }, { submit: async () => ({ redirect: '//evil.test/x' }) })
  await ext.submit()
  assert.deepEqual(went, ['/about']) // only same-origin paths, like callForm
  delete globalThis.__brainNavigate
  const err = status => Object.assign(new Error(status >= 500 ? 'Internal error' : 'Email already used'), { code: 'rpc', status })
  const f500 = form({ name: 'str' }, { submit: async () => { throw err(500) } })
  await f500.submit()
  assert.equal(f500.error, 'Something went wrong. Try again.')
  const f409 = form({ name: 'str' }, { submit: async () => { throw err(409) } })
  await f409.submit()
  assert.equal(f409.error, 'Email already used')
  const net = form({ name: 'str' }, { submit: async () => { throw new TypeError('fetch failed') } }) // what brain-rpc rethrows
  await net.submit()
  assert.equal(net.error, 'Something went wrong. Try again.')
  const own = form({ name: 'str' }, { submit: async () => { throw new Error('Pick a date first') } })
  await own.submit()
  assert.equal(own.error, 'Pick a date first') // the author's own message stays
})

test('F10: replacing f.v or f.err throws an actionable error; the fields stay bound and reset() loads values', async () => {
  let sent = null
  const f = form({ email: 'email!', name: 'str' }, { submit: async v => (sent = v) })
  const live = f.v
  assert.throws(() => (f.v = { email: 'a@b.co', name: 'Ada' }), /f\.v.*cannot be replaced.*form\(schema, \{ init \}\).*f\.reset\(values\)/)
  assert.throws(() => (f.err = {}), /f\.err.*cannot be replaced/)
  assert.equal(f.v, live) // still the object the inputs are bound to
  f.reset({ email: 'a@b.co', name: 'Ada' })
  await f.submit()
  assert.deepEqual(sent, { email: 'a@b.co', name: 'Ada' }) // the loaded values reach submit, no 'Required'
  assert.equal(f.err.email, '')
})

test('F12: validation messages come from the page dictionary (form.* keys) and fall back to English', async () => {
  const c = (r, v) => check(rule(r), v)
  ;(globalThis.__brainDict ||= {}).xx = {
    'form.required': 'Needed here',
    'form.min_chars': { one: 'One character minimum', other: '{n} characters minimum' },
    'form.item': 'Entry {n}: {error}',
    'form.error': 'The server failed',
  }
  globalThis.__brainLocale = 'xx'
  try {
    assert.equal(c('str!', ''), 'Needed here')
    assert.equal(c('bool!', false), 'Needed here')
    assert.equal(c('str>=3', 'ab'), '3 characters minimum') // plural object chosen by n
    assert.equal(c('str>=1', ''), '') // optional and empty
    assert.equal(c('[str!]', ['ok', '']), 'Entry 2: Needed here')
    assert.equal(c('email', 'a@b'), 'Enter a valid email') // key missing: English text
    const f = form({ name: 'str!' }, { submit: async () => { throw Object.assign(new Error('x'), { status: 500 }) } })
    await f.submit()
    assert.equal(f.err.name, 'Needed here')
    f.v.name = 'Ada'
    await f.submit()
    assert.equal(f.error, 'The server failed')
  } finally {
    delete globalThis.__brainDict.xx
    globalThis.__brainLocale = null
  }
  assert.equal(c('str!', ''), 'Required')
})
