#!/usr/bin/env node
// Helpers for the release workflow's GitHub Release job. Pure functions are tested in npm/test/github-release.test.mjs.
//   node npm/github-release.mjs notes <version>                                       prints that CHANGELOG section's body
//   node npm/github-release.mjs agree <version> <tag|-> <npm latest|-> <release tag|->  exits 1 when they disagree
import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { resolve } from 'node:path'

const VERSION = /^(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)(?:-[0-9A-Za-z]+(?:[.-][0-9A-Za-z]+)*)?$/

export function releaseNotes(changelog, version) {
  if (!VERSION.test(version)) throw Error(`not an exact version: ${version}`)
  const lines = changelog.split('\n')
  const start = lines.findIndex((line) => line === `## [${version}]` || line.startsWith(`## [${version}] `))
  if (start < 0) throw Error(`CHANGELOG.md has no section for ${version}`)
  const rest = lines.slice(start + 1)
  const end = rest.findIndex((line) => line.startsWith('## '))
  const body = (end < 0 ? rest : rest.slice(0, end)).join('\n').trim()
  if (!body) throw Error(`the CHANGELOG.md section for ${version} is empty`)
  return `${body}\n`
}

// Everything a reader can look at for "the current release" must name the same version.
export function disagreements({ version, tag, npmLatest, releaseTag }) {
  const expected = `v${version}`
  const found = []
  if (tag !== expected) found.push(`git tag: expected ${expected}, found ${tag || 'none'}`)
  if (npmLatest !== version) found.push(`npm latest: expected ${version}, found ${npmLatest || 'none'}`)
  if (releaseTag !== expected) found.push(`latest GitHub Release: expected ${expected}, found ${releaseTag || 'none'}`)
  return found
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const [command, version, tag, npmLatest, releaseTag] = process.argv.slice(2)
  const given = (value) => (value === '-' ? '' : value)
  if (command === 'notes') {
    process.stdout.write(releaseNotes(readFileSync('CHANGELOG.md', 'utf8'), version))
  } else if (command === 'agree') {
    const found = disagreements({ version, tag: given(tag), npmLatest: given(npmLatest), releaseTag: given(releaseTag) })
    for (const line of found) console.error(line)
    if (found.length) process.exit(1)
    console.log(`v${version}: tag, npm latest and GitHub Release agree`)
  } else {
    console.error('usage: github-release.mjs notes <version> | agree <version> <tag> <npm latest> <release tag>')
    process.exit(2)
  }
}
