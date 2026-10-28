// #159 brain run --mock: fake() makes deterministic data from api() schemas; http({ mock }) answers api() with it.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { install } from '../shim/dom.js'

install()
const rt = await import('./rt.js')
const { api, http } = await rt.sibling('brain-data.js')
const { fake, mocker } = await rt.sibling('brain-mock.js')
const { rule, check } = await rt.sibling('brain-schema.js')
const User = { id: 'num!', name: 'str!', email: 'email!', city: 'str', joined: 'date', bio: 'str<=40', age: 'num>=18', active: 'bool', tags: '[str]' }
const valid = (s, v) => Object.keys(s).every(k => !check(rule(s[k]), v[k]))

test('fake(): the same schema and seed give the same JSON; another seed gives other data; every value passes its rule', () => {
  const a = JSON.stringify(fake([User], 'seed-1')), b = JSON.stringify(fake([User], 'seed-1')), c = JSON.stringify(fake([User], 'seed-2'))
  assert.equal(a, b)
  assert.notEqual(a, c)
  const list = fake([User], 'seed-1')
  assert.ok(list.length >= 3 && list.length <= 6)
  for (const u of list) assert.ok(valid(User, u), JSON.stringify(u))
  assert.deepEqual(list.map(u => u.id), list.map((_, i) => i + 1)) // ids unique and stable in a list
  const u = list[0]
  assert.equal(u.email, u.name.toLowerCase().replace(' ', '.').replace(/[^a-z.]/g, '') + '@example.com') // one person per object
  assert.ok(u.bio.length <= 40 && u.age >= 18)
  assert.match(u.joined, /^\d{4}-\d{2}-\d{2}$/)
  assert.deepEqual(fake({ post: { title: 'str!', author: { name: 'str!' } } }, 'x'), fake({ post: { title: 'str!', author: { name: 'str!' } } }, 'x'))
})

test('http({ mock: mocker(seed) }): api() answers from `out` without the network, per URL; writes echo the input; off again', async () => {
  let calls = 0
  globalThis.fetch = async () => (calls++, { ok: true, status: 200, text: async () => '[]' })
  http({ mock: mocker('demo') })
  const users = api('/users', { get: { out: [User] }, post: { in: { name: 'str!' }, out: User } })
  const one = api('/users/:id', { get: { out: User } })
  const l1 = await users.get(), l2 = await users.get()
  assert.deepEqual(l1, l2)
  assert.notDeepEqual(await one.get({ id: 1 }), await one.get({ id: 2 })) // seeded by URL
  const made = await users.post({ name: 'Grace Hopper' })
  assert.equal(made.name, 'Grace Hopper')
  assert.ok(valid(User, made))
  assert.equal(calls, 0)
  http({ mock: null })
  await users.get()
  assert.equal(calls, 1)
})

test('brain-data does not import brain-mock (fake data never ships to production)', async () => {
  const { readFileSync } = await import('node:fs')
  assert.doesNotMatch(readFileSync(new URL('../brain-data.js', import.meta.url), 'utf8'), /^\s*import[^\n]*brain-mock|import\(['"][^'"]*brain-mock/m)
})
