#!/usr/bin/env node
// Baseline table for the B12xx compatibility rules (INTERNALS §20), generated from the web-features package
// (https://github.com/web-platform-dx/web-features, Apache-2.0).
//
//   node crates/brain-cli/tools/baseline.mjs [--version X.Y.Z] [--from <unpacked package dir>] [--out <file>]
//
// Without --from it runs `npm pack web-features@<version>` (default: the latest) into a temporary folder.
// The output is a dated TSV, committed with the code that embeds it (include_str!): building the brain binary never
// touches the network. Only keys that are NOT widely available, or that are discouraged, are written: a key missing
// from the table is Baseline widely available (or unknown to web-features) and never flagged.
//
//   # web-features <version> <date> · Apache-2.0 · <rows> rows
//   <compat key> \t <low|no|high> \t <baseline low date or -> \t <feature id> \t <discouraged: 1 or 0> \t <alternative ids or ->
//
// `low` = Baseline newly available (fails "baseline-widely"), `no` = limited availability (needs feature
// detection), `high` = widely available but discouraged (B1203). Compat keys look like `api.Element.moveBefore`,
// `css.properties.field-sizing`, `javascript.builtins.Array.fromAsync`, `html.elements.dialog`.
import { execFileSync } from 'node:child_process'
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

const argv = process.argv.slice(2)
const opt = (k) => (argv.includes(k) ? argv[argv.indexOf(k) + 1] : null)
let dir = opt('--from')
let tmp = null
if (!dir) {
  const version = opt('--version') ?? execFileSync('npm', ['view', 'web-features', 'version'], { encoding: 'utf8' }).trim()
  tmp = mkdtempSync(join(tmpdir(), 'web-features-'))
  const tgz = execFileSync('npm', ['pack', `web-features@${version}`, '--silent', '--pack-destination', tmp], { encoding: 'utf8' }).trim().split('\n').pop()
  execFileSync('tar', ['xzf', join(tmp, tgz), '-C', tmp])
  dir = join(tmp, 'package')
}
const pkg = JSON.parse(readFileSync(join(dir, 'package.json'), 'utf8'))
const { features } = JSON.parse(readFileSync(join(dir, 'data.json'), 'utf8'))

const rows = new Map()
for (const [id, f] of Object.entries(features)) {
  if (f.kind && f.kind !== 'feature') continue
  const discouraged = f.discouraged ? 1 : 0
  const alt = f.discouraged?.alternatives?.length ? f.discouraged.alternatives.join(',') : '-'
  for (const [key, s] of Object.entries(f.status?.by_compat_key ?? {})) {
    if (!/^(api|css|javascript|html)\./.test(key)) continue
    const level = s.baseline === 'high' ? 'high' : s.baseline === 'low' ? 'low' : 'no'
    if (level === 'high' && !discouraged) continue
    // a key listed by two features keeps the less available status
    const prev = rows.get(key)
    const rank = { no: 0, low: 1, high: 2 }
    if (prev && rank[prev[0]] <= rank[level] && !(discouraged && !prev[3])) continue
    rows.set(key, [level, s.baseline_low_date ?? '-', id, discouraged, alt])
  }
}
const keys = [...rows.keys()].sort()
const date = new Date().toISOString().slice(0, 10)
let out = `# web-features ${pkg.version} ${date} · Apache-2.0 · ${keys.length} rows\n`
for (const k of keys) out += [k, ...rows.get(k)].join('\t') + '\n'
const to = opt('--out')
if (to) {
  writeFileSync(to, out)
  console.error(`${to}: web-features ${pkg.version}, ${keys.length} rows, ${(out.length / 1024).toFixed(0)} KB`)
} else process.stdout.write(out)
if (tmp) rmSync(tmp, { recursive: true, force: true })
