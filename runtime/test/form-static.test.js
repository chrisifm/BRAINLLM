import test from 'node:test'
import assert from 'node:assert/strict'
import { install } from '../shim/dom.js'
install()
const { mount, tpl, sibling } = await import('./rt.js')
const { form, formStatic, formScalars, formStaticSync, formScalarsSync } = await sibling('brain-form.js')
const schema = { email: 'email!', password: 'str!', nickname: 'str', backup: 'email' }
const compiled = { email: ['email', true], password: ['str', true], nickname: ['str', false], backup: ['email', false] }

test('compiled literal forms preserve values, required/email validation, server errors, reset and cleanup', async () => {
  const traces = []
  for (const [factory, input] of [[form, schema], [formStatic, compiled], [formStaticSync, compiled]]) {
    let value, calls = 0
    const trace = [], dispose = mount(() => {
      value = factory(input, { submit: async values => { calls++; trace.push(values); return { errors: { email: 'Unavailable' } } } })
      return tpl('<div></div>')()
    }, document.body)
    try {
      trace.push(value.ok, value.values())
      value.v.email = 'invalid'; value.touch('email')
      trace.push(value.err.email)
      await value.submit(); trace.push(calls, value.err.password)
      value.v.email = 'person@example.test'; value.v.password = 'example password'
      trace.push(value.ok)
      await value.submit(); trace.push(calls, value.err.email, value.busy)
      value.reset({ email: 'other@example.test', password: '' })
      trace.push(value.values(), value.ok, value.error, value.dirty)
      assert.ok(globalThis.__brainDirty.has(value))
      traces.push(trace)
    } finally { dispose() }
    assert.ok(!globalThis.__brainDirty.has(value))
  }
  for (const trace of traces.slice(1)) assert.deepEqual(trace, traces[0])
})

test('compiled form checks still await async validation and surface remote field errors', async () => {
  let value, submitted = false
  const dispose = mount(() => {
    value = formStatic(compiled, { check: { email: async () => 'Already registered' }, submit: async () => { submitted = true } })
    return tpl('<div></div>')()
  }, document.body)
  try {
    value.v.email = 'person@example.test'; value.v.password = 'example password'
    await value.submit()
    assert.equal(submitted, false)
    assert.equal(value.err.email, 'Already registered')
    assert.equal(value.checking, false)
  } finally { dispose() }
})

test('compiled schema helper refuses malformed metadata instead of admitting unsupported rules', () => {
  for (const input of [null, [], { field: ['date', false] }, { field: ['str', 'yes'] }, { constructor: ['str', false] }, { field: ['str'] }]) {
    assert.throws(() => formStatic(input), /invalid compiled schema/)
  }
})


test('compiled scalar validator exactly matches public check for supported rules and value shapes', async () => {
  const { checkText, checkScalar, check, rule } = await sibling('brain-schema.js')
  const values = [undefined, null, '', ' ', 'name', 'person@example.test', 'invalid@host', 0, 1, NaN, true, false, [], ['x'], {}, new Date(0), Symbol('example'), Infinity, -Infinity, '4.5', ' 7.25 ', '-.2e+3', '0x10', '1_000', 'Infinity', '1,2', '1e999']
  for (const type of ['str', 'str!', 'email', 'email!', 'bool', 'bool!', 'num', 'num!']) {
    const parsed = rule(type)
    for (const value of values) assert.equal((type.startsWith('num') || type.startsWith('bool') ? checkScalar : checkText)(parsed, value), check(parsed, value), type)
  }
})


test('public form rule/check aliases retain the exact schema function identities and arity', async () => {
  const forms = await sibling('brain-form.js'), schema = await sibling('brain-schema.js')
  assert.equal(forms.rule, schema.rule)
  assert.equal(forms.check, schema.check)
  assert.equal(forms.rule.length, schema.rule.length)
  assert.equal(forms.check.length, schema.check.length)
})


