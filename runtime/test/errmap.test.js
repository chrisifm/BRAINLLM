// brain-err maps (INTERNALS §18, #158): in dev an error in a handler is reported at its file.brain:line, through the
// inline source map of each module; two views with the same file name in different folders never mix.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { mkdirSync, mkdtempSync, readFileSync, writeFileSync } from 'node:fs'
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
