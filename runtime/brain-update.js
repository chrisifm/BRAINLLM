// BRAINLLM app updates with user control (brain.json `"cache": { "update": "prompt" }`, INTERNALS §35.8).
// `updateReady()` is a signal read: true once a new build's service worker is installed and waiting while this page is
// controlled by the previous one. `applyUpdate()` tells that worker to take over (`brain:skip-waiting`) and reloads
// the page once it controls it. Without a service worker both are inert: false / a resolved promise.
import { sig } from './brain.js'

const G = globalThis
let READY = null, reg = null
function start() {
  READY = sig(false)
  const sw = G.navigator?.serviceWorker
  if (!sw?.getRegistration) return
  const seen = r => {
    if (!r || reg === r) return
    reg = r
    const mark = () => (READY.v = !!(r.waiting && sw.controller))
    mark()
    r.addEventListener?.('updatefound', () => {
      const n = r.installing
      n?.addEventListener?.('statechange', () => n.state === 'installed' && mark())
    })
  }
  sw.getRegistration().then(seen, () => {})
}
export function updateReady() {
  if (!READY) start()
  return READY.v
}
// reloads once, when the new worker controls the page; resolves false when there is nothing to apply
export function applyUpdate() {
  const sw = G.navigator?.serviceWorker
  if (!reg?.waiting || !sw) return Promise.resolve(false)
  return new Promise(ok => {
    sw.addEventListener('controllerchange', () => (ok(true), G.location?.reload()), { once: true })
    reg.waiting.postMessage('brain:skip-waiting')
  })
}
