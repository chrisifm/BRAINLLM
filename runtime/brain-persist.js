// BRAINLLM store persistence (INTERNALS §35.4), loaded on demand by `store(init, { persist: true })` (brain-ch.js).
// brain-ch restores small states from localStorage synchronously; this module restores large ones from IndexedDB and
// writes every change (debounced 100 ms): below 4 KB to localStorage, above to IndexedDB with a marker in localStorage.
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

export async function persist({ key, v, big, apply, snap, S, base }) {
  let mem = false, last = base
  if (big)
    try {
      const r = G.indexedDB ? await idb('readonly', s => s.get(key)) : null
      if (r && r.v === v) apply(r.d), (last = JSON.stringify(snap()))
    } catch (e) {
      report(e)
    }
  let t = null, inIdb = big
  const write = async j => {
    if (mem) return
    try {
      if (j.length < LIMIT || !G.indexedDB) {
        G.localStorage.setItem(key, `{"v":${JSON.stringify(v)},"d":${j}}`)
        if (inIdb) (inIdb = false), await idb('readwrite', s => s.delete(key))
      } else {
        await idb('readwrite', s => s.put({ v, d: JSON.parse(j) }, key))
        inIdb = true
        G.localStorage.setItem(key, JSON.stringify({ v, idb: 1 }))
      }
    } catch (e) {
      mem = true
      report(e)
    }
  }
  I.root(() =>
    fx(() => {
      const d = {}
      for (const k in S) d[k] = S[k].v // tracks every field
      const j = JSON.stringify(d)
      if (j === last) return
      last = j
      clearTimeout(t)
      t = setTimeout(() => write(last), WAIT)
    })
  )
}
