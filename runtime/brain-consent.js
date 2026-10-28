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
const read = () => {
  try {
    return JSON.parse(G.localStorage.getItem(KEY)) || {}
  } catch {
    return {}
  }
}
const apply = state => {
  const u = {}
  for (const k in state) for (const t of TYPES[k] || []) u[t] = state[k] ? 'granted' : 'denied'
  if (!Object.keys(u).length) return
  if (typeof G.gtag === 'function') G.gtag('consent', 'update', u)
  else if (G.dataLayer) G.dataLayer.push(['consent', 'update', u]) // gtag not loaded yet: queued the way gtag reads it
}

export function consent(kind, granted = true) {
  if (!TYPES[kind]) throw Error(`consent: unknown kind "${kind}" (${Object.keys(TYPES).join(', ')})`)
  const s = { ...read(), [kind]: !!granted }
  try {
    G.localStorage.setItem(KEY, JSON.stringify(s))
  } catch {}
  apply({ [kind]: !!granted })
}
export const consented = kind => !!read()[kind]

apply(read()) // a returning visitor: their earlier answer, before any tag reads the defaults
