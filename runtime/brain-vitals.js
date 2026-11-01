// Core Web Vitals, opt-in: `vitals(report)` measures in this page and `reportVitals(url)` sends the final values.
// Nothing is emitted unless an app imports this module. Metrics a browser lacks are never reported, so read them as
// optional: LCP and INP do not exist in every engine. Thresholds are the published web.dev good/poor limits.
const LIMITS = { LCP: [2500, 4000], CLS: [0.1, 0.25], INP: [200, 500], FCP: [1800, 3000], TTFB: [800, 1800] }
const rate = (name, value) => (value <= LIMITS[name][0] ? 'good' : value <= LIMITS[name][1] ? 'needs-improvement' : 'poor')

export function vitals(report, env = globalThis) {
  const PO = env.PerformanceObserver
  const types = PO?.supportedEntryTypes ?? []
  const stops = []
  const send = (name, value) => report({ name, value: name === 'CLS' ? +value.toFixed(4) : Math.round(value), rating: rate(name, value) })
  const watch = (type, options, read) => {
    if (!types.includes(type)) return
    const po = new PO(list => read(list.getEntries()))
    po.observe({ type, buffered: true, ...options })
    stops.push(() => po.disconnect())
  }
  const nav = env.performance?.getEntriesByType?.('navigation')?.[0]
  if (nav?.responseStart > 0) send('TTFB', nav.responseStart)
  watch('paint', {}, list => { for (const e of list) if (e.name === 'first-contentful-paint') send('FCP', e.startTime) })
  watch('largest-contentful-paint', {}, list => { const last = list[list.length - 1]; if (last) send('LCP', last.startTime) })
  // CLS: the worst session window (gap under 1 s, at most 5 s long), without shifts caused by recent input.
  let worst = 0, sum = 0, first = 0, previous = 0
  watch('layout-shift', {}, list => {
    for (const e of list) {
      if (e.hadRecentInput) continue
      if (sum && e.startTime - previous < 1000 && e.startTime - first < 5000) sum += e.value
      else { sum = e.value; first = e.startTime }
      previous = e.startTime
      if (sum > worst) { worst = sum; send('CLS', worst) }
    }
  })
  // INP: the slowest interaction, one in 50 discarded as an outlier, as the metric's definition says.
  const slow = new Map()
  let inp = -1
  watch('event', { durationThreshold: 40 }, list => {
    for (const e of list) if (e.interactionId) slow.set(e.interactionId, Math.max(slow.get(e.interactionId) ?? 0, e.duration))
    const all = [...slow.values()].sort((a, b) => b - a)
    const value = all[Math.min(all.length - 1, Math.floor(all.length / 50))]
    if (value !== undefined && value !== inp) send('INP', (inp = value))
    if (slow.size > 500) for (const key of [...slow.keys()].slice(0, 250)) slow.delete(key)
  })
  return () => { for (const stop of stops) stop() }
}

// One beacon each time the page is hidden, with what changed since the last one; the query string is never sent.
export function reportVitals(url, env = globalThis) {
  if (typeof url !== 'string' || !url.startsWith('/')) throw new TypeError('reportVitals(url): pass a same-origin path, such as "/api/vitals"')
  const doc = env.document
  const fresh = new Map()
  const stop = vitals(metric => fresh.set(metric.name, metric), env)
  const flush = () => {
    if (doc.visibilityState !== 'hidden' || !fresh.size) return
    const body = JSON.stringify({ path: env.location.pathname, metrics: [...fresh.values()] })
    if (env.navigator.sendBeacon?.(url, new Blob([body], { type: 'application/json' }))) fresh.clear()
  }
  doc.addEventListener('visibilitychange', flush)
  env.addEventListener('pagehide', flush)
  return () => { stop(); doc.removeEventListener('visibilitychange', flush); env.removeEventListener('pagehide', flush) }
}
