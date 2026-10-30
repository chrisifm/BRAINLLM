#!/usr/bin/env node
// Explicit contract snapshots and semver checks; never mutates framework versions or gates.
import { readFileSync, readdirSync, statSync, writeFileSync } from 'node:fs'
import { resolve, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const surfaces = ['commands', 'diagnostics', 'primitives', 'outputs']
const maxBytes = 4 * 1024 * 1024
const plain = value => value !== null && typeof value === 'object' && !Array.isArray(value)
const stable = value => JSON.stringify(sort(value))
function sort(value) {
  if (Array.isArray(value)) return value.map(sort)
  if (plain(value)) return Object.fromEntries(Object.keys(value).sort().map(key => [key, sort(value[key])]))
  return value
}
function boundedFile(path) {
  if (statSync(path).size > maxBytes) throw new Error(`contract input exceeds 4 MiB: ${path}`)
  return readFileSync(path, 'utf8')
}
function version(value) {
  const match = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-((?:0|[1-9]\d*|\d*[A-Za-z-][0-9A-Za-z-]*)(?:\.(?:0|[1-9]\d*|\d*[A-Za-z-][0-9A-Za-z-]*))*))?(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$/.exec(value)
  if (!match || match.slice(1, 4).some(n => !Number.isSafeInteger(Number(n)))) throw new Error('frameworkVersion must be an explicit valid semantic version')
  return { major: Number(match[1]), minor: Number(match[2]), patch: Number(match[3]), prerelease: match[4]?.split('.') || [] }
}
function compareVersion(a, b) {
  for (const key of ['major', 'minor', 'patch']) if (a[key] !== b[key]) return Math.sign(a[key] - b[key])
  if (!a.prerelease.length || !b.prerelease.length) return a.prerelease.length ? -1 : b.prerelease.length ? 1 : 0
  for (let i = 0; i < Math.max(a.prerelease.length, b.prerelease.length); i++) {
    if (a.prerelease[i] === undefined) return -1
    if (b.prerelease[i] === undefined) return 1
    const x = a.prerelease[i], y = b.prerelease[i]
    if (x === y) continue
    const xn = /^\d+$/.test(x), yn = /^\d+$/.test(y)
    if (xn && yn) return x.length === y.length ? Math.sign(x.localeCompare(y)) : Math.sign(x.length - y.length)
    if (xn !== yn) return xn ? -1 : 1
    return x < y ? -1 : 1
  }
  return 0
}
function validateTree(value, depth = 0) {
  if (depth > 32) throw new Error('contract nesting exceeds 32 levels')
  if (plain(value)) for (const [key, child] of Object.entries(value)) {
    if (['__proto__', 'constructor', 'prototype'].includes(key)) throw new Error('unsafe contract key')
    validateTree(child, depth + 1)
  }
  else if (Array.isArray(value)) value.forEach(child => validateTree(child, depth + 1))
}
export function validateManifest(manifest) {
  if (!plain(manifest) || manifest.manifestVersion !== 1) throw new Error('unsupported contract manifestVersion (expected 1)')
  if (typeof manifest.frameworkVersion !== 'string') throw new Error('frameworkVersion is required')
  version(manifest.frameworkVersion)
  if (!plain(manifest.surfaces)) throw new Error('surfaces must be an object')
  let count = 0
  for (const name of surfaces) {
    if (!plain(manifest.surfaces[name])) throw new Error(`surfaces.${name} must be an explicit object`)
    for (const [id, entry] of Object.entries(manifest.surfaces[name])) {
      if (!id || id.length > 200 || !plain(entry)) throw new Error(`invalid ${name} entry`)
      if (++count > 10000) throw new Error('manifest exceeds 10000 contract entries')
      for (const key of ['signature', 'replacement', 'mechanicalFix', 'renamedFrom']) if (entry[key] !== undefined && typeof entry[key] !== 'string') throw new Error(`${name}.${id}.${key} must be a string`)
      if (entry.fields !== undefined && (!Array.isArray(entry.fields) || entry.fields.some(f => typeof f !== 'string'))) throw new Error(`${name}.${id}.fields must be string names`)
      if (entry.deprecated !== undefined && typeof entry.deprecated !== 'boolean') throw new Error('deprecated must be boolean')
      for (const key of ['inputSchema', 'outputSchema']) if (entry[key] !== undefined && !plain(entry[key])) throw new Error(`${key} must be an object`)
    }
  }
  if (manifest.security !== undefined && !plain(manifest.security)) throw new Error('security must be an object of enabled invariant flags')
  for (const enabled of Object.values(manifest.security || {})) if (typeof enabled !== 'boolean') throw new Error('security invariants must be boolean flags')
  for (const name of surfaces) for (const [id, entry] of Object.entries(manifest.surfaces[name])) {
    for (const key of ['inputSchema', 'outputSchema']) if (entry[key]) validateSchema(entry[key], `${name}.${id}.${key}`)
  }
  validateTree(manifest)
  return manifest
}
function validateSchema(schema, path, depth = 0) {
  if (!plain(schema) || depth > 24) throw new Error(`invalid or too deeply nested schema: ${path}`)
  if (schema.properties !== undefined && !plain(schema.properties)) throw new Error(`properties must be an object: ${path}`)
  if (schema.required !== undefined && (!Array.isArray(schema.required) || schema.required.some(v => typeof v !== 'string'))) throw new Error(`required must be an array of names: ${path}`)
  if (schema.enum !== undefined && !Array.isArray(schema.enum)) throw new Error(`enum must be an array: ${path}`)
  for (const [name, child] of Object.entries(schema.properties || {})) validateSchema(child, `${path}.${name}`, depth + 1)
  if (schema.items !== undefined) validateSchema(schema.items, `${path}.items`, depth + 1)
}
export function readManifest(path) { return validateManifest(JSON.parse(boundedFile(path))) }

// A conservative directional JSON Schema subset. Unknown changed keywords are
// breaking instead of silently accepting constraints this tool cannot prove.
function schemaChanges(before, after, direction, path, emit) {
  if (stable(before) === stable(after)) return
  const ignored = new Set(['description', 'title', '$comment', 'examples', 'default', 'deprecated'])
  for (const key of new Set([...Object.keys(before), ...Object.keys(after)])) {
    if (ignored.has(key)) continue
    const a = before[key], b = after[key]
    if (stable(a) === stable(b)) continue
    if (key === 'properties' && (a === undefined || plain(a)) && (b === undefined || plain(b))) {
      for (const name of new Set([...Object.keys(a || {}), ...Object.keys(b || {})])) {
        const at = `${path}.properties.${name}`
        if (!(name in (b || {}))) emit('breaking', at, 'property removed')
        else if (!(name in (a || {}))) {
          const inputClosed = before.additionalProperties === false
          const required = (after.required || []).includes(name)
          emit(direction === 'input' && required || direction === 'output' && inputClosed ? 'breaking' : 'additive', at, 'property added')
        } else schemaChanges(a[name], b[name], direction, at, emit)
      }
    } else if (key === 'required' && [a, b].every(v => v === undefined || Array.isArray(v))) {
      const removed = (a || []).filter(v => !(b || []).includes(v)), added = (b || []).filter(v => !(a || []).includes(v))
      if (direction === 'input' ? added.length : removed.length) emit('breaking', `${path}.required`, direction === 'input' ? 'new required input' : 'output no longer guaranteed')
      else emit('additive', `${path}.required`, 'requirement widened compatibly')
    } else if (key === 'enum' && [a, b].every(v => v === undefined || Array.isArray(v))) {
      const subset = (small, big) => big === undefined || small !== undefined && small.every(v => big.some(w => stable(w) === stable(v)))
      const compatible = direction === 'input' ? subset(a, b) : subset(b, a)
      emit(compatible ? 'additive' : 'breaking', `${path}.enum`, direction === 'input' ? 'accepted values changed' : 'possible output values changed')
    } else if (key === 'items' && plain(a) && plain(b)) schemaChanges(a, b, direction, `${path}.items`, emit)
    else emit('breaking', `${path}.${key}`, 'schema constraint changed; compatibility requires explicit review')
  }
}

export function compareManifests(baseline, current) {
  validateManifest(baseline); validateManifest(current)
  const changes = [], errors = [], warnings = []
  const add = (kind, path, message, migration) => changes.push({ kind, path, message, ...(migration ? { migration } : {}) })
  for (const surface of surfaces) {
    const old = baseline.surfaces[surface], next = current.surfaces[surface]
    for (const id of new Set([...Object.keys(old), ...Object.keys(next)])) {
      const path = `${surface}.${id}`, a = old[id], b = next[id]
      if (!b) {
        const renamed = Object.entries(next).find(([, value]) => value.renamedFrom === id)
        add('breaking', path, renamed ? `renamed to ${renamed[0]} without preserving the old contract` : 'contract removed', a.mechanicalFix || a.replacement)
        continue
      }
      if (!a) { add('additive', path, 'contract added'); continue }
      const migration = b.mechanicalFix || b.replacement
      if (a.signature !== b.signature) add('breaking', `${path}.signature`, 'calling syntax changed', migration)
      for (const key of ['inputSchema', 'outputSchema']) {
        if (a[key] !== undefined || b[key] !== undefined) {
          if (!plain(a[key]) || !plain(b[key])) add('breaking', `${path}.${key}`, 'schema contract added or removed; review the explicit baseline', migration)
          else schemaChanges(a[key], b[key], key === 'inputSchema' ? 'input' : 'output', `${path}.${key}`, (kind, at, message) => add(kind, at, message, migration))
        }
      }
      if (a.fields !== undefined || b.fields !== undefined) {
        for (const field of a.fields || []) if (!(b.fields || []).includes(field)) add('breaking', `${path}.fields.${field}`, 'output field removed', migration)
        for (const field of b.fields || []) if (!(a.fields || []).includes(field)) add('additive', `${path}.fields.${field}`, 'output field added')
      }
      if (a.schemaVersion !== b.schemaVersion) add('breaking', `${path}.schemaVersion`, 'wire format version changed', migration)
      if (a.severity !== b.severity) add('breaking', `${path}.severity`, 'diagnostic severity changed', migration)
      if (b.deprecated && !a.deprecated) add('deprecation', path, 'contract deprecated; keep its existing behavior during migration', migration)
      const known = new Set(['signature', 'inputSchema', 'outputSchema', 'fields', 'schemaVersion', 'severity', 'deprecated', 'replacement', 'mechanicalFix', 'renamedFrom', 'description', 'evidence'])
      for (const key of new Set([...Object.keys(a), ...Object.keys(b)])) if (!known.has(key) && stable(a[key]) !== stable(b[key])) add('breaking', `${path}.${key}`, 'explicit contract value changed', migration)
    }
  }
  for (const [name, enabled] of Object.entries(baseline.security || {})) if (enabled && current.security?.[name] !== true) {
    add('security', `security.${name}`, 'enabled security invariant removed or disabled')
    errors.push(`security.${name}: safety invariants cannot be weakened by a version bump`)
  }
  const old = version(baseline.frameworkVersion), next = version(current.frameworkVersion)
  if (compareVersion(next, old) < 0) errors.push('frameworkVersion must not decrease')
  const majorBump = next.major > old.major
  const minorBump = majorBump || next.major === old.major && next.minor > old.minor
  const breaking = changes.some(c => c.kind === 'breaking')
  const additive = changes.some(c => c.kind === 'additive' || c.kind === 'deprecation')
  const requires = breaking ? old.major === 0 ? 'minor (pre-1.0)' : 'major' : additive ? 'minor' : 'none'
  if (breaking && !(old.major === 0 ? minorBump : majorBump)) errors.push(`breaking contracts require a ${requires} version increase`)
  if (!breaking && additive && !minorBump) errors.push('additions and deprecations require a minor version increase')
  for (const change of changes) {
    if (change.kind === 'breaking') warnings.push(`${change.path}: provide an upgrade note${change.migration ? `; suggested mechanical fix: ${change.migration}` : ' and a checked mechanical fix where possible'}`)
    if (change.kind === 'deprecation' || change.kind === 'additive') warnings.push(`${change.path}: ${change.message}; old supported apps must keep working`)
  }
  const coverageLimitations = [...new Set([...(baseline.provenance?.limitations || []), ...(current.provenance?.limitations || [])])]
  if (coverageLimitations.length) warnings.push('Source inventory is partial evidence; compatibility applies only to explicitly captured contract surfaces')
  return { manifestVersion: 1, scope: 'declared contract surfaces only', coverageLimitations, baselineVersion: baseline.frameworkVersion, currentVersion: current.frameworkVersion, compatible: !breaking && !changes.some(c => c.kind === 'security'), policyPassed: errors.length === 0, requiredBump: requires, changes, warnings, errors }
}

export function snapshotRepository(root) {
  root = resolve(root)
  const cargo = boundedFile(join(root, 'Cargo.toml'))
  const frameworkVersion = /^\[workspace\.package\][\s\S]*?^version\s*=\s*"([^"]+)"/m.exec(cargo)?.[1]
  if (!frameworkVersion) throw new Error('cannot determine explicit workspace package version')
  const main = boundedFile(join(root, 'crates/brain-cli/src/main.rs'))
  const usage = /const USAGE:\s*&str\s*=\s*"([\s\S]*?)"\s*;/.exec(main)?.[1]
  if (!usage) throw new Error('cannot find CLI USAGE contract')
  const commands = Object.create(null)
  for (const line of usage.split('\n')) {
    const match = /^\s*brain\s+([a-z][a-z-]*|--version)(.*)$/.exec(line)
    if (!match) continue
    const suffix = match[2].split(/\s{2,}/)[0].trim().replace(/\\"/g, '"')
    for (const name of [match[1], ...[...suffix.matchAll(/\|\s*([a-z][a-z-]*)/g)].map(m => m[1])].filter(n => ['stop', 'reload', 'status'].includes(match[1]) || n === match[1])) {
      commands[name] = { signature: `brain ${name}${suffix && !suffix.startsWith('|') ? ` ${suffix}` : ''}` }
    }
  }
  const scope = boundedFile(join(root, 'crates/brain-check/src/scope.rs'))
  const primitiveArray = /pub const PRIMITIVES:[\s\S]*?=\s*&\[([\s\S]*?)\];/.exec(scope)?.[1]
  if (!primitiveArray) throw new Error('cannot find primitive registry')
  const primitives = Object.fromEntries([...primitiveArray.matchAll(/"([A-Za-z_$][\w$]*)"/g)].map(m => [m[1], {}]))
  const diagnostics = Object.create(null)
  let files = 0, bytes = 0
  function scan(dir, depth = 0) {
    if (depth > 8) throw new Error('diagnostic source scan exceeds 8 levels')
    for (const entry of readdirSync(dir, { withFileTypes: true }).sort((a, b) => a.name.localeCompare(b.name))) {
      if (entry.isSymbolicLink()) throw new Error('diagnostic source scan refuses symlinks')
      if (['target', 'node_modules', 'tests'].includes(entry.name)) continue
      const path = join(dir, entry.name)
      if (entry.isDirectory()) scan(path, depth + 1)
      else if (entry.name.endsWith('.rs')) {
        if (++files > 2048) throw new Error('diagnostic source scan exceeds 2048 files')
        const source = boundedFile(path); bytes += Buffer.byteLength(source)
        if (bytes > 32 * 1024 * 1024) throw new Error('diagnostic source scan exceeds 32 MiB')
        for (const m of source.matchAll(/"(B\d{4})"/g)) diagnostics[m[1]] = {}
      }
    }
  }
  scan(join(root, 'crates'))
  const diag = boundedFile(join(root, 'crates/brain-lex/src/diag.rs'))
  const serializer = /pub fn to_json\([\s\S]*?(?=\n    pub fn|\n    \/\/\/)/.exec(diag)?.[0]
  if (!serializer) throw new Error('cannot find diagnostic JSON serializer')
  const fields = [...new Set([...serializer.matchAll(/\\"([A-Za-z]\w*)\\"\s*:/g)].map(m => m[1]))].sort()
  if (!fields.length) throw new Error('cannot extract diagnostic output fields')
  const outputs = { diagnostics: { fields }, check: { signature: 'JSON array of diagnostic objects' } }
  return validateManifest({ manifestVersion: 1, frameworkVersion, provenance: { kind: 'source-inventory', baseline: false, coverage: ['CLI help syntax (top-level commands)', 'literal diagnostic IDs in Rust source (including inline tests)', 'registered author primitive names', 'diagnostic serializer field names, including edit fields'], limitations: ['Not a released baseline', 'Does not infer runtime primitive arity or behavior', 'Does not prove diagnostic severity or complete command output schemas', 'No security audit inferred; explicit security invariants require authored manifests'] }, surfaces: { commands, diagnostics, primitives, outputs }, security: {} })
}

function main(argv) {
  const action = argv.shift(), options = Object.create(null)
  if (!['snapshot', 'compare'].includes(action)) throw new Error('usage: compatibility-check.mjs snapshot --output PATH [--root PATH] | compare --baseline PATH [--current PATH | --root PATH] [--json]')
  while (argv.length) {
    const flag = argv.shift()
    if (flag === '--json') { options.json = true; continue }
    if (!['--root', '--output', '--baseline', '--current'].includes(flag) || !argv.length || argv[0].startsWith('--') || options[flag]) throw new Error(`invalid or duplicate option: ${flag}`)
    options[flag] = argv.shift()
  }
  const root = options['--root'] || resolve(fileURLToPath(new URL('..', import.meta.url)))
  if (action === 'snapshot') {
    if (!options['--output'] || options['--baseline'] || options['--current']) throw new Error('snapshot requires --output and never creates an implicit baseline')
    const manifest = snapshotRepository(root)
    writeFileSync(resolve(options['--output']), `${JSON.stringify(sort(manifest), null, 2)}\n`, { flag: 'wx', mode: 0o600 })
    console.log(`Source inventory saved to ${resolve(options['--output'])}; this is not a released baseline.`)
    return 0
  }
  if (!options['--baseline'] || options['--output'] || options['--current'] && options['--root']) throw new Error('compare requires an explicit --baseline and at most one of --current/--root')
  const result = compareManifests(readManifest(options['--baseline']), options['--current'] ? readManifest(options['--current']) : snapshotRepository(root))
  if (options.json) console.log(JSON.stringify(result, null, 2))
  else {
    console.log(`Compatibility ${result.baselineVersion} -> ${result.currentVersion}: ${result.policyPassed ? 'policy passed' : 'policy failed'}; required bump: ${result.requiredBump}`)
    for (const change of result.changes) console.log(`${change.kind}: ${change.path}: ${change.message}`)
    for (const warning of result.warnings) console.log(`warning: ${warning}`)
    for (const error of result.errors) console.error(`error: ${error}`)
  }
  return result.policyPassed ? 0 : 1
}
if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try { process.exitCode = main(process.argv.slice(2)) }
  catch (error) { console.error(`compatibility-check: ${error.message}`); process.exitCode = 2 }
}
