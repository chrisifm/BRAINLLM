// BRAINLLM consent (INTERNALS §32, Google Consent Mode v2). The build injects Analytics/Tag Manager with every consent
// type denied by default; the app grants what the visitor accepted:
//   consent('analytics') · consent('ads') · consent('analytics', false) (revoke) · consented('analytics') → true/false
// The answer is remembered in localStorage and applied again on the next visit as soon as this module loads.
const G = globalThis
const KEY = 'brain:consent'
const TYPES = {
  analytics: ['analytics_storage'],
  ads: ['ad_storage', 'ad_user_data', 'ad_personalization'],
  functional: ['functionality_storage'],
  personal: ['personalization_storage'],
}
const MEMORY = Symbol.for('brain:consent-state')
const read = (env = G) => {
  const state = {}
  try {
    const stored = JSON.parse(env.localStorage.getItem(KEY))
    for (const kind of Object.keys(TYPES)) if (typeof stored?.[kind] === 'boolean') state[kind] = stored[kind]
  } catch {}
  const memory = env[MEMORY]
  for (const kind of Object.keys(TYPES)) if (typeof memory?.[kind] === 'boolean') state[kind] = memory[kind]
  return state
}
const apply = (state, env = G) => {
  const u = {}
  for (const k in state) for (const t of TYPES[k] || []) if (typeof state[k] === 'boolean') u[t] = state[k] ? 'granted' : 'denied'
  if (!Object.keys(u).length) return
  try {
    if (typeof env.gtag === 'function') env.gtag('consent', 'update', u)
    else if (Array.isArray(env.dataLayer)) env.dataLayer.push((function () { return arguments })('consent', 'update', u))
  } catch {} // a blocked or broken external tag must not prevent a local privacy decision
}
const notify = env => {
  const target = env.window || env
  const EventType = env.Event || G.Event
  if (target.dispatchEvent && EventType) target.dispatchEvent(new EventType('brain:consent'))
}
export function consent(kind, granted = true, env = G) {
  if (!TYPES[kind]) throw Error(`consent: unknown kind "${kind}" (${Object.keys(TYPES).join(', ')})`)
  if (typeof granted !== 'boolean') throw Error('consent: granted must be true or false')
  if (env.__brainSSR || env.__brainRoute) return
  const s = { ...read(env), [kind]: granted }
  env[MEMORY] = s // a current-page decision still works when browser storage is blocked
  try { env.localStorage.setItem(KEY, JSON.stringify(s)) } catch {}
  // Disable the bridge/Google tag before a denied update can be processed by an already-loaded tag.
  if (!granted) notify(env)
  apply({ [kind]: granted }, env)
  if (granted) notify(env)
}
export const consented = (kind, env = G) => !env.__brainSSR && !env.__brainRoute && read(env)[kind] === true

// Opt-in, non-modal controls: native buttons work with keyboard and screen readers; no custom key trap.
// App policy wording and any additional jurisdiction-specific requirements remain the site's responsibility.
export function consentBanner(options = {}, env = G) {
  const doc = env.document
  if (env.__brainSSR || env.__brainRoute || !doc?.body || options.enabled === false) return null
  const region = doc.createElement('section'), choices = doc.createElement('div')
  region.setAttribute('role', 'region'); region.setAttribute('aria-label', 'Privacy preferences')
  region.setAttribute('data-brain-consent', '')
  const message = doc.createElement('p')
  message.textContent = 'Optional Google analytics stays off until you accept. You can change your choice here.'
  choices.appendChild(message)
  const button = (label, fn, parent) => {
    const el = doc.createElement('button'); el.setAttribute('type', 'button'); el.textContent = label
    el.addEventListener('click', fn); parent.appendChild(el); return el
  }
  const refresh = () => { choices.hidden = Object.hasOwn(read(env), 'analytics'); revoke.hidden = !consented('analytics', env) }
  const choose = value => { consent('analytics', value, env); refresh(); manage.focus?.() }
  button('Deny analytics', () => choose(false), choices)
  button('Accept analytics', () => choose(true), choices)
  region.appendChild(choices)
  const manage = button('Manage privacy preferences', () => { choices.hidden = false; choices.querySelector('button')?.focus?.() }, region)
  const revoke = button('Revoke analytics consent', () => choose(false), region)
  const target = env.window || env
  const storageChanged = event => { if (event.key === KEY || event.key === null) { delete env[MEMORY]; notify(env); refresh() } }
  target.addEventListener?.('brain:consent', refresh)
  target.addEventListener?.('storage', storageChanged)
  refresh(); doc.body.appendChild(region)
  return { element: region, stop() { target.removeEventListener?.('brain:consent', refresh); target.removeEventListener?.('storage', storageChanged); region.parentNode?.removeChild(region) } }
}

apply(read()) // a returning visitor: their earlier answer, before any tag reads the defaults

// The public consent API also invalidates page-local decisions across tabs when no collector or banner is mounted.
// One page-lifetime listener is shared by the generated bridge and separately imported consent module.
const STORAGE_SYNC = Symbol.for('brain:consent-storage-sync')
const consentWindow = G.window || G
if (!G.__brainSSR && !G.__brainRoute && consentWindow.addEventListener && !G[STORAGE_SYNC]) {
  G[STORAGE_SYNC] = true
  consentWindow.addEventListener('storage', event => {
    if (event.key !== KEY && event.key !== null) return
    delete G[MEMORY]
    notify(G)
    const state = read()
    const deniedMissing = {}
    for (const kind of Object.keys(TYPES)) deniedMissing[kind] = state[kind] === true
    apply(deniedMissing)
  })
}
