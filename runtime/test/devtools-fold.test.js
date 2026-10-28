// devtools/lib/fold.jsbrain (#27): the pure piece of the debugger. Inspector records (FORMAT v2) → debugger state.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { copyFileSync, mkdtempSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { pathToFileURL } from 'node:url'

const tmp = join(mkdtempSync(join(tmpdir(), 'brain-fold-')), 'fold.mjs')
copyFileSync(new URL('../../devtools/lib/fold.jsbrain', import.meta.url), tmp) // a .jsbrain lib is plain JS
const { default: fold } = await import(pathToFileURL(tmp).href)
const S0 = { seq: -1, heap: [], inst: [], comps: [], ev: [], jank: [], inp: [], upd: [], handlers: [], net: [], storage: [], conns: [], timers: [], state: {}, graph: {}, leaks: [], errors: [] }

test('records with an id merge into the earlier one (net, timers, connections, instances); gone removes', () => {
  let s = fold(S0, { seq: 0, d: [{ k: 'net', id: 1, m: 'GET', url: '/a', st: null }, { k: 'timer', id: 7, kind: 'interval', st: 'on' }, { k: 'inst', id: 3, name: 'Card' }] })
  s = fold(s, { seq: 1, d: [{ k: 'net', id: 1, st: 200, ms: 12 }, { k: 'timer', id: 7, st: 'off' }, { k: 'gone', id: 3 }] })
  assert.deepEqual(s.net, [{ k: 'net', id: 1, m: 'GET', url: '/a', st: 200, ms: 12 }])
  assert.equal(s.timers[0].st, 'off')
  assert.deepEqual(s.inst, [])
  s = fold(s, { seq: 2, d: [{ k: 'err', fp: 'x', msg: 'boom', n: 1 }, { k: 'err', fp: 'x', n: 3 }, { k: 'leak', code: 'B0601', h: 'a', msg: 'm' }, { k: 'leak', code: 'B0601', h: 'a', msg: 'm' }] })
  assert.deepEqual(s.errors, [{ k: 'err', fp: 'x', msg: 'boom', n: 3 }])
  assert.equal(s.leaks.length, 1) // the same leak twice is one
})

test('every list keeps at most 300 records, newest first; heap keeps the last 300 samples in time order', () => {
  const d = []
  for (let i = 0; i < 350; i++) d.push({ k: 'upd', at: i, ms: i }, { k: 'net', id: i, url: '/' + i }, { k: 'heap', at: i, used: i })
  const s = fold(S0, { seq: 0, d })
  assert.equal(s.upd.length, 300)
  assert.equal(s.upd[0].at, 349) // newest first
  assert.equal(s.net.length, 300)
  assert.equal(s.heap.length, 300)
  assert.deepEqual([s.heap[0].at, s.heap.at(-1).at], [50, 349])
})

test('a page reload (seq back to 0) resets the live data and keeps the history', () => {
  let s = fold(S0, { seq: 0, d: [{ k: 'inst', id: 1, name: 'A' }, { k: 'comp', h: 'a', inst: 1 }, { k: 'timer', id: 1, st: 'on' }, { k: 'conn', id: 1, st: 'open' }, { k: 'state', id: 1, sigs: [] }, { k: 'net', id: 1, url: '/x' }, { k: 'err', fp: 'e', n: 1 }, { k: 'leak', code: 'B0602', h: 'a', msg: 'm' }, { k: 'heap', at: 1, used: 5 }] })
  s = fold(s, { seq: 5, d: [] })
  s = fold(s, { seq: 0, d: [{ k: 'inst', id: 9, name: 'B' }] }) // the app reloaded
  assert.deepEqual(s.inst.map(i => i.name), ['B'])
  assert.deepEqual([s.comps, s.timers, s.conns, s.state], [[], [], [], {}])
  assert.deepEqual([s.net.length, s.errors.length, s.leaks.length, s.heap.length], [1, 1, 1, 1]) // history stays
  assert.equal(s.seq, 0)
})

test('records out of order: a patch before its record, comps replaced by the newest snapshot, plain lists', () => {
  let s = fold(S0, { seq: 0, d: [{ k: 'net', id: 4, st: 200 }] }) // the patch arrives first
  s = fold(s, { seq: 1, d: [{ k: 'net', id: 4, m: 'POST', url: '/save' }] })
  assert.deepEqual(s.net, [{ k: 'net', id: 4, st: 200, m: 'POST', url: '/save' }])
  s = fold(s, { seq: 2, d: [{ k: 'comp', h: 'a', inst: 1 }, { k: 'comp', h: 'b', inst: 2 }] })
  s = fold(s, { seq: 3, d: [{ k: 'comp', h: 'b', inst: 1 }] }) // `a` unmounted: the snapshot no longer has it
  assert.deepEqual(s.comps.map(c => c.h), ['b'])
  s = fold(s, [{ k: 'leak', code: 'B0601', h: 'z', msg: 'q' }]) // the server's leak list is a plain array (no seq)
  assert.equal(s.seq, 3)
  assert.equal(s.leaks[0].h, 'z')
  assert.equal(fold(s, { seq: 4, d: [{ k: 'unknown' }] }).inst, s.inst) // unknown kinds are ignored
})
