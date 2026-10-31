// Built-in AI (Chrome's on-device Gemini Nano, INTERNALS §19, docs checked 2026-10-29): `ai` is a primitive, so a view
// uses it without an import. Every call feature-detects the browser API; on the server or in a browser without it,
// `ai.ready()` answers 'unavailable' and the other calls throw an Error with `code: 'unavailable'`, so the app decides
// what to show instead. What the model does is up to the app: the runtime only opens the door.
//   await ai.ready()                       'available' | 'downloadable' | 'downloading' | 'unavailable' (Prompt API)
//   await ai.ready('summarizer')           same for summarizer, detector, writer, rewriter, proofreader
//   await ai.ready('translator', { from: 'en', to: 'es' })   the translator needs its two languages
//   await ai.prompt('Name three colors', { system, schema, signal, onProgress })   → string (stateless call)
//   for await (const part of ai.stream('Tell a story')) …                          → text chunks as they arrive
//   const chat = await ai.session({ system })   native session: chat.prompt(), append(), clone(), destroy()
//   await ai.summarize(text, { type, format, length, context })                     → string
//   await ai.translate(text, { from: 'en', to: 'es' })                              → string
//   await ai.detect(text)                                                           → [{ detectedLanguage, confidence }]
//   await ai.write(task) · ai.rewrite(text, { tone }) · ai.proofread(text)
// Status (Chrome 138+ stable): Summarizer, Translator and LanguageDetector on the web; the Prompt API is an origin
// trial on the web (Chrome 148) and stable in extensions; Writer, Rewriter and Proofreader are trials. The Prompt API
// runs in the top window and same-origin iframes (`allow="language-model"` for others), not in Web Workers. A model
// that is 'downloadable' starts downloading on the first create, which Chrome only allows after a user gesture: call
// these from an event handler the first time; `onProgress(0..1)` follows the download.
const G = globalThis
const API = {
  prompt: 'LanguageModel',
  summarizer: 'Summarizer',
  translator: 'Translator',
  detector: 'LanguageDetector',
  writer: 'Writer',
  rewriter: 'Rewriter',
  proofreader: 'Proofreader',
}
const native = kind => {
  const name = API[kind]
  if (!name) throw Error(`ai: unknown model \`${kind}\`; use one of ${Object.keys(API).join(', ')}`)
  return G[name] || null
}
const missing = kind => Object.assign(Error(`Built-in AI is not available in this browser (${API[kind]})`), { code: 'unavailable' })

// `{ from, to }` are the translator's languages, as in ai.translate (Translator.availability needs them)
const langs = ({ from, to, ...o } = {}) => ({ ...o, ...(from ? { sourceLanguage: from } : {}), ...(to ? { targetLanguage: to } : {}) })
async function ready(kind = 'prompt', options) {
  const N = native(kind)
  if (!N || !N.availability) return 'unavailable'
  try {
    return await N.availability(langs(options))
  } catch {
    return 'unavailable'
  }
}

// create(options) with the download progress wired to onProgress (e.loaded is 0..1)
async function create(kind, o = {}) {
  const N = native(kind)
  if (!N || !N.create) throw missing(kind)
  const { onProgress, ...rest } = o
  if (onProgress) rest.monitor = m => m.addEventListener('downloadprogress', e => onProgress(e.loaded))
  return N.create(rest)
}

// one base session per system prompt; each stateless call works on a clone, so calls never share history. A system
// prompt built from data (a name, a date) would open a native session per value: keep the 8 most recent, destroy the rest.
const bases = new Map(), BASES = 8
async function base(o) {
  const k = o.system || ''
  let p = bases.get(k)
  if (p) bases.delete(k)
  else {
    p = create('prompt', { ...(o.system ? { initialPrompts: [{ role: 'system', content: o.system }] } : {}), ...(o.onProgress ? { onProgress: o.onProgress } : {}) })
    p.catch(() => bases.get(k) === p && bases.delete(k))
    if (bases.size >= BASES) {
      const [old, q] = bases.entries().next().value
      bases.delete(old)
      q.then(s => s.destroy && s.destroy(), () => {})
    }
  }
  bases.set(k, p)
  return (await p).clone()
}
const opts = o => ({ ...(o.signal ? { signal: o.signal } : {}), ...(o.schema ? { responseConstraint: o.schema } : {}) })

async function prompt(text, o = {}) {
  const s = await base(o)
  try {
    const out = await s.prompt(text, opts(o))
    return o.schema && typeof out === 'string' ? JSON.parse(out) : out
  } finally {
    s.destroy && s.destroy()
  }
}

async function* stream(text, o = {}) {
  const s = await base(o)
  try {
    for await (const part of s.promptStreaming(text, opts(o))) yield part
  } finally {
    s.destroy && s.destroy()
  }
}

// a native session that keeps history (a chat); `system` becomes its first prompt
const session = (o = {}) => {
  const { system, ...rest } = o
  return create('prompt', { ...(system ? { initialPrompts: [{ role: 'system', content: system }, ...(rest.initialPrompts || [])] } : {}), ...rest })
}

// one model per kind and options, created on first use and reused
const tools = new Map()
const tool = (kind, o) => {
  const { onProgress, signal, ...rest } = o || {}
  const k = kind + JSON.stringify(rest)
  if (!tools.has(k)) {
    const p = create(kind, { ...rest, ...(onProgress ? { onProgress } : {}) })
    p.catch(() => tools.delete(k))
    tools.set(k, p)
  }
  return tools.get(k)
}
const call = (kind, method, keys) => async (text, o = {}) => {
  const make = {}, run = {}
  for (const [k, v] of Object.entries(o)) (keys.includes(k) ? run : make)[k] = v
  if (o.signal) run.signal = o.signal
  return (await tool(kind, make))[method](text, run)
}

export const ai = {
  ready,
  prompt,
  stream,
  session,
  summarize: call('summarizer', 'summarize', ['context']),
  translate: (text, o = {}) => {
    const { from, to, ...rest } = o
    return call('translator', 'translate', ['signal'])(text, { sourceLanguage: from, targetLanguage: to, ...rest })
  },
  detect: call('detector', 'detect', []),
  write: call('writer', 'write', ['context']),
  rewrite: call('rewriter', 'rewrite', ['context']),
  proofread: call('proofreader', 'proofread', []),
}
