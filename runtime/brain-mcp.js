// BRAINLLM agent tools (WebMCP, INTERNALS §32). A component exposes functions to the browser's AI agent:
//   tool('search_products', 'Search the catalog by text', { q: 'str!', max: 'num<=50' }, ({ q, max }) => …)
// Registered when the component mounts, unregistered when it unmounts; a no-op where the browser has no model context
// (feature-detected: document.modelContext, the newer name, or navigator.modelContext, Chrome's origin trial). The
// parameters use form()'s mini-language and become a JSON Schema; arguments are validated before `run` is called and
// a bad call answers with the field's message instead of running. Tools act with the page's permissions only:
// anything destructive should still ask for confirmation in the UI.
// Optional 5th argument (WebMCP draft 2026-10-27): { title, readOnly, consequential, untrusted } → `title` and
// `annotations: { readOnlyHint, consequentialHint, untrustedContentHint }` on the registration; only the keys given.
// `<meta tools>` (#106) compiles to registerTools([{ name, description, inputSchema, execute }]): the schema is built by
// the compiler from the tool's parameter list, the rest (detection, lifetime, validation, answers) is the same as tool().
import { fx } from './brain.js'
import { rule, check, msg } from './brain-schema.js'

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

// title/annotations onto a registration dictionary, only the keys given.
function extras(def, opts) {
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

// The registration dictionary: name, description, inputSchema, then title/annotations only when given.
export function definition(name, description, params, opts) {
  return extras({ name, description, inputSchema: schema(params) }, opts)
}

// The browser's model context, or null where tools must not be registered (no WebMCP, server render, prerender).
const live = () => {
  const mc = context()
  return mc && mc.registerTool && !G.__brainSSR && !G.__brainRoute ? mc : null
}

// `execute` for a registration: invalid arguments answer with the field's message and never run the function; its
// result (or error) is the tool's text answer.
const answer = (invalid, run) => async (args = {}) => {
  const m = invalid(args || {})
  if (m) return text(m)
  try {
    return text(await run(args || {}))
  } catch (e) {
    return text(`Error: ${(e && e.message) || e}`)
  }
}

// Registers the dictionaries for the lifetime of the calling component (one effect, cleaned up on dispose). A
// registration the browser refuses (a name another mounted instance already holds) is skipped: the page keeps working
// and unmounting this instance leaves the other one's tool in place.
function own(mc, make) {
  fx(() => {
    const defs = make(), offs = []
    for (let i = 0; i < defs.length; i++) {
      const name = defs[i].name
      let reg
      try {
        reg = mc.registerTool(defs[i])
      } catch {
        continue
      }
      offs.push(() => (reg && reg.unregister ? reg.unregister() : mc.unregisterTool && mc.unregisterTool(name)))
    }
    return () => {
      for (let i = 0; i < offs.length; i++) offs[i]()
    }
  })
}

export function tool(name, description, params, run, opts) {
  const mc = live()
  if (!mc) return
  const rules = Object.fromEntries(Object.entries(params || {}).map(([k, s]) => [k, rule(s)]))
  const invalid = args => {
    for (const k in rules) {
      const m = check(rules[k], args[k])
      if (m) return `${k}: ${m}`
    }
    return ''
  }
  own(mc, () => [extras({ name, description, inputSchema: schema(params), execute: answer(invalid, run) }, opts)])
}

// A compiled parameter's JSON Schema → the message for a bad value ('' when it fits). Required means present: `false`
// and `0` are values; a missing or empty one is not.
const FORMAT = { email: 'email', uri: 'url', date: 'date' }, KIND = { number: 'num', boolean: 'bool' }
function argError(p, v, req) {
  if (v == null || v === '') return req ? msg('form.required', 'Required') : ''
  if (p.type === 'array') return Array.isArray(v) ? '' : msg('form.list', 'Must be a list')
  if (p.type === 'object') return typeof v === 'object' && !Array.isArray(v) ? '' : 'Must be an object'
  const t = p.type === 'string' ? FORMAT[p.format] || 'str' : KIND[p.type]
  return t ? check(rule(t), v) : ''
}

// `<meta tools>`: [{ name, description, inputSchema, execute, title?, readOnly?, consequential?, untrusted? }], emitted
// by the compiler with `execute` bound to the component's same-named function.
export function registerTools(list) {
  const mc = live()
  if (!mc) return
  own(mc, () =>
    list.map(t => {
      const s = t.inputSchema || {}, props = s.properties || {}, req = s.required || []
      const invalid = args => {
        for (const k in props) {
          const m = argError(props[k], args[k], req.includes(k))
          if (m) return `${k}: ${m}`
        }
        return ''
      }
      return extras({ name: t.name, description: t.description, inputSchema: s, execute: answer(invalid, t.execute) }, t)
    }),
  )
}
