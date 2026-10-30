// brain-err maps (INTERNALS §18, #158): in dev an error in a handler is reported at its file.brain:line, through the
// inline source map of each module; two views with the same file name in different folders never mix.
import { test, after } from 'node:test'
import assert from 'node:assert/strict'
import { mkdirSync, mkdtempSync, readFileSync, writeFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'
import { install, MouseEvent } from '../shim/dom.js'

const doc = install()
const rt = await import('./rt.js')
const E = await rt.sibling('brain-err.js')
const errors = E.capture({ maps: true, log: false, load: url => readFileSync(fileURLToPath(url), 'utf8') })
const seen = []
E.sub.add(r => r.src && seen.push(r.src))

// a module "compiled in dev": generated line 6 throws; the map sends every line to `at` except line 6 → `hit`
const B64 = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/'
const vlq = v => {
  let n = v < 0 ? (-v << 1) | 1 : v << 1, o = ''
  do {
    let d = n & 31
    n >>= 5
    if (n) d |= 32
    o += B64[d]
  } while (n)
  return o
}
const dir = mkdtempSync(join(tmpdir(), 'brain-errmap-'))
after(() => rmSync(dir, { recursive: true, force: true }))
function view(rel, name, hit) {
  const js = [
    `import { tpl, on } from ${JSON.stringify(rt.base + 'brain.js')};`,
    `const T = tpl('<button>x</button>');`,
    `export function ${name}() {`,
    `  const r = T();`,
    `  on(r, 'click', () => {`,
    `    throw new TypeError('boom ${name}');`,
    `  });`,
    `  return r;`,
    `}`,
  ]
  let prev = 0
  const mappings = js.map((_, i) => {
    const src = (i === 5 ? hit : 1) - 1
    const seg = vlq(0) + vlq(0) + vlq(src - prev) + vlq(0)
    prev = src
    return seg
  }).join(';')
  const map = { version: 3, sources: [rel.split('/').pop()], sourcesContent: [''], names: [], mappings }
  const f = join(dir, rel)
  mkdirSync(join(f, '..'), { recursive: true })
  writeFileSync(f, js.join('\n') + '\n//# sourceMappingURL=data:application/json;base64,' + Buffer.from(JSON.stringify(map)).toString('base64') + '\n')
  return pathToFileURL(f).href
}
const until = async f => {
  for (let i = 0; i < 100 && !f(); i++) await new Promise(r => setTimeout(r, 5))
}

test('an error thrown in a handler is reported at its .brain line; same file name in two folders does not mix', async () => {
  const { A } = await import(view('views/a.brain.mjs', 'A', 14))
  const { B } = await import(view('views/users/a.brain.mjs', 'B', 3))
  const host = doc.createElement('div')
  doc.body.appendChild(host)
  rt.mount(() => [A(), B()], host)
  host.firstChild.dispatchEvent(new MouseEvent('click', { bubbles: true }))
  host.lastChild.dispatchEvent(new MouseEvent('click', { bubbles: true }))
  await until(() => seen.length >= 2)
  const a = errors.find(r => r.msg === 'boom A'), b = errors.find(r => r.msg === 'boom B')
  assert.ok(a.src.endsWith('views/a.brain.mjs:14'), a.src)
  assert.ok(b.src.endsWith('views/users/a.brain.mjs:3'), b.src)
  assert.ok(!b.src.includes('views/a.brain'), 'no cross-mapping between the two a.brain files')
  assert.equal(a.brainStack[0], a.src)
  assert.equal(a.location.line, 14)
  assert.equal(a.location.column, null, 'line-only maps never invent an original source column')
  assert.equal(a.location.precision, 'source-line')
  assert.equal(a.generatedLocation.line, 6)
  assert.equal(a.frames[0].source.line, 14)
  assert.equal(a.mapping.status, 'mapped')
})

test('a module without a source map (prod) keeps the generated stack and gets no src', async () => {
  const f = join(dir, 'plain.mjs')
  writeFileSync(f, `import { tpl, on } from ${JSON.stringify(rt.base + 'brain.js')};\nexport function P() { const r = tpl('<i>p</i>')(); on(r, 'click', () => { throw new Error('plain') }); return r }\n`)
  const { P } = await import(pathToFileURL(f).href)
  const host = doc.createElement('div')
  doc.body.appendChild(host)
  rt.mount(P, host)
  host.firstChild.dispatchEvent(new MouseEvent('click', { bubbles: true }))
  await new Promise(r => setTimeout(r, 30))
  const r = errors.find(r => r.msg === 'plain')
  assert.ok(r, 'recorded')
  assert.equal(r.src, undefined)
  assert.ok(r.stack.some(l => l.includes('plain.mjs')))
})


test('nested causes acquire independent source coordinates before the enriched notification', async () => {
  const outerUrl = view('views/outer.brain.mjs', 'Outer', 27)
  const causeUrl = view('comp/cause.brain.mjs', 'Cause', 9)
  const cause = new Error('cause detail')
  cause.stack = `Error\n    at cause (${causeUrl}:6:4)`
  const e = new Error('mapped outer', { cause })
  e.stack = `Error\n    at outer (${outerUrl}:6:8)`
  let observed = null
  const listener = r => { if (r.msg === 'mapped outer' && r.src) observed = r.causes[0].location.line }
  E.sub.add(listener)
  const stop = rt.fx(() => { throw e })
  stop()
  await until(() => observed !== null)
  E.sub.delete(listener)
  const r = errors.find(r => r.msg === 'mapped outer')
  assert.equal(r.location.line, 27)
  assert.equal(r.causes[0].location.line, 9)
  assert.equal(r.causes[0].location.column, null)
  assert.equal(r.causes[0].generatedLocation.column, 4)
  assert.equal(observed, 9)
})

test('source mapping has a total deadline, finite concurrency and aborts owned loads', async () => {
  let active = 0, peak = 0, aborted = 0
  E.capture({ maps: true, load: (_url, { signal }) => new Promise((_, reject) => {
    active++; peak = Math.max(peak, active)
    signal.addEventListener('abort', () => { active--; aborted++; reject(new Error('aborted map')) }, { once: true })
  }) })
  const before = errors.length
  for (let i = 0; i < 3; i++) {
    const e = new Error(`deadline ${i}`)
    e.stack = `Error\n    at action (file:///deadline-${i}.brain:20:3)`
    const stop = rt.fx(() => { throw e })
    stop()
  }
  await new Promise(resolve => setTimeout(resolve, 2100))
  const records = errors.slice(before)
  assert.equal(records.length, 3)
  assert.ok(records.every(r => r.mapping.status === 'timeout'))
  assert.ok(records.every(r => r.truncated.mapping && r.generatedLocation.line === 20))
  assert.equal(peak, 2)
  assert.equal(active, 0)
  assert.ok(aborted >= 2 && aborted <= 3, 'only started loads are aborted; a queued request can briefly start before its own deadline')
})
