# guide: ai — Chrome's built-in AI (Gemini Nano) on the device, when the browser has it

## When
A feature can run on the user's device with no server and no API key: summarize an article, translate a comment,
detect a language, suggest a reply, rewrite a draft, fill a form from free text. The model is Chrome's Gemini Nano;
other browsers have none, so every feature built on it is an extra, never the only way to do the task.

## What exists (`ai` is a primitive: no import)
| Call | Returns | Browser API (status 2026-10-29) |
|---|---|---|
| `await ai.ready(kind?)` | `'available'`, `'downloadable'`, `'downloading'` or `'unavailable'` | all of them |
| `await ai.prompt(text, { system, schema, signal, onProgress })` | string, or the parsed object with `schema` | Prompt API: origin trial on the web, stable in extensions |
| `for await (const part of ai.stream(text, o))` | text chunks | Prompt API |
| `await ai.session({ system })` | native session with history: `prompt`, `append`, `clone`, `destroy` | Prompt API |
| `await ai.summarize(text, { type, format, length, context })` | string | Summarizer, stable since Chrome 138 |
| `await ai.translate(text, { from, to })` | string | Translator, stable since Chrome 138 |
| `await ai.detect(text)` | `[{ detectedLanguage, confidence }]` | LanguageDetector, stable since Chrome 138 |
| `await ai.write(task)` · `ai.rewrite(text, { tone })` · `ai.proofread(text)` | string or corrections | trials |
`kind` is `prompt` (default), `summarizer`, `translator`, `detector`, `writer`, `rewriter` or `proofreader`.

## Steps
1. Check before showing the feature: `let can = sig(false)` and `mounted(async () => (can = (await ai.ready('summarizer')) !== 'unavailable'))`.
   Render the button only `{#if can}`; on the server and in other browsers it stays hidden.
2. Call it from an event handler the first time: a model that is `'downloadable'` downloads on first use, and Chrome
   only allows that after a user gesture. Show the download with `onProgress: p => (progress = p)` (0 to 1).
3. Every call can fail (`e.code === 'unavailable'`, a quota, an abort): wrap it in `try`/`catch` and keep the manual
   path working.
4. Stateless questions use `ai.prompt` (each call starts clean). A chat keeps one `ai.session()` and calls
   `destroy()` when the view unmounts (return it from `mounted`).
5. Structured output: pass a JSON Schema as `schema` and read the object it returns; never parse free text.
6. To use the Prompt API on a public site, register the origin trial and add its token to the layout's `<head>`.
   Third-party iframes need `allow="language-model"`; it does not run in Web Workers.

## Verify
```bash
brain check --json      # []
brain run               # open the view in Chrome; without the model the feature stays hidden and nothing logs errors
```

## Common mistakes
- Assuming the model exists: a page that only works with `ai` breaks for every other browser. Keep the manual path.
- Calling `ai.prompt` on load: the first download needs a click; call it from a handler.
- Sending private data to a server "as a fallback" without saying so: the point of `ai` is that data stays on the device.

## Minimal correct example
```html
<meta props="text:str">

<script>
  let can = sig(false)
  let summary = sig('')
  let busy = sig(false)
  mounted(async () => (can = (await ai.ready('summarizer')) !== 'unavailable'))
  async function summarize() {
    busy = true
    try {
      summary = await ai.summarize(text, { type: 'tldr', length: 'short' })
    } catch (e) {
      summary = ''
    }
    busy = false
  }
</script>

{#if can}
  <Button busy={busy} on:click={summarize}>Summarize on this device</Button>
  {#if summary}<p>{summary}</p>{/if}
{/if}
```
