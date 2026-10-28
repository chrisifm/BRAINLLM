#!/usr/bin/env node
// `brain` launcher (INTERNALS §23): runs the native binary of this platform, shipped in one of the
// @christianirack/brainllm-<os>-<arch> optional dependencies. No postinstall, no download, no dependencies.
//
// 1. A global `brain` delegates to the version the project pins (node_modules/@christianirack/brainllm of the nearest
//    package.json above the working directory), so two projects on different versions coexist.
// 2. BRAIN_BINARY=<path> runs that binary instead (local builds, unsupported platforms).
// 3. Otherwise the platform package's bin/brain(.exe); its exit code and signal pass through unchanged.
'use strict'
const { spawnSync } = require('node:child_process')
const { existsSync, realpathSync } = require('node:fs')
const { dirname, join, resolve } = require('node:path')

const SCOPE = '@christianirack'
const here = realpathSync(__dirname)

function projectLauncher() {
  if (process.env.BRAIN_NO_DELEGATE) return null
  let dir = process.cwd()
  for (;;) {
    const cand = join(dir, 'node_modules', SCOPE, 'brainllm', 'bin', 'brain.js')
    if (existsSync(cand)) {
      const real = realpathSync(cand)
      return dirname(real) === here ? null : real
    }
    const up = dirname(dir)
    if (up === dir) return null
    dir = up
  }
}

function isMusl() {
  if (process.platform !== 'linux') return false
  try {
    return !process.report.getReport().header.glibcVersionRuntime
  } catch {
    return existsSync('/etc/alpine-release')
  }
}

function platformPackage() {
  const { platform, arch } = process
  const libc = isMusl() ? '-musl' : ''
  const supported = ['darwin-arm64', 'darwin-x64', 'linux-x64', 'linux-arm64', 'linux-x64-musl', 'win32-x64', 'win32-arm64']
  const key = `${platform}-${arch}${libc}`
  return supported.includes(key) ? `${SCOPE}/brainllm-${key}` : null
}

function binary() {
  if (process.env.BRAIN_BINARY) return resolve(process.env.BRAIN_BINARY)
  const pkg = platformPackage()
  if (!pkg) {
    fail(`no prebuilt brain for ${process.platform}-${process.arch}; fix: build it with \`cargo build --release -p brain-cli\` and set BRAIN_BINARY to target/release/brain`)
  }
  const exe = process.platform === 'win32' ? 'brain.exe' : 'brain'
  try {
    return join(dirname(require.resolve(`${pkg}/package.json`, { paths: [here] })), 'bin', exe)
  } catch {
    fail(`${pkg} is not installed (it is an optional dependency of ${SCOPE}/brainllm); fix: reinstall without --no-optional / --omit=optional, e.g. \`npm install ${SCOPE}/brainllm\``)
  }
}

function fail(msg) {
  process.stderr.write(`error: ${msg}\n`)
  process.exit(3)
}

const delegate = projectLauncher()
const [cmd, args] = delegate ? [process.execPath, [delegate, ...process.argv.slice(2)]] : [binary(), process.argv.slice(2)]
const r = spawnSync(cmd, args, { stdio: 'inherit', windowsHide: true })
if (r.error) fail(`could not run ${cmd}: ${r.error.message}`)
if (r.signal) process.kill(process.pid, r.signal)
process.exit(r.status ?? 1)
