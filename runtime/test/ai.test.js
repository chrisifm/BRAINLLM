// brain-ai.js: Chrome's built-in AI behind `ai`, feature-detected; 'unavailable' when the browser has no model.
import { test } from 'node:test'
import assert from 'node:assert/strict'

const rt = await import('./rt.js')
const { ai } = await rt.sibling('brain-ai.js')

test('without the browser APIs: ready() is unavailable and calls throw code "unavailable"', async () => {
  assert.equal(await ai.ready(), 'unavailable')
  assert.equal(await ai.ready('summarizer'), 'unavailable')
  await assert.rejects(ai.prompt('hi'), e => e.code === 'unavailable')
  await assert.rejects(ai.translate('hi', { from: 'en', to: 'es' }), e => e.code === 'unavailable')
  await assert.rejects(ai.ready('vision'), /unknown model `vision`/)
})

test('Prompt API: stateless prompts run on clones of one base session; schema parses JSON; progress is wired', async () => {
  const made = [], cloned = []
  let destroyed = 0
  globalThis.LanguageModel = {
    availability: async o => (o.expectedInputs ? 'downloadable' : 'available'),
    create: async o => {
      made.push(o)
      if (o.monitor) o.monitor({ addEventListener: (t, f) => t === 'downloadprogress' && f({ loaded: 0.5 }) })
      const s = {
        clone: async () => (cloned.push(1), s),
        prompt: async (t, x) => (x && x.responseConstraint ? '{"n":3}' : `echo:${t}`),
        async *promptStreaming(t) {
          yield 'a'
          yield 'b'
        },
        destroy: () => destroyed++,
      }
      return s
    },
  }
  assert.equal(await ai.ready(), 'available')
  assert.equal(await ai.ready('prompt', { expectedInputs: [{ type: 'image' }] }), 'downloadable')
  const seen = []
  assert.equal(await ai.prompt('x', { system: 'be brief', onProgress: p => seen.push(p) }), 'echo:x')
  assert.equal(await ai.prompt('y', { system: 'be brief' }), 'echo:y')
  assert.equal(made.length, 1) // one base per system prompt
  assert.deepEqual(made[0].initialPrompts, [{ role: 'system', content: 'be brief' }])
  assert.deepEqual(seen, [0.5])
  assert.equal(cloned.length, 2)
  assert.equal(destroyed, 2)
  assert.deepEqual(await ai.prompt('z', { schema: { type: 'object' } }), { n: 3 })
  let out = ''
  for await (const p of ai.stream('s')) out += p
  assert.equal(out, 'ab')
  const chat = await ai.session({ system: 'you are kind' })
  assert.equal(await chat.prompt('hello'), 'echo:hello')
  delete globalThis.LanguageModel
})

test('task models: created once per options, call options passed per call', async () => {
  let made = 0
  globalThis.Translator = { availability: async o => (o.sourceLanguage === 'en' && o.targetLanguage === 'es' ? 'available' : 'unavailable'), create: async o => (made++, { translate: async t => `${o.sourceLanguage}>${o.targetLanguage}:${t}` }) }
  globalThis.Summarizer = { create: async o => ({ summarize: async (t, r) => `${o.type}|${r.context}|${t}` }) }
  assert.equal(await ai.ready('translator', { from: 'en', to: 'es' }), 'available')
  assert.equal(await ai.ready('translator', { from: 'en', to: 'xx' }), 'unavailable')
  assert.equal(await ai.translate('hi', { from: 'en', to: 'es' }), 'en>es:hi')
  assert.equal(await ai.translate('bye', { from: 'en', to: 'es' }), 'en>es:bye')
  assert.equal(made, 1)
  assert.equal(await ai.summarize('long text', { type: 'tldr', context: 'news' }), 'tldr|news|long text')
  delete globalThis.Translator
  delete globalThis.Summarizer
})
