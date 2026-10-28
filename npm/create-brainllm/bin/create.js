#!/usr/bin/env node
// `npm create @christianirack/brainllm <name>` (also `npx @christianirack/create-brainllm <name>`): runs `brain new`
// with the brain this package depends on (same version), then installs the app's pinned devDependency.
// Flags after the name go to `brain new`; `--no-install` skips `npm install`.
'use strict'
const { spawnSync } = require('node:child_process')
const { join } = require('node:path')

const args = process.argv.slice(2)
const name = args.find((a) => !a.startsWith('-'))
if (!name) {
  process.stderr.write('usage: npm create @christianirack/brainllm <app-name>\n')
  process.exit(2)
}
const launcher = require.resolve('@christianirack/brainllm/bin/brain.js')
const run = (cmd, argv, cwd) => {
  const r = spawnSync(cmd, argv, { stdio: 'inherit', cwd, shell: process.platform === 'win32' && cmd === 'npm' })
  if (r.status !== 0) process.exit(r.status ?? 1)
}
// BRAIN_NO_DELEGATE: the new app has no node_modules yet; use this package's brain
process.env.BRAIN_NO_DELEGATE = '1'
run(process.execPath, [launcher, 'new', ...args.filter((a) => a !== '--no-install')], process.cwd())
if (!args.includes('--no-install')) run('npm', ['install', '--no-audit', '--no-fund'], join(process.cwd(), name))
process.stdout.write(`\nnext: cd ${name} && npx brain run\n`)
