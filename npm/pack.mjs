#!/usr/bin/env node
// npm packages of BRAINLLM (INTERNALS §23), lockstep with the workspace version in Cargo.toml:
//   @christianirack/brainllm                 launcher (bin/brain.js) + brain.schema.json; one optional dependency
//                                            per platform, so npm installs only the matching binary
//   @christianirack/brainllm-<os>-<arch>     the native `brain` binary (7 platforms; `os`/`cpu`/`libc` select it)
//   @christianirack/create-brainllm          `npm create @christianirack/brainllm <app>`
//   @christianirack/create-brainll           shorthand initializer with the same pinned compiler
//   @christianirack/brainllm-runtime         the runtime ES modules (already embedded in the binary; for tooling)
//
//   node npm/pack.mjs [--dry-run] [--bins <dir>] [--require-all] [--no-build]
//
// The host's binary is built into target/npm with remapped paths; other platforms come from target/<triple>/release/brain or, with --bins, from
// <dir>/<os>-<arch>/brain[.exe] (the CI artifacts). Output: npm/dist/stage/<package>/, npm/dist/*.tgz and
// npm/dist/SHA256SUMS. A platform without a binary is skipped and listed; --require-all makes that an error (CI).
// This script never publishes: publishing is Chris's call, version by version (npm/README.md).
import { execFileSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import { chmodSync, copyFileSync, cpSync, existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))
const root = join(here, '..')
const SCOPE = '@christianirack'
const argv = process.argv.slice(2)
const dry = argv.includes('--dry-run')
const requireAll = argv.includes('--require-all')
const binsAt = argv.includes('--bins') ? argv[argv.indexOf('--bins') + 1] : null

const cargo = readFileSync(join(root, 'Cargo.toml'), 'utf8')
const version = /\[workspace\.package\][^[]*?\nversion\s*=\s*"([^"]+)"/.exec(cargo)?.[1]
const license = /\[workspace\.package\][^[]*?\nlicense\s*=\s*"([^"]+)"/.exec(cargo)?.[1] ?? 'UNLICENSED'
if (!version) throw new Error('no [workspace.package] version in Cargo.toml')

const PLATFORMS = [
  { key: 'darwin-arm64', triple: 'aarch64-apple-darwin', os: 'darwin', cpu: 'arm64' },
  { key: 'darwin-x64', triple: 'x86_64-apple-darwin', os: 'darwin', cpu: 'x64' },
  { key: 'linux-x64', triple: 'x86_64-unknown-linux-gnu', os: 'linux', cpu: 'x64', libc: 'glibc' },
  { key: 'linux-arm64', triple: 'aarch64-unknown-linux-gnu', os: 'linux', cpu: 'arm64', libc: 'glibc' },
  { key: 'linux-x64-musl', triple: 'x86_64-unknown-linux-musl', os: 'linux', cpu: 'x64', libc: 'musl' },
  { key: 'win32-x64', triple: 'x86_64-pc-windows-msvc', os: 'win32', cpu: 'x64', exe: '.exe' },
  { key: 'win32-arm64', triple: 'aarch64-pc-windows-msvc', os: 'win32', cpu: 'arm64', exe: '.exe' },
]
const hostTriple = /host: (\S+)/.exec(execFileSync('rustc', ['-vV'], { encoding: 'utf8' }))?.[1]
// the host's binary is built here, in its own target dir, with this machine's paths remapped out of it (dependency
// panic locations would otherwise expose ~/.cargo/registry/…); --bins (CI) and --no-build skip it
if (!binsAt && !argv.includes('--no-build')) {
  const flags = [`--remap-path-prefix=${process.env.HOME}=~`, `--remap-path-prefix=${root}=/brainllm`].join(' ')
  execFileSync('cargo', ['build', '--release', '--locked', '-p', 'brain-cli', '--target-dir', join(root, 'target/npm')], { cwd: root, stdio: ['ignore', 'inherit', 'inherit'], env: { ...process.env, RUSTFLAGS: flags } })
}

const out = join(here, 'dist')
const stage = join(out, 'stage')
rmSync(out, { recursive: true, force: true })
mkdirSync(stage, { recursive: true })

