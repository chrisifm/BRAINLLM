import { test } from 'node:test'
import assert from 'node:assert/strict'
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, rmSync } from 'node:fs'
import { join } from 'node:path'
import { tmpdir } from 'node:os'
import { collect, normalize, render, sync } from '../sync-contributors.mjs'
const account = (login = 'octocat', type = 'User') => ({ login, type, contributions: 1, avatar_url: 'https://avatars.githubusercontent.com/u/1?v=4' })
const response = (rows, headers = {}) => new Response(JSON.stringify(rows), { headers })
test('collect paginates without following arbitrary Link URLs and retains public bot accounts', async () => {
  const calls = []
  const list = await collect(async url => {
    calls.push(url)
    return calls.length === 1 ? response([account()], { link: '<https://untrusted.invalid/>; rel="next"' }) : response([account('github-actions[bot]', 'Bot')])
  })
  assert.equal(calls.length, 2)
  assert.ok(calls.every(url => url.startsWith('https://api.github.com/repos/chrisifm/BRAINLLM/contributors?')))
  assert.equal(list.length, 2)
  assert.match(render('# Project\n', list), /github-actions\\\[bot\\\]/)
  assert.equal(render(render('# Project\n', list), list), render('# Project\n', list))
})
test('invalid, empty and partial API failures preserve existing README and snapshot', async t => {
  const root = mkdtempSync(join(tmpdir(), 'brain-contributors-'))
  t.after(() => rmSync(root, { recursive: true, force: true }))
  writeFileSync(join(root, 'README.md'), 'keep existing source\n')
  mkdirSync(join(root, 'docs/site'), { recursive: true })
  writeFileSync(join(root, 'docs/site/contributors.json'), 'keep snapshot\n')
  for (const fetcher of [async () => response([]), async () => new Response('rate limited', { status: 403 }), async () => response([account('bad<script>')])]) {
    await assert.rejects(sync(root, fetcher))
    assert.equal(readFileSync(join(root, 'README.md'), 'utf8'), 'keep existing source\n')
    assert.equal(readFileSync(join(root, 'docs/site/contributors.json'), 'utf8'), 'keep snapshot\n')
  }
  let page = 0
  await assert.rejects(sync(root, async () => ++page === 1 ? response([account()], { link: '<next>; rel="next"' }) : new Response('', { status: 500 })))
  assert.equal(readFileSync(join(root, 'README.md'), 'utf8'), 'keep existing source\n')
  assert.throws(() => normalize([{ ...account(), avatar_url: 'https://untrusted.invalid/icon.png' }]))
  assert.throws(() => render('<!-- contributors:start -->', normalize([account()])))
})

test('attribution stays stable when only commit counts change', () => {
  const initial = normalize([account('beta'), account('alpha')])
  const updated = normalize([{ ...account('alpha'), contributions: 10 }, { ...account('beta'), contributions: 20 }])
  assert.deepEqual(updated, initial)
  assert.equal(render('# Project\n', updated), render('# Project\n', initial))
})
