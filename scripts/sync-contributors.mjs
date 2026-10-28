#!/usr/bin/env node
// Public account attribution only. No emails, private profiles or write API calls.
import { readFileSync, writeFileSync, mkdirSync, renameSync, unlinkSync } from 'node:fs'
import { resolve, dirname } from 'node:path'
import { fileURLToPath } from 'node:url'

const REPO = 'chrisifm/BRAINLLM'
const START = '<!-- contributors:start -->', END = '<!-- contributors:end -->'
export function normalize(rows) {
  const accounts = new Map()
  for (const row of rows) {
    if (!['User', 'Bot'].includes(row.type) || !/^[a-z\d][a-z\d-]{0,38}(?:\[bot\])?$/i.test(row.login ?? '')) throw new Error('Unexpected contributor account; previous attribution preserved')
    const avatar = new URL(row.avatar_url)
    if (avatar.protocol !== 'https:' || avatar.hostname !== 'avatars.githubusercontent.com' || avatar.username || avatar.password || avatar.hash) throw new Error('Untrusted avatar URL; previous attribution preserved')
    if (!Number.isSafeInteger(row.contributions) || row.contributions < 0) throw new Error('Invalid contribution count')
    accounts.set(row.login, { login: row.login, profileUrl: `https://github.com/${encodeURIComponent(row.login)}`, avatarUrl: avatar.href, contributions: row.contributions, type: row.type })
  }
  if (!accounts.size) throw new Error('Empty contributor response; previous attribution preserved')
  return [...accounts.values()].sort((a, b) => b.contributions - a.contributions || a.login.localeCompare(b.login, 'en'))
}

export async function collect(fetcher = fetch, token = process.env.GH_TOKEN || process.env.GITHUB_TOKEN) {
  const signal = AbortSignal.timeout(120000), all = []
  for (let page = 1; page <= 20; page++) {
    const url = `https://api.github.com/repos/${REPO}/contributors?per_page=100&page=${page}&anon=false`
    let response
    for (let attempt = 0; attempt < 3; attempt++) {
      response = await fetcher(url, { redirect: 'error', signal, headers: { Accept: 'application/vnd.github+json', 'X-GitHub-Api-Version': '2022-11-28', ...(token ? { Authorization: `Bearer ${token}` } : {}) } })
      if (response.status !== 202) break
      await response.body?.cancel()
      await new Promise(r => setTimeout(r, 500))
    }
    if (response.status !== 200) throw new Error(`Contributor API returned HTTP ${response.status}; previous attribution preserved`)
    // Bound the response before JSON parsing, including a server without Content-Length.
    const reader = response.body.getReader(), chunks = []; let bytes = 0
    try {
      for (;;) {
        const { done, value } = await reader.read()
        if (done) break
        bytes += value.byteLength
        if (bytes > 2 * 1024 * 1024) throw new Error('Contributor response exceeds 2 MiB')
        chunks.push(value)
      }
    } finally { await reader.cancel().catch(() => {}) }
    const rows = JSON.parse(Buffer.concat(chunks).toString('utf8'))
    if (!Array.isArray(rows) || rows.length > 100) throw new Error('Invalid contributor response')
    all.push(...rows)
    const next = /rel="next"/.test(response.headers.get('link') || '')
    if (!next && rows.length < 100) return normalize(all)
    if (!next && rows.length === 0) return normalize(all)
    // A full page without Link may still have more records; check the next numbered page.
  }
  throw new Error('Contributor pagination exceeds 2,000 accounts; previous attribution preserved')
}

export function render(readme, contributors) {
  const lines = [START, '## Built by contributors', '', 'Every merged contribution helps the next developer and their agent. Thank you to the public GitHub accounts credited below.', '',
    ...contributors.map(c => `[<img src="${c.avatarUrl.replace(/&/g, '&amp;').replace(/"/g, '&quot;')}" width="48" height="48" alt="${c.login.replace(/\[/g, '&#91;').replace(/\]/g, '&#93;')}">](${c.profileUrl}) [${c.login.replace(/([\[\]])/g, '\\$1')}](${c.profileUrl})`),
    '', 'Attribution is refreshed through a reviewed pull request. [How it works](docs/CONTRIBUTORS.md).', END]
  const block = lines.join('\n')
  const a = readme.indexOf(START), b = readme.indexOf(END)
  if (a < 0 && b < 0) return `${readme.trimEnd()}\n\n${block}\n`
  if (a < 0 || b < a || readme.indexOf(START, a + START.length) >= 0 || readme.indexOf(END, b + END.length) >= 0) throw new Error('Ambiguous contributor markers; README preserved')
  return readme.slice(0, a) + block + readme.slice(b + END.length)
}

export async function sync(root, fetcher = fetch) {
  const contributors = await collect(fetcher)
  const readmePath = resolve(root, 'README.md'), snapshotPath = resolve(root, 'docs/site/contributors.json')
  const readme = render(readFileSync(readmePath, 'utf8'), contributors)
  const snapshot = JSON.stringify({ schemaVersion: 1, repository: REPO, contributors }, null, 2) + '\n'
  // Fetch, validate and render everything before replacing either destination.
  const staged = []
  try {
    for (const [path, content] of [[readmePath, readme], [snapshotPath, snapshot]]) {
      mkdirSync(dirname(path), { recursive: true })
      const temp = `${path}.${process.pid}.tmp`
      staged.push(temp)
      writeFileSync(temp, content, { flag: 'wx' })
    }
    renameSync(staged[0], readmePath); renameSync(staged[1], snapshotPath)
  } finally { for (const temp of staged) { try { unlinkSync(temp) } catch {} } }
  return contributors.length
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try { console.log(`Updated attribution for ${await sync(process.argv.includes('--root') ? resolve(process.argv[process.argv.indexOf('--root') + 1]) : resolve(fileURLToPath(new URL('..', import.meta.url))))} public contributor accounts.`) }
  catch (error) { console.error(`Contributor sync failed: ${error.message}`); process.exitCode = 1 }
}