// Every scoped package is public, MIT licensed and attributed to the principal author.
const writePkg = (dir, pkg) => {
  copyFileSync(join(root, 'LICENSE'), join(dir, 'LICENSE'))
  writeFileSync(join(dir, 'package.json'), JSON.stringify({ name: pkg.name, version, ...pkg, license, author: 'Christian Irack Flores Maldonado <christian@irack.mx> (https://ingenieria.dev)', repository: { type: 'git', url: 'git+https://github.com/chrisifm/BRAINLLM.git' }, homepage: 'https://chrisifm.github.io/BRAINLLM/', publishConfig: { access: 'public', registry: 'https://registry.npmjs.org/' } }, null, 2) + '\n')
}

// npm renders the same marketing story; repository-relative resources become public URLs.
const npmReadme = readFileSync(join(root, 'README.md'), 'utf8')
  .replace(/(!?\[[^\]]*\]\()([^\s)]+)(\))/g, (all, start, path, end) => {
    if (/^(?:[a-z]+:|#|\/\/)/i.test(path)) return all
    return `${start}https://github.com/chrisifm/BRAINLLM/${start.startsWith('!') ? 'raw' : 'blob'}/main/${path}${end}`
  })
  .replace(/src="(?![a-z]+:|\/\/)([^"]+)"/gi, (_, path) => `src="https://raw.githubusercontent.com/chrisifm/BRAINLLM/main/${path}"`)
  .replace(/href="(?![a-z]+:|\/\/|#)([^"]+)"/gi, (_, path) => `href="https://github.com/chrisifm/BRAINLLM/blob/main/${path}"`)


// 1. platform packages
const sums = []
const built = []
const missing = []
for (const p of PLATFORMS) {
  const exe = 'brain' + (p.exe ?? '')
  const cands = binsAt ? [join(binsAt, p.key, exe)] : [p.triple === hostTriple && join(root, 'target/npm/release', exe), join(root, 'target', p.triple, 'release', exe)].filter(Boolean)
  const src = cands.find((c) => existsSync(c))
  if (!src) {
    missing.push(p.key)
    continue
  }
  const name = `${SCOPE}/brainllm-${p.key}`
  const dir = join(stage, `brainllm-${p.key}`)
  mkdirSync(join(dir, 'bin'), { recursive: true })
  copyFileSync(src, join(dir, 'bin', exe))
  chmodSync(join(dir, 'bin', exe), 0o755)
  const bytes = readFileSync(src)
  // a release binary must not carry this machine's paths (panic locations of dependencies under ~/.cargo)
  const home = process.env.HOME
  if (home && bytes.includes(Buffer.from(home + '/'))) {
    console.error(`${src} embeds paths under ${home}; fix: RUSTFLAGS="--remap-path-prefix=${home}=~" cargo build --release -p brain-cli --target ${p.triple}, then pack again`)
    process.exit(1)
  }
  sums.push(`${createHash('sha256').update(bytes).digest('hex')}  ${p.key}/${exe}`)
  writePkg(dir, {
    name,
    description: `The native Rust brain binary of BRAINLLM for ${p.key}. Installed automatically by ${SCOPE}/brainllm.`,
    os: [p.os],
    cpu: [p.cpu],
    ...(p.libc ? { libc: [p.libc] } : {}),
    files: ['bin'],
    preferUnplugged: true,
  })
  writeFileSync(join(dir, 'README.md'), `# ${name}\n\nThe native \`brain\` binary for ${p.key}, compiled from Rust. Do not install it directly: install \`${SCOPE}/brainllm\`.\n`)
  built.push({ name, dir, size: bytes.length })
}
if (missing.length && requireAll) {
  console.error(`missing binaries: ${missing.join(', ')}; fix: build them (CI matrix) or pass --bins <dir>`)
  process.exit(1)
}

// 2. launcher package
const main = join(stage, 'brainllm')
cpSync(join(here, 'brainllm'), main, { recursive: true })
copyFileSync(join(root, 'crates/brain-cli/schema/brain.schema.json'), join(main, 'brain.schema.json'))
writePkg(main, {
  name: `${SCOPE}/brainllm`,
  description: 'BRAINLLM: the web compiler for LLMs, built in Rust. One blazing-fast native binary: the `brain` CLI, dev server, build (~34 ms) and MCP server.',
  bin: { brain: 'bin/brain.js' },
  files: ['bin', 'brain.schema.json', 'README.md'],
  engines: { node: '>=22' },
  optionalDependencies: Object.fromEntries(PLATFORMS.map((p) => [`${SCOPE}/brainllm-${p.key}`, version])),
  keywords: ['framework', 'llm', 'ai', 'compiler', 'rust', 'fast', 'mcp', 'web'],
})

