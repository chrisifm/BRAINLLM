#!/usr/bin/env node
// Read-only publication gate. Findings contain locations/categories, never matched secret values.
import { execFileSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import { readFileSync, lstatSync, existsSync } from 'node:fs'
import { resolve, join } from 'node:path'
import { pathToFileURL } from 'node:url'
import { git } from './lib/jobs.mjs'

const MAX = 8 * 1024 * 1024
const rules = [
  ['private-key', /-----BEGIN (?:[A-Z0-9]+ )*PRIVATE KEY-----/g],
  ['credential', /\b(?:sk-(?:ant-)?[A-Za-z0-9_-]{20,}|gh[pousr]_[A-Za-z0-9]{20,}|github_pat_[A-Za-z0-9_]{30,}|AKIA[A-Z0-9]{16}|npm_[A-Za-z0-9]{30,})\b/g],
  ['credential-url', /\b(?:https?|postgres(?:ql)?|mysql|mongodb):\/\/[^\s/:@]+:[^\s/@]+@/g],
  ['home-path', /(?:\/Users\/|\/home\/|[A-Z]:\\Users\\)[A-Za-z0-9_.-]+[\/\\]/g],
]
const forbidden = p => /(^|\/)(?:\.env(?:\..*)?|\.brain_env|\.npmrc|\.netrc|id_rsa|id_ed25519|credentials(?:\.json)?|\.claude|\.codex)(\/|$)|\.(?:pem|p12|pfx|key|sqlite|sqlite3)$/i.test(p)
const digest = s => createHash('sha256').update(s).digest('hex')

export function inspect(path, data, exceptions = []) {
  const out = []
  if (forbidden(path)) out.push({ path, rule: 'private-file' })
  if (data.length > MAX) return [...out, { path, rule: 'oversized-unreviewed-file' }]
  const text = data.toString('utf8')
  for (const [rule, pattern] of rules) {
    pattern.lastIndex = 0
    for (const m of text.matchAll(pattern)) {
      if (exceptions.some(e => e.path === path && e.rule === rule && e.sha256 === digest(m[0]))) continue
      out.push({ path, rule, line: text.slice(0, m.index).split('\n').length })
    }
  }
  return out
}

export function audit(root, { history = false, exceptions = [] } = {}) {
  const findings = [], seen = new Set()
  let files = 0, blobs = 0
  const paths = git(root, 'ls-files', '--cached', '--others', '--exclude-standard', '-z').split('\0').filter(Boolean)
  for (const p of new Set(paths)) {
    const full = join(root, p)
    if (!existsSync(full)) continue
    const stat = lstatSync(full)
    if (!stat.isFile()) { findings.push({ path: p, rule: 'nonregular-file' }); continue }
    files++
    if (stat.size > MAX) findings.push({ path: p, rule: 'oversized-unreviewed-file' })
    else findings.push(...inspect(p, readFileSync(full), exceptions))
  }
  if (history) {
    // Scan every reachable blob, including removed files. Commit messages and identities matter as well.
    const objects = git(root, 'rev-list', '--objects', '--all', 'HEAD').split('\n').filter(Boolean)
    for (const row of objects) {
      const [oid, ...name] = row.split(' '), path = name.join(' ') || '(commit metadata)'
      if (seen.has(oid)) continue
      seen.add(oid)
      const kind = git(root, 'cat-file', '-t', oid)
      if (kind !== 'blob' && kind !== 'commit') continue
      const size = Number(git(root, 'cat-file', '-s', oid))
      if (size > MAX) { findings.push({ path, object: oid, rule: 'oversized-unreviewed-file' }); continue }
      const data = execFileSync('git', ['cat-file', kind, oid], { cwd: root, maxBuffer: MAX + 1, timeout: 30_000 })
      blobs++
      findings.push(...inspect(path, data, exceptions).map(f => ({ ...f, object: oid })))
    }
  }
  return { schemaVersion: 1, ok: findings.length === 0, files, historyObjects: blobs, findings,
    coverage: 'Known credential patterns, private paths and reachable Git objects. Public author identity is intentional. Not proof that arbitrary secrets or personal data are absent.' }
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  try {
    const root = resolve(process.argv.find(a => a.startsWith('--root='))?.slice(7) || '.')
    const allow = join(root, 'scripts/privacy-fixtures.json')
    const result = audit(root, { history: process.argv.includes('--history'), exceptions: existsSync(allow) ? JSON.parse(readFileSync(allow)) : [] })
    console.log(JSON.stringify(result, null, 2))
    process.exitCode = result.ok ? 0 : 1
  } catch { console.log(JSON.stringify({ schemaVersion: 1, ok: false, error: 'Audit incomplete; inspect repository readability and Git availability.' })); process.exitCode = 2 }
}
