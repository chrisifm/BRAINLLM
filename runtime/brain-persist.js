// BRAINLLM store persistence (INTERNALS §35.4), loaded on demand by `store(init, { persist: true })` (brain-ch.js).
// brain-ch restores small states from localStorage synchronously; this module restores large ones from IndexedDB and
// writes small changes immediately; larger changes debounce 100 ms into IndexedDB, or flush when their owner leaves.
// Records carry the shape version `v`; another shape is discarded. Quota or private-mode errors switch that store to
// memory and are reported once (reportError → brain-err), never thrown.
import { _rt, fx } from './brain.js'

const G = globalThis, I = _rt(), LIMIT = 4096, WAIT = 100
let warned = false
const report = e => {
  if (warned) return
  warned = true
  const x = Object.assign(new Error('persist: ' + ((e && e.message) || e) + '; the store stays in memory'), { code: 'persist', cause: e })
  G.reportError ? G.reportError(x) : console.warn(x.message)
}

let dbp = null
const req = r => new Promise((ok, fail) => ((r.onsuccess = () => ok(r.result)), (r.onerror = () => fail(r.error))))
const db = () =>
  (dbp ||= new Promise((ok, fail) => {
    const r = G.indexedDB.open('brain', 1)
    r.onupgradeneeded = () => r.result.createObjectStore('kv')
    r.onsuccess = () => ok(r.result)
    r.onerror = () => fail(r.error)
  }).catch(e => ((dbp = null), Promise.reject(e))))
const idb = async (mode, f) => req(f((await db()).transaction('kv', mode).objectStore('kv')))

export async function persist({ key, v, big, apply, snap, S, base, owner }) {
  let mem = false, last = base, revision = 0, t = null, inIdb = big, stopped = false
  const write = async j => {
    if (mem) return
    const current = ++revision
    try {
      if (j.length < LIMIT || !G.indexedDB) {
        G.localStorage.setItem(key, `{"v":${JSON.stringify(v)},"d":${j}}`)
        if (inIdb) (inIdb = false), await idb('readwrite', s => s.delete(key))
      } else {
        await idb('readwrite', s => s.put({ v, d: JSON.parse(j) }, key))
        if (current !== revision) return
        inIdb = true
        G.localStorage.setItem(key, JSON.stringify({ v, idb: 1 }))
      }
    } catch (e) {
      if (current === revision) {
        mem = true
        report(e)
      }
    }
  }
  const finalSnapshot = async () => {
    const j = JSON.stringify(snap())
    if (j !== base) await write(j)
  }
  // Disposal can happen while importing the module or restoring IndexedDB. Preserve the edit, but attach nothing.
  if (owner?.s === 3) { await finalSnapshot(); return }
  if (big)
    try {
      const r = G.indexedDB ? await idb('readonly', s => s.get(key)) : null
      if (owner?.s === 3) { await finalSnapshot(); return }
      if (r && r.v === v) apply(r.d), (last = JSON.stringify(snap()))
    } catch (e) { report(e) }
  if (owner?.s === 3) { await finalSnapshot(); return }
  // A page can leave before the effect queue flushes. Only small snapshots have a synchronous durability path;
  // pending IndexedDB work remains asynchronous and is never promised to finish after the document closes.
  const flush = large => {
    if (stopped || mem) return
    try {
      const j = JSON.stringify(snap())
      if (large || j.length < LIMIT || !G.indexedDB) {
        clearTimeout(t)
        t = null
        write(j)
      }
    } catch (e) { mem = true; report(e) }
  }
  const flushSmall = () => flush(false)
  const hidden = () => { if (G.document?.visibilityState === 'hidden') flushSmall() }
  I.root(() => {
    fx(() => {
      const d = {}
      for (const k in S) d[k] = S[k].v // tracks every field
      const j = JSON.stringify(d)
      if (j === last) return
      last = j
      clearTimeout(t)
      t = null
      if (j.length < LIMIT || !G.indexedDB) write(j)
      else t = setTimeout(() => { t = null; write(last) }, WAIT)
    })
    G.addEventListener?.('pagehide', flushSmall)
    G.document?.addEventListener?.('visibilitychange', hidden)
    fx(() => () => {
      flush(true)
      stopped = true
      clearTimeout(t)
      G.removeEventListener?.('pagehide', flushSmall)
      G.document?.removeEventListener?.('visibilitychange', hidden)
    })
  }, owner)
}
