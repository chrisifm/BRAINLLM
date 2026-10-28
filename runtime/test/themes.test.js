// #82 theme presets (runtime/themes/*.cssbrain, written by `brain theme <preset>`): same tokens as the starter theme,
// and every text pair the kit uses passes WCAG AA (4.5:1); `contrast` passes AAA (7:1).
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { readdirSync, readFileSync } from 'node:fs'

const dir = new URL('../themes/', import.meta.url)
const themes = readdirSync(dir).filter(f => f.endsWith('.cssbrain'))
const tokens = css => Object.fromEntries([...css.split('@layer')[0].matchAll(/(--[\w-]+):\s*([^;]+);/g)].map(m => [m[1], m[2].trim()]))
const lum = hex => {
  const c = hex.replace('#', '').match(/../g).map(x => parseInt(x, 16) / 255).map(v => (v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4))
  return 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}
const ratio = (a, b) => {
  const [x, y] = [lum(a), lum(b)].sort((p, q) => q - p)
  return (x + 0.05) / (y + 0.05)
}
const PAIRS = [
  ...['fg', 'muted', 'accent', 'ok', 'bad'].flatMap(t => ['bg', 'surface'].map(b => [t, b])),
  ['on-accent', 'accent'], ['on-accent', 'ok'], ['on-accent', 'bad'],
]

test('presets: glass, dark, paper and contrast exist and define every token of the starter theme', () => {
  assert.deepEqual(themes.sort(), ['contrast.cssbrain', 'dark.cssbrain', 'glass.cssbrain', 'paper.cssbrain'])
  const starter = Object.keys(tokens(readFileSync(new URL('../../crates/brain-cli/src/templates/tokens.cssbrain', import.meta.url), 'utf8'))).sort()
  for (const f of themes) assert.deepEqual(Object.keys(tokens(readFileSync(new URL(f, dir), 'utf8'))).sort(), starter, f)
})

test('every text pair passes AA (4.5:1); the contrast preset passes AAA (7:1)', () => {
  for (const f of themes) {
    const t = tokens(readFileSync(new URL(f, dir), 'utf8')), min = f === 'contrast.cssbrain' ? 7 : 4.5
    for (const [fg, bg] of PAIRS) {
      const r = ratio(t['--' + fg], t['--' + bg])
      assert.ok(r >= min, `${f}: --${fg} on --${bg} is ${r.toFixed(2)}:1, needs ${min}:1`)
    }
  }
})
