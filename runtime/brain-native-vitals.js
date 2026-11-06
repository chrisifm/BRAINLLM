// Optional initial-document performance estimates. Only declared patterns leave the browser.
import { consented } from './brain-consent.js'
import { vitals } from './brain-vitals.js'
const STATE = Symbol.for('brain:native-vitals')
const ROUTE = /^(?:\/|(?:\/(?:[A-Za-z0-9_-]+|:[a-z][a-z0-9_]*))+)$/
const MAX = { LCP: 3600000, CLS: 1000, INP: 3600000, FCP: 3600000, TTFB: 3600000 }
export function startNativeVitals(config, env = globalThis) {
  if (env.__brainSSR || env.__brainRoute || !env.window || !env.document || !env.location || env[STATE]) return env[STATE]
  if (!config || typeof config.base !== 'string' || !/^(?:\/[A-Za-z0-9_.-]+)*$/.test(config.base) || config.base.split('/').some(p => p === '.' || p === '..') || !Array.isArray(config.routes) || !config.routes.length || config.routes.length > 512 || config.routes.some(p => typeof p !== 'string' || p.length > 160 || !ROUTE.test(p)) || new Set(config.routes).size !== config.routes.length) return
  let origin
  try { const url = new URL(env.location.href); if (!/^https?:$/.test(url.protocol)) return; origin = url.origin } catch { return }
  const initialPath = env.location.pathname
  if (config.base && !initialPath.startsWith(config.base + '/')) return
  const local = config.base ? initialPath.slice(config.base.length) : initialPath
  const pathname = local.length > 1 && local.endsWith('/') ? local.slice(0, -1) : local
  const parts = pathname.split('/')
  let route, score = -1
  for (const pattern of config.routes) {
    const wanted = pattern.split('/')
    if (wanted.length !== parts.length) continue
    let matches = true, literal = 0
    for (let i = 0; i < wanted.length; i++) {
      if (wanted[i].startsWith(':')) { if (!parts[i]) { matches = false; break } }
      else if (wanted[i] !== parts[i]) { matches = false; break }
      else literal++
    }
    if (matches && literal > score) { route = pattern; score = literal }
  }
  if (!route) return
  const target = env.window, doc = env.document, fresh = new Map()
  let active = false, disposed = false, navigated = false, off, pending, timer, generation = 0
  const dnt = value => value === '1' || value === 'yes' || value === true
  const permitted = () => consented('analytics', env) && !dnt(env.navigator?.doNotTrack) && !dnt(env.navigator?.msDoNotTrack) && !dnt(env.doNotTrack) && env.navigator?.globalPrivacyControl !== true
  const deactivate = () => {
    active = false; generation++; off?.(); off = undefined; fresh.clear()
    doc.removeEventListener('visibilitychange', flush); target.removeEventListener('pagehide', flush)
    pending?.abort(); pending = undefined; env.clearTimeout?.(timer); timer = undefined
  }
  const navigation = () => { navigated = true; deactivate() }
  const flush = event => {
    if (!active || !permitted()) return deactivate()
    if (env.location.pathname !== initialPath) return navigation()
    if (event?.type !== 'pagehide' && doc.visibilityState !== 'hidden' || !fresh.size || pending) return
    const body = JSON.stringify({ route, consent: true, metrics: [...fresh.values()] })
    fresh.clear()
    if (body.length > 4096) return
    const controller = new env.AbortController(), epoch = generation
    pending = controller
    timer = env.setTimeout(() => controller.abort(), 3000)
    Promise.resolve().then(() => active && epoch === generation && permitted() ? env.fetch(origin + config.base + '/_brain/vitals', {
      method: 'POST', headers: { 'content-type': 'application/json' }, body,
      mode: 'same-origin', credentials: 'omit', referrerPolicy: 'no-referrer', redirect: 'error', cache: 'no-store', keepalive: true, signal: controller.signal,
    }) : undefined).then(response => { if (response?.status === 404) state.stop() }).catch(() => {}).finally(() => {
      if (pending === controller) { pending = undefined; env.clearTimeout(timer); timer = undefined }
    })
  }
  const update = () => {
    if (disposed) return
    if (!permitted()) return deactivate()
    if (active || navigated || env.location.pathname !== initialPath || typeof env.fetch !== 'function' || !env.AbortController) return
    active = true
    const epoch = generation
    off = vitals(metric => {
      if (!active || epoch !== generation || !permitted()) return
      if (Object.hasOwn(MAX, metric.name) && Number.isFinite(metric.value) && metric.value >= 0 && metric.value <= MAX[metric.name]) fresh.set(metric.name, { name: metric.name, value: metric.value })
    }, env)
    doc.addEventListener('visibilitychange', flush); target.addEventListener('pagehide', flush)
  }
  const storage = event => { if (event.key === 'brain:consent' || event.key === null) { delete env[Symbol.for('brain:consent-state')]; update() } }
  const state = { stop() { disposed = true; deactivate(); target.removeEventListener('brain:navigate', navigation); target.removeEventListener('popstate', navigation); target.removeEventListener('brain:consent', update); target.removeEventListener('storage', storage); delete env[STATE] } }
  env[STATE] = state
  target.addEventListener('brain:consent', update); target.addEventListener('storage', storage)
  // Lifetime events only: no performance access or collection occurs before consent. Revocation must not
  // erase knowledge that the original document has navigated away and returned to the same pathname.
  target.addEventListener('brain:navigate', navigation); target.addEventListener('popstate', navigation)
  update()
  return state
}