test('compiled boolean and numeric forms preserve initialization, validation and submit coercion', async () => {
  const traces = []
  for (const factory of [form, formScalars, formScalarsSync]) {
    let f
    const trace = [], schema = factory === form
      ? { accepted: 'bool!', optional: 'bool', score: 'num!', extra: 'num' }
      : { accepted: ['bool', true], optional: ['bool', false], score: ['num', true], extra: ['num', false] }
    const dispose = mount(() => {
      f = factory(schema, { submit: async values => { trace.push(values); return {} } })
      return tpl('<div></div>')()
    }, document.body)
    try {
      trace.push(f.values(), f.ok)
      await f.submit(); trace.push(f.err.accepted, f.err.score)
      f.v.accepted = true; f.v.score = ' 4.5 '; f.v.extra = ''
      trace.push(f.ok); await f.submit()
      f.v.score = 7.25; f.v.optional = false; f.v.extra = '-.2e+3'
      await f.submit()
      f.v.score = '0x10'; await f.submit(); trace.push(f.err.score)
      f.reset(); trace.push(f.values(), f.ok)
      traces.push(trace)
    } finally { dispose() }
  }
  for (const trace of traces.slice(1)) assert.deepEqual(trace, traces[0])
  assert.deepEqual(traces[0][5], { accepted: true, optional: false, score: 4.5, extra: null })
  assert.deepEqual(traces[0][6], { accepted: true, optional: false, score: 7.25, extra: -200 })
})


test('compiler-only sync forms reject unexpected async/cross-field options instead of dropping validation', () => {
  for (const factory of [formStaticSync, formScalarsSync]) {
    for (const options of [{check:{}},{validate:()=>({})},Object.create({check:{}})]) {
      assert.throws(()=>factory({name:['str',false]},options),/invalid compiled synchronous options/)
    }
  }
})

test('compiled synchronous forms retain busy lock, scalar errors, server state and owner cleanup', async () => {
  const traces=[]
  for (const factory of [form,formScalarsSync]) {
    let f,release,calls=0
    const result=[],schema=factory===form?{accepted:'bool!',amount:'num!',title:'str!'}:{accepted:['bool',true],amount:['num',true],title:['str',true]}
    const submit=async values=>{calls++;await new Promise(resolve=>release=resolve);return {errors:{title:'Already exists'}}}
    submit.id='sync-fixture/save'
    globalThis.__brainForms={'sync-fixture/save':{values:{accepted:false,amount:'0x10',title:''},errors:{title:'Required'},error:'Retry'}}
    const dispose=mount(()=>{f=factory(schema,{submit});return tpl('<div></div>')()},document.body)
    globalThis.__brainForms=null
    try {
      result.push(f.values(),f.err.title,f.error,f.checking)
      f.v.accepted=true;f.v.amount='4.5';f.v.title='Example'
      const first=f.submit();await Promise.resolve()
      await f.submit();result.push(calls,f.busy,f.checking)
      release();await first;result.push(f.busy,f.err.title,f.dirty)
      f.errors({amount:'Not available'});f.reset({accepted:true,amount:7.25,title:'Other'})
      result.push(f.values(),f.err.amount,f.dirty,f.ok)
      traces.push(result)
    } finally {dispose()}
    assert.ok(!globalThis.__brainDirty.has(f))
  }
  assert.deepEqual(traces[1],traces[0])
})

test('dynamic compiled options keep checks installed through the submit receiver', async () => {
  let f,calls=0
  const options={submit:function(){calls++;this.check={name:async()=> 'Unavailable'}}}
  const dispose=mount(()=>{f=formStatic({name:['str',true]},options);return tpl('<div></div>')()},document.body)
  try {
    f.v.name='Example'
    await f.submit();assert.equal(calls,1)
    await f.submit();assert.equal(calls,1)
    assert.equal(f.err.name,'Unavailable');assert.equal(f.checking,false)
  } finally {dispose()}
})
