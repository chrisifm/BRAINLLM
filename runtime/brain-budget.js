// Preventive checkpoints for compiler-owned code. Native/foreign calls cannot be preempted here.
import { _rt, fx } from './brain.js'
const I = _rt(), owners = new WeakMap(), active = []
const LIMITS = Object.freeze({ steps: 1000000, depth: 128, ms: 200, rows: 100000, signals: 100000, resources: 1024, pending: 32, workerMs: 30000 })
const now = () => globalThis.performance ? performance.now() : Date.now()
const metaOf = m => ({ file: typeof m?.file === 'string' ? m.file.slice(0, 256) : null, component: typeof m?.component === 'string' ? m.component.slice(0, 80) : null, line: Number.isSafeInteger(m?.line) && m.line > 0 ? m.line : null, column: Number.isSafeInteger(m?.column) && m.column > 0 ? m.column : null })
const limitsOf = options => Object.fromEntries(Object.entries(LIMITS).map(([k, max]) => [k, Number.isFinite(options?.[k]) && options[k] >= 1 ? Math.min(max, options[k]) : max]))
const failure = (code, kind, limit, meta) => Object.assign(new Error(`${code}: ${kind} budget exceeded`), { code, kind: 'budget', budget: kind, limit, ...metaOf(meta), fix: kind === 'ms' || kind === 'steps' || kind === 'depth' ? 'Bound the work or move it to an owned worker.' : 'Bound the collection and dispose resources with their owner.' })
function charge(root) {
  if (root.running) { const t = now(); root.work += Math.max(0, t - root.started); root.started = t }
}
function enforce(frame, kind, value) {
  const r = frame.root
  if (r.error) throw r.error
  if (value > r.limits[kind]) throw (r.error = failure('B0910', kind, r.limits[kind], frame.meta))
}
export function budgetOwner() { return I.owner() }
export function budgetEnter(meta, owner = I.owner(), options) {
  const parent = active[active.length - 1]
  const root = parent?.root || { limits: limitsOf(options), steps: 0, work: 0, depth: 0, running: false, started: 0, error: null }
  const f = { root, meta: metaOf(meta), owner, state: 'active' }
  charge(root)
  enforce(f, 'depth', root.depth + 1)
  root.depth++
  if (!root.running) { root.running = true; root.started = now() }
  active.push(f)
  return f
}
export function budgetStep(frame, site) {
  if (!frame || frame.state !== 'active') throw failure('B0910', 'inactive-frame', 0, site)
  if (site) frame.meta = metaOf(site)
  const r = frame.root
  // A synchronous throw inside a paused await expression must not leave the clock stopped.
  if (frame.paused) { frame.paused = 0; r.running = true; r.started = now() }
  charge(r)
  enforce(frame, 'steps', ++r.steps)
  enforce(frame, 'ms', r.work)
}
function detach(f) {
  if (f.state !== 'active') return
  const index = active.lastIndexOf(f)
  if (index >= 0) active.splice(index, 1)
  const r = f.root
  charge(r)
  r.depth--
  // Native code continuing a paused await expression is not charged.
  const top = active.at(-1)
  if (top?.root !== r || top.paused) r.running = false
}
export function budgetLeave(frame) {
  if (!frame || frame.state === 'closed') return
  detach(frame)
  frame.state = 'closed'
}
// Generated as `budgetSuspend(frame)(expr)`: the clock stops before `expr` runs, so native work
// (a fetch's connection setup) is not charged; author functions called inside it restart the clock.
// The outcome always resolves: the generated resume runs even on a rejected await, preserving catch/finally.
export function budgetSuspend(frame) {
  if (frame?.state === 'active' && !frame.paused) { charge(frame.root); frame.root.running = false; frame.paused = 1 }
  return promise => {
    frame.paused = 0
    detach(frame)
    frame.state = 'suspended'
    return Promise.resolve(promise).then(value => ({ ok: true, value }), error => ({ ok: false, error }))
  }
}
export function budgetResume(frame, outcome) {
  if (frame.state !== 'suspended') throw failure('B0910', 'inactive-frame', 0, frame.meta)
  const r = frame.root
  if (!r.running) { r.running = true; r.started = now() }
  r.depth++
  frame.state = 'active'
  active.push(frame)
  // Preserve the original rejection; a later checkpoint still sees a poisoned root.
  if (!outcome.ok) throw outcome.error
  budgetStep(frame)
  return outcome.value
}
function ledger(owner, meta, options) {
  if (!owner || typeof owner !== 'object') throw failure('B0911', 'missing-owner', 0, meta)
  if (owner.s === 3) throw failure('B0911', 'disposed-owner', 0, meta)
  let l = owners.get(owner)
  if (!l) {
    l = { limits: limitsOf(options), counts: Object.create(null), releases: new Set(), disposed: false }
    owners.set(owner, l)
    I.root(() => fx(() => () => {
      l.disposed = true
      owners.delete(owner)
      for (const release of [...l.releases]) release()
    }), owner)
  }
  if (l.disposed) throw failure('B0911', 'disposed-owner', 0, meta)
  return l
}
// Core hook before owned sig construction / list materialization. Arrays may already exist at this point.
export function budgetAllocation(kind, owner, units) {
  if (!owner) return // Module-level shared state has no disposable component owner.
  if (!['rows', 'signals'].includes(kind) || !Number.isSafeInteger(units) || units < 0) throw failure('B0911', 'invalid-allocation', 0, null)
  const frame = active[active.length - 1]
  const meta = frame?.meta
  const l = ledger(owner, meta)
  const next = kind === 'signals' ? (l.counts.signals || 0) + units : units
  if (next > l.limits[kind]) throw failure('B0911', kind, l.limits[kind], meta)
  l.counts[kind] = next
}
export function budgetClaim(owner, kind, units = 1, dispose, meta, options) {
  const l = ledger(owner, meta, options)
  if (!['rows', 'signals', 'resources', 'pending'].includes(kind) || !Number.isSafeInteger(units) || units < 0) throw failure('B0911', 'invalid-claim', 0, meta)
  const count = l.counts[kind] || 0
  if (count + units > l.limits[kind]) throw failure('B0911', kind, l.limits[kind], meta)
  // Bound disposal bookkeeping independently of units (including zero-unit claims).
  if (l.releases.size >= LIMITS.resources) throw failure('B0911', 'resources', LIMITS.resources, meta)
  l.counts[kind] = count + units
  let live = true
  const release = () => {
    if (!live) return
    live = false
    l.releases.delete(release)
    l.counts[kind] -= units
    if (dispose) {
      try { dispose() } catch { I.oops(failure('B0911', 'cleanup', 0, meta), owner, 'budget') }
    }
  }
  l.releases.add(release)
  return release
}
// Caller must supply a Worker it created exclusively, never an unrelated shared worker.
export function budgetWatchWorker(ownedWorker, rejectAll, meta, options) {
  const limits = limitsOf(options), tasks = new Map()
  let closed = false
  const stop = (error = failure('B0911', 'worker-disposed', 0, meta)) => {
    if (closed) return
    closed = true
    for (const timer of tasks.values()) clearTimeout(timer)
    tasks.clear()
    ownedWorker.terminate()
    rejectAll(error)
  }
  return {
    track(id) {
      if (closed) throw failure('B0911', 'worker-disposed', 0, meta)
      if (tasks.has(id)) throw failure('B0911', 'duplicate-worker-task', 0, meta)
      if (tasks.size >= limits.pending) throw failure('B0911', 'pending', limits.pending, meta)
      tasks.set(id, setTimeout(() => stop(failure('B0910', 'workerMs', limits.workerMs, meta)), limits.workerMs))
      let live = true
      return () => { if (live) { live = false; clearTimeout(tasks.get(id)); tasks.delete(id) } }
    },
    dispose: stop,
  }
}
// Optional small core hook; old runtimes remain loadable while the compiler/runtime version gate handles upgrades.
I.budget?.(budgetAllocation)
