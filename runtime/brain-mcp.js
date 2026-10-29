// BRAINLLM agent tools (WebMCP, INTERNALS §32). A component exposes functions to the browser's AI agent:
//   tool('search_products', 'Search the catalog by text', { q: 'str!', max: 'num<=50' }, ({ q, max }) => …)
// Registered when the component mounts, unregistered when it unmounts; a no-op where the browser has no model context
// (feature-detected: document.modelContext, the newer name, or navigator.modelContext, Chrome's origin trial). The
// parameters use form()'s mini-language and become a JSON Schema; arguments are validated before `run` is called and
// a bad call answers with the field's message instead of running. Tools act with the page's permissions only:
// anything destructive should still ask for confirmation in the UI.
// Optional 5th argument (WebMCP draft 2026-10-27): { title, readOnly, consequential, untrusted } → `title` and
// `annotations: { readOnlyHint, consequentialHint, untrustedContentHint }` on the registration; only the keys given.
import { fx } from './brain.js'
import { rule, check } from './brain-schema.js'

const G = globalThis
export const context = () => (G.document && G.document.modelContext) || (G.navigator && G.navigator.modelContext) || null

const TYPE = { str: 'string', email: 'string', url: 'string', date: 'string', num: 'number', bool: 'boolean' }
// { q: 'str!', tags: '[str]' } → JSON Schema (object, properties, required)
export function schema(params = {}) {
  const properties = {}, required = []
  for (const [k, src] of Object.entries(params)) {
    const r = rule(src), item = { type: TYPE[r.t] || 'string' }
    if (r.t === 'email') item.format = 'email'
    if (r.t === 'url') item.format = 'uri'
    if (r.t === 'date') item.format = 'date'
    if (r.t === 'num' && r.min != null) item.minimum = r.min
    if (r.t === 'num' && r.max != null) item.maximum = r.max
    if (r.t === 'str' && r.min != null) item.minLength = r.min
    if (r.t === 'str' && r.max != null) item.maxLength = r.max
    properties[k] = r.list ? { type: 'array', items: item } : item
    if (r.req) required.push(k)
  }
  return { type: 'object', properties, required }
}

const text = v => ({ content: [{ type: 'text', text: typeof v === 'string' ? v : JSON.stringify(v ?? null) }] })

const HINTS = [['readOnly', 'readOnlyHint'], ['consequential', 'consequentialHint'], ['untrusted', 'untrustedContentHint']]

// The registration dictionary: name, description, inputSchema, then title/annotations only when given.
export function definition(name, description, params, opts) {
  const def = { name, description, inputSchema: schema(params) }
  if (!opts) return def
  if (typeof opts.title === 'string' && opts.title) def.title = opts.title
  let annotations = null
  for (let i = 0; i < HINTS.length; i++) {
    const v = opts[HINTS[i][0]]
    if (v != null) (annotations || (annotations = {}))[HINTS[i][1]] = !!v
  }
  if (annotations) def.annotations = annotations
  return def
}

export function tool(name, description, params, run, opts) {
  const mc = context()
  if (!mc || !mc.registerTool || G.__brainSSR || G.__brainRoute) return
  const rules = Object.fromEntries(Object.entries(params || {}).map(([k, s]) => [k, rule(s)]))
  fx(() => {
    const def = definition(name, description, params, opts)
    def.execute = async (args = {}) => {
      for (const k in rules) {
        const m = check(rules[k], args[k])
        if (m) return text(`${k}: ${m}`)
      }
      try {
        return text(await run(args))
      } catch (e) {
        return text(`Error: ${(e && e.message) || e}`)
      }
    }
    const reg = mc.registerTool(def)
    return () => (reg && reg.unregister ? reg.unregister() : mc.unregisterTool && mc.unregisterTool(name))
  })
}