writeFileSync(join(main, 'README.md'), npmReadme)

// 3. create packages: canonical spelling and the requested brainll shorthand
const creators = []
for (const suffix of ['brainllm', 'brainll']) {
  const create = join(stage, `create-${suffix}`)
  cpSync(join(here, 'create-brainllm'), create, { recursive: true })
  writePkg(create, {
    name: `${SCOPE}/create-${suffix}`,
    description: `npm create ${SCOPE}/${suffix} <app>: a new BRAINLLM app (the Rust-built web compiler for LLMs) with its brain version pinned.`,
    bin: { [`create-${suffix}`]: 'bin/create.js' },
    files: ['bin', 'README.md'],
    engines: { node: '>=22' },
    dependencies: { [`${SCOPE}/brainllm`]: version },
  })
  writeFileSync(join(create, 'README.md'), `# ${SCOPE}/create-${suffix}\n\nThe BRAINLLM project initializer. Create an app:\n\n\`\`\`sh\nnpm create ${SCOPE}/${suffix}@latest my-app\ncd my-app\nnpm run dev\n\`\`\`\n\nRuns the same pinned BRAINLLM compiler and installs the generated app dependencies. Pass \`-- --no-install\` to skip dependency installation.\n\n` + npmReadme)
  creators.push(create)
}

// 4. runtime package: the ES modules without tests, demos or benchmarks
const rt = join(stage, 'brainllm-runtime')
mkdirSync(rt, { recursive: true })
for (const f of execFileSync('git', ['ls-files', '--cached', '--others', '--exclude-standard', 'runtime'], { cwd: root, encoding: 'utf8' }).split('\n')) {
  // tracked and new source files only: no tests, demos, benchmarks or local caches (any dot folder, e.g. ui/.brain/)
  if (!f || /^runtime\/(test|demo|perf)\//.test(f) || /(^|\/)\./.test(f.slice('runtime/'.length)) || f === 'runtime/package.json' || !existsSync(join(root, f))) continue
  const to = join(rt, f.slice('runtime/'.length))
  mkdirSync(dirname(to), { recursive: true })
  copyFileSync(join(root, f), to)
}
cpSync(join(here, 'brainllm-runtime', 'README.md'), join(rt, 'README.md'))
writePkg(rt, {
  name: `${SCOPE}/brainllm-runtime`,
  description: 'The BRAINLLM runtime for the Rust-built web compiler for LLMs: dependency-free ES modules (signals, DOM, router, data, forms, channels). The brain binary embeds this same version.',
  type: 'module',
  exports: { '.': './brain.js', './*': './*' },
  sideEffects: false,
})


// 5. npm pack (never publish)
const rows = []
for (const dir of [...built.map((b) => b.dir), main, ...creators, rt]) {
  const res = JSON.parse(execFileSync('npm', ['pack', '--json', '--pack-destination', out, ...(dry ? ['--dry-run'] : [])], { cwd: dir, encoding: 'utf8', stdio: ['ignore', 'pipe', 'inherit'] }))[0]
  rows.push(`${res.name.padEnd(42)} ${String(res.entryCount).padStart(4)} files  ${(res.size / 1024).toFixed(0).padStart(7)} KB  ${dry ? '(dry run)' : res.filename}`)
}
// checksums: every native binary and, after packing, every tarball (a downloaded .tgz can be verified)
if (!dry) for (const r of rows.map((l) => l.trim().split(/\s+/).pop())) sums.push(`${createHash('sha256').update(readFileSync(join(out, r))).digest('hex')}  ${r}`)
writeFileSync(join(out, 'SHA256SUMS'), sums.join('\n') + (sums.length ? '\n' : ''))
console.log(`BRAINLLM ${version} (${license})`)
console.log(rows.join('\n'))
console.log(missing.length ? `skipped (no binary here; CI builds them): ${missing.join(', ')}` : 'all 7 platforms packed')
console.log(`${dry ? 'dry run: staging files and checksums only; no tarballs or publication' : `tarballs + SHA256SUMS in ${out}`}; publish only with Chris's OK (npm/README.md)`)
