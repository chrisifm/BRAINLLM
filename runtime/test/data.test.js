// brain-data.js: {@html sanitize(x)} → native setHTML when present, else the allowlist sanitizer.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { install } from '../shim/dom.js'

const doc = install()
const rt = await import('./rt.js')
const { sig } = rt
const { sanitize, html, clean } = await rt.sibling('brain-data.js')
const tick = () => new Promise(r => setTimeout(r, 0))
const out = frag => {
  const d = doc.createElement('div')
  d.appendChild(frag)
  return d.innerHTML
}

test('fallback sanitizer: drops scripts and handlers, unwraps unknown tags, filters URLs', () => {
  const h = '<p onclick="x()">hi <b>there</b><script>alert(1)</script><blink>old</blink>' +
    '<a href="javascript:alert(1)">bad</a><a href="/ok" target="_blank">ok</a><img src="data:x" alt="i"></p><iframe src="/x"></iframe>'
  assert.equal(out(clean(h)), '<p>hi <b>there</b>old<a>bad</a><a href="/ok" rel="noopener noreferrer">ok</a><img alt="i"></p>')
})

test('html(): uses native setHTML when the element has it, reactive to changes', async () => {
  const el = doc.createElement('div'), calls = []
  el.setHTML = s => calls.push(s)
  const v = sig('<b>1</b>')
  html(el, () => sanitize(v.v))
  v.v = '<b>2</b>'
  await tick()
  assert.deepEqual(calls, ['<b>1</b>', '<b>2</b>'])
})

test('html(): without sanitize() the value is inserted as text, never parsed', () => {
  const el = doc.createElement('div')
  html(el, () => '<img src=x onerror=alert(1)>')
  assert.equal(el.childNodes.length, 1)
  assert.equal(el.firstChild.nodeType, 3)
})

test('html(): fallback path inserts cleaned nodes', () => {
  const el = doc.createElement('div')
  html(el, () => sanitize('<em>yes</em><script>no()</script>'))
  assert.equal(el.innerHTML, '<em>yes</em>')
})
