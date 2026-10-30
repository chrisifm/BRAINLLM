// Dedicated speculative-fetch worker. Only bounded HTML/assets cross the bridge; app scripts never run.
import { prefetchPage } from './brain-prefetch-shared.js'
const jobs = new Map()
self.onmessage = async e => {
  const m = e.data
  if (!m || !Number.isSafeInteger(m.id)) return
  if (m.cancel) { jobs.get(m.id)?.abort(); return }
  if (jobs.size >= 2 || jobs.has(m.id) || typeof m.url !== 'string') return
  let url
  try { url = new URL(m.url); if (url.origin !== self.location.origin) return } catch { return }
  const controller = new AbortController()
  jobs.set(m.id, controller)
  const timer = setTimeout(() => controller.abort(), 6000)
  try {
    const result = await prefetchPage(url.href, controller.signal)
    self.postMessage(result ? { id: m.id, ...result, headers: [...result.headers] } : { id: m.id, ok: false })
  } catch { self.postMessage({ id: m.id, ok: false }) }
  finally { clearTimeout(timer); controller.abort(); jobs.delete(m.id) }
}
