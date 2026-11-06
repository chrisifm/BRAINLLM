// Optional error reporting. Consent owns the entire capture lifetime; raw values never leave this function.
import { _rt } from './brain.js'
import { consented } from './brain-consent.js'

const TYPES = new Set(['Error', 'TypeError', 'RangeError', 'ReferenceError', 'SyntaxError', 'URIError', 'EvalError', 'AggregateError'])
const UUID = /^[a-f0-9]{8}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{12}$/
const STATE = Symbol.for('brain:sentry')
export function startSentry(config, env = globalThis, subscribe) {
  if (env.__brainSSR || env.__brainRoute || !env.window || !env.location || env[STATE]) return env[STATE]
  if (!config || !config.assets || typeof config.base !== 'string' || !/^(?:\/[a-zA-Z0-9_.-]+)*$/.test(config.base) || config.base.split('/').some(p => p === '.' || p === '..')) return
  const assets = new Map()
  for (const [path, id] of Object.entries(config.assets)) if (path.length <= 511 && /^(?:[a-zA-Z0-9_.-]+\/)*[a-z0-9]{8}\.(?:js|jsbrain)$/.test(path) && UUID.test(id)) assets.set('/' + path, id)
  const target = env.window, pending = new Set(), seen = new Set()
  let active = false, disposed = false, off, windowStart = 0, count = 0, generation = 0
  const dnt = value => value === '1' || value === 'yes' || value === true
  const permitted = () => consented('analytics', env) && !dnt(env.navigator?.doNotTrack) && !dnt(env.navigator?.msDoNotTrack) && !dnt(env.doNotTrack) && env.navigator?.globalPrivacyControl !== true
  const capture = error => {
    try {
      if (!active || !permitted() || pending.size >= 4 || !error) return
      const stack = error.stack
      if (typeof stack !== 'string') return
      const frames = []
      for (const line of stack.slice(0, 8192).split('\n').slice(0, 41)) {
        const m = /^\s+at (?:[^\r\n()]{1,160} \()?(https?:\/\/[^\s()]+):(\d{1,7}):(\d{1,7})\)?$/.exec(line) || /^[^\s@()]{0,160}@(https?:\/\/[^\s()]+):(\d{1,7}):(\d{1,7})$/.exec(line)
        if (!m) continue
        const url = new URL(m[1]), id = assets.get(url.pathname)
        if (url.origin !== env.location.origin || url.search || url.hash || !id || +m[2] < 1 || +m[3] < 1) continue
        frames.push({ asset: url.pathname, line: +m[2], column: +m[3], debugId: id })
        if (frames.length >= 20) break
      }
      if (!frames.length) return
      const name = error.name
      const type = TYPES.has(name) ? name : 'Error'
      const payload = JSON.stringify({ schemaVersion: 1, type, frames })
      const now = Date.now()
      if (now - windowStart >= 60000) { windowStart = now; count = 0; seen.clear() }
      if (count >= 8 || seen.has(payload) || seen.size >= 32) return
      count++; seen.add(payload)
      const controller = new env.AbortController()
      const epoch = generation
      pending.add(controller)
      const timer = env.setTimeout(() => controller.abort(), 3000)
      Promise.resolve().then(() => active && epoch === generation && permitted() ? env.fetch(env.location.origin + config.base + '/_brain/sentry', {
        method: 'POST', headers: { 'content-type': 'application/json' }, body: payload,
        mode: 'same-origin', credentials: 'omit', referrerPolicy: 'no-referrer', redirect: 'error', cache: 'no-store', signal: controller.signal,
      }) : undefined).catch(() => {}).finally(() => { env.clearTimeout(timer); pending.delete(controller) })
    } catch {} // reporting, including hostile accessors, never enters the app's error routing
  }
  const onError = event => capture(event.error)
  const onRejection = event => capture(event.reason)
  const deactivate = () => {
    active = false; generation++; off?.(); off = undefined
    target.removeEventListener('error', onError); target.removeEventListener('unhandledrejection', onRejection)
    for (const request of pending) request.abort()
    pending.clear(); seen.clear(); count = 0; windowStart = 0
  }
  const update = () => {
    if (disposed) return
    if (!permitted()) return deactivate()
    if (active || typeof env.fetch !== 'function' || !env.AbortController) return
    active = true
    off = (subscribe || _rt().listenErrors)(capture)
    target.addEventListener('error', onError); target.addEventListener('unhandledrejection', onRejection)
  }
  const storage = event => { if (event.key === 'brain:consent' || event.key === null) update() }
  target.addEventListener('brain:consent', update); target.addEventListener('storage', storage)
  const state = { stop() { disposed = true; deactivate(); target.removeEventListener('brain:consent', update); target.removeEventListener('storage', storage); delete env[STATE] } }
  env[STATE] = state
  update()
  return state
}
