#!/usr/bin/env node
// Offline-first web-features ingestion. Dataset keys are evidence, not proof of an application's receiver type.
import { createHash, randomUUID } from 'node:crypto'
import { execFileSync } from 'node:child_process'
import { mkdirSync, mkdtempSync, readFileSync, writeFileSync, renameSync, rmSync, statSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'

export const PIN = Object.freeze({
  version: '3.44.0', published: '2026-10-22T16:30:26.001Z', license: 'Apache-2.0',
  tarball: 'https://registry.npmjs.org/web-features/-/web-features-3.44.0.tgz',
  integrity: 'sha512-K/+YJI7uVE/b4G8SU67/F/4j81/ajJ6MfBCZyuQaqyuPlVXF8cd6dLqBznXIWJI9nwnvuY2dapKK08OHzTjYww==',
  dataSha256: '72577e8df9c8abe8a8af86a03bc161ac06be21488d76857f157c7f83f017f706',
})
export const BROWSERS = ['chrome', 'chrome_android', 'edge', 'firefox', 'firefox_android', 'safari', 'safari_ios']
const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const OUTPUT = join(ROOT, 'crates/brain-check/src/baseline_data.rs')
const MAX_BYTES = 16 * 1024 * 1024
const RECEIVERS = {
  navigator: 'Navigator', document: 'Document', window: 'Window', crypto: 'Crypto', performance: 'Performance',
  scheduler: 'Scheduler', location: 'Location', history: 'History', localStorage: 'Storage', sessionStorage: 'Storage',
  caches: 'CacheStorage', indexedDB: 'IDBFactory', 'navigator.clipboard': 'Clipboard',
  'navigator.serviceWorker': 'ServiceWorkerContainer', 'navigator.storage': 'StorageManager',
  'navigator.permissions': 'Permissions', 'navigator.mediaDevices': 'MediaDevices', 'navigator.wakeLock': 'WakeLock',
  'crypto.subtle': 'SubtleCrypto', 'document.fonts': 'FontFaceSet',
}
// Explicit ECMAScript bindings: BCD keys omit prototype and normalize some legacy property names.
// Unknown members stay unclassified rather than being guessed from spelling or camelCase.
const STATIC = {
  Intl: 'Collator DateTimeFormat DisplayNames DurationFormat ListFormat Locale NumberFormat PluralRules RelativeTimeFormat Segmenter getCanonicalLocales supportedValuesOf',
  Array: 'from fromAsync isArray of', ArrayBuffer: 'isView',
  Object: 'assign create defineProperties defineProperty entries freeze fromEntries getOwnPropertyDescriptor getOwnPropertyDescriptors getOwnPropertyNames getOwnPropertySymbols getPrototypeOf groupBy hasOwn is isExtensible isFrozen isSealed keys preventExtensions seal setPrototypeOf values',
  Map: 'groupBy', Promise: 'all allSettled any race reject resolve try withResolvers',
  String: 'fromCharCode fromCodePoint raw', Number: 'isFinite isInteger isNaN isSafeInteger parseFloat parseInt EPSILON MAX_SAFE_INTEGER MAX_VALUE MIN_SAFE_INTEGER MIN_VALUE NEGATIVE_INFINITY NaN POSITIVE_INFINITY',
  BigInt: 'asIntN asUintN', Symbol: 'for keyFor asyncIterator hasInstance isConcatSpreadable iterator match matchAll replace search species split toPrimitive toStringTag unscopables',
  Date: 'now parse UTC', RegExp: 'escape input lastMatch lastParen leftContext rightContext', Error: 'isError',
  TypedArray: 'from of',
}
const INSTANCE = {
  Array: 'at concat copyWithin entries every fill filter find findIndex findLast findLastIndex flat flatMap forEach includes indexOf join keys lastIndexOf map pop push reduce reduceRight reverse shift slice some sort splice toLocaleString toReversed toSorted toSpliced toString unshift values with',
  ArrayBuffer: 'byteLength detached maxByteLength resizable resize slice transfer transferToFixedLength',
  SharedArrayBuffer: 'byteLength grow growable maxByteLength slice',
  DataView: 'buffer byteLength byteOffset getBigInt64 getBigUint64 getFloat16 getFloat32 getFloat64 getInt8 getInt16 getInt32 getUint8 getUint16 getUint32 setBigInt64 setBigUint64 setFloat16 setFloat32 setFloat64 setInt8 setInt16 setInt32 setUint8 setUint16 setUint32',
  Set: 'add clear delete difference entries forEach has intersection isDisjointFrom isSubsetOf isSupersetOf keys size symmetricDifference union values',
  Map: 'clear delete entries forEach get getOrInsert getOrInsertComputed has keys set size values',
  WeakMap: 'delete get getOrInsert getOrInsertComputed has set', WeakSet: 'add delete has',
  Promise: 'catch finally then', Error: 'cause message name stack toString',
  Object: 'defineGetter defineSetter hasOwnProperty isPrototypeOf lookupGetter lookupSetter propertyIsEnumerable proto toLocaleString toString valueOf',
  String: 'anchor at big blink bold charAt charCodeAt codePointAt concat endsWith fixed fontcolor fontsize includes indexOf isWellFormed italics lastIndexOf length link localeCompare match matchAll normalize padEnd padStart repeat replace replaceAll search slice small split startsWith strike sub substr substring sup toLocaleLowerCase toLocaleUpperCase toLowerCase toString toUpperCase toWellFormed trim trimEnd trimLeft trimRight trimStart valueOf',
  Number: 'toExponential toFixed toLocaleString toPrecision toString valueOf',
  Boolean: 'toString valueOf', BigInt: 'toLocaleString toString valueOf', Symbol: 'description toString valueOf',
  Date: 'getDate getDay getFullYear getHours getMilliseconds getMinutes getMonth getSeconds getTime getTimezoneOffset getUTCDate getUTCDay getUTCFullYear getUTCHours getUTCMilliseconds getUTCMinutes getUTCMonth getUTCSeconds getYear setDate setFullYear setHours setMilliseconds setMinutes setMonth setSeconds setTime setUTCDate setUTCFullYear setUTCHours setUTCMilliseconds setUTCMinutes setUTCMonth setUTCSeconds setYear toDateString toGMTString toISOString toJSON toLocaleDateString toLocaleString toLocaleTimeString toString toTemporalInstant toTimeString toUTCString valueOf',
  RegExp: 'compile dotAll exec flags global hasIndices ignoreCase lastIndex multiline source sticky test toString unicode unicodeSets',
  Function: 'apply bind call length name toString',
  TypedArray: 'at buffer byteLength byteOffset copyWithin entries every fill filter find findIndex findLast findLastIndex forEach includes indexOf join keys lastIndexOf length map reduce reduceRight reverse set slice some sort subarray toLocaleString toReversed toSorted toString values with',
}
const ALIASES = { 'Object.defineGetter': '__defineGetter__', 'Object.defineSetter': '__defineSetter__', 'Object.lookupGetter': '__lookupGetter__', 'Object.lookupSetter': '__lookupSetter__', 'Object.proto': '__proto__' }
const includes = (table, owner, member) => (table[owner] || '').split(' ').includes(member)
const sha256 = value => createHash('sha256').update(value).digest('hex')
const identifier = value => /^[A-Za-z_$][\w$]*$/.test(value)
const ordered = map => [...map.entries()].sort(([a], [b]) => a < b ? -1 : a > b ? 1 : 0)

export function status(s) {
  if (!s || !['high', 'low', false].includes(s.baseline) || !s.support || typeof s.support !== 'object') throw Error('invalid web-features status')
  const date = s.baseline_low_date || null
  if (date !== null && !/^≤?\d{4}-\d{2}-\d{2}$/.test(date)) throw Error('invalid Baseline date')
  if (s.baseline !== false && date === null) throw Error('Baseline low/high requires its availability date')
  let mask = 0
  BROWSERS.forEach((browser, bit) => { if (typeof s.support[browser] === 'string' && s.support[browser]) mask |= 1 << bit })
  return { year: s.baseline === false ? 0 : Number(date.replace(/^≤/, '').slice(0, 4)), widely: s.baseline === 'high', mask, lowDate: date, highDate: s.baseline_high_date || null }
}
function mergeStatus(a, b) {
  // Collisions retain the least-supported declaration, never the most optimistic alias.
  return { year: a.year === 0 || b.year === 0 ? 0 : Math.max(a.year, b.year), widely: a.widely && b.widely, mask: a.mask & b.mask, lowDate: a.lowDate && b.lowDate ? [a.lowDate, b.lowDate].sort((x,y) => x.replace(/^≤/, '').localeCompare(y.replace(/^≤/, ''))).at(-1) : null, highDate: a.highDate && b.highDate ? [a.highDate, b.highDate].sort((x,y) => x.replace(/^≤/, '').localeCompare(y.replace(/^≤/, ''))).at(-1) : null }
}
export function generate(data, globals = new Set()) {
  if (!data || !data.features || typeof data.features !== 'object' || Array.isArray(data.features)) throw Error('web-features.features must be an object')
  const tables = Object.fromEntries(['paths', 'members', 'props', 'selectors', 'atRules', 'elements', 'attrs', 'raw', 'unclassified'].map(name => [name, new Map()]))
  const features = Object.keys(data.features).filter(id => data.features[id]?.kind === 'feature').sort()
  if (features.length > 10000) throw Error('dataset exceeds 10000 features')
  let keyCount = 0
  const put = (table, name, st, id, extra = '') => {
    const prev = table.get(name)
    if (prev) { prev.status = mergeStatus(prev.status, st); prev.features = [...new Set([...prev.features, id])].sort() }
    else table.set(name, { status: st, features: [id], extra })
  }
  for (const id of features) {
    const feature = data.features[id]
    status(feature.status)
    const overrides = feature.status.by_compat_key || {}
    const keys = [...new Set([...(feature.compat_features || []), ...Object.keys(overrides)])].sort()
    for (const key of keys) {
      if (++keyCount > 50000 || typeof key !== 'string' || key.length > 300 || !/^[\w.@$:-]+$/.test(key)) throw Error('invalid or oversized compatibility key set')
      const st = status(overrides[key] || feature.status)
      put(tables.raw, key, st, id)
      const parts = key.split('.'), [top, sub, owner, raw] = parts
      if (top === 'javascript' && sub === 'builtins') {
        if (parts.length === 3 && globals.has(owner)) put(tables.paths, owner, st, id, 'global')
        else if (parts.length === 4 && identifier(raw)) {
          const member = ALIASES[`${owner}.${raw}`] || raw
          if (includes(STATIC, owner, raw) && globals.has(owner)) put(tables.paths, `${owner}.${member}`, st, id, 'static')
          else if (includes(INSTANCE, owner, raw)) {
            put(tables.members, `${owner}.${member}`, st, id, owner)
            if (globals.has(owner)) put(tables.paths, `${owner}.prototype.${member}`, st, id, 'prototype')
          } else if (['Math', 'JSON', 'Atomics', 'Reflect'].includes(owner) && globals.has(owner)) put(tables.paths, `${owner}.${member}`, st, id, 'namespace')
          else put(tables.unclassified, key, st, id, 'unclassified JavaScript binding; receiver or alias requires explicit mapping')
        } else put(tables.unclassified, key, st, id, 'unclassified JavaScript subfeature; raw compatibility evidence only')
      } else if (top === 'api' && parts.length === 3) {
        const iface = sub, member = parts[2]
        if (member.endsWith('_static') && identifier(member.slice(0, -7)) && globals.has(iface)) put(tables.paths, `${iface}.${member.slice(0, -7)}`, st, id, 'static')
        else if (member !== iface && identifier(member) && !member.includes('_')) {
          put(tables.members, `${iface}.${member}`, st, id, iface)
          for (const [binding, type] of Object.entries(RECEIVERS)) if (type === iface) put(tables.paths, `${binding}.${member}`, st, id, `receiver:${iface}`)
          if (iface === 'Window' && globals.has(member)) put(tables.paths, member, st, id, 'global')
        } else if (member === iface && globals.has(iface)) put(tables.paths, iface, st, id, 'constructor')
        else put(tables.unclassified, key, st, id, 'API event, alias or subfeature; raw compatibility evidence only')
      } else if (top === 'css' && parts.length === 3 && sub === 'properties' && !owner.startsWith('-')) put(tables.props, owner, st, id)
      else if (top === 'css' && parts.length === 3 && sub === 'selectors' && /^[a-z][a-z-]*$/.test(owner)) put(tables.selectors, owner, st, id)
      else if (top === 'css' && parts.length === 3 && sub === 'at-rules' && /^[a-z][a-z-]*$/.test(owner)) put(tables.atRules, owner, st, id)
      else if (top === 'html' && sub === 'elements' && parts.length === 3) put(tables.elements, owner, st, id)
      else if (top === 'html' && sub === 'elements' && parts.length === 4 && owner === 'input' && raw.startsWith('type_')) put(tables.attrs, `input[type=${raw.slice(5)}]`, st, id)
      else if (top === 'html' && sub === 'elements' && parts.length === 4 && /^[a-z][a-z-]*$/.test(raw)) put(tables.attrs, `${owner}.${raw}`, st, id)
      else if (top === 'html' && sub === 'global_attributes' && parts.length === 3 && /^[a-z][a-z-]*$/.test(owner)) put(tables.attrs, `*.${owner}`, st, id)
      else put(tables.unclassified, key, st, id, 'raw evidence only; syntax/value matching is not implemented')
    }
    if (!keys.length) put(tables.unclassified, `feature:${id}`, status(feature.status), id, 'upstream provides no compatibility-key mapping')
  }
  return { features, tables }
}

export function render(data, globals, provenance) {
  const { features, tables } = generate(data, globals), ids = new Map(features.map((id, index) => [id, index]))
  const q = s => JSON.stringify(s).replace(/\\u([0-9a-fA-F]{4})/g, '\\u{$1}')
  const table = (name, records) => `pub const ${name}: &[(&str, u16, bool, u8, &str, &str, &str, &str)] = &[\n${ordered(records).map(([key, row]) => {
    const s = row.status
    return `    (${q(key)}, ${s.year}, ${s.widely}, ${s.mask}, ${q(row.features.map(id => ids.get(id)).join(','))}, ${q(row.extra)}, ${q(s.lowDate || '')}, ${q(s.highDate || '')}),`
  }).join('\n')}\n];\n`
  const surface = [['PATHS','paths'],['MEMBERS','members'],['CSS_PROPERTIES','props'],['CSS_SELECTORS','selectors'],['CSS_AT_RULES','atRules'],['HTML_ELEMENTS','elements'],['HTML_ATTRIBUTES','attrs'],['COMPAT_KEYS','raw'],['UNCLASSIFIED','unclassified']]
  return `//! GENERATED by scripts/gen-baseline.mjs. No network or application code runs in builds.\n//! Source: web-features ${provenance.version}, Apache-2.0; https://github.com/web-platform-dx/web-features.\n//! Tuple: key, Baseline year (0=limited), widely, browser mask, comma-separated feature indexes, match kind/owner, low date, high date.\n//! MEMBERS require a proven receiver type. Raw keys and UNCLASSIFIED are evidence, not source-code matchers.\n//! This table implements neither diagnostics, lowering nor polyfills.\n\npub const SOURCE: &str = ${q(`web-features ${provenance.version}`)};\npub const PUBLISHED: &str = ${q(provenance.published)};\npub const DATA_SHA256: &str = ${q(provenance.dataSha256)};\npub const ARCHIVE_INTEGRITY: &str = ${q(provenance.integrity)};\npub const GLOBALS_SHA256: &str = ${q(sha256([...globals].sort().join('\n')))};\npub const BROWSERS: &[&str] = &[${BROWSERS.map(q).join(', ')}];\npub const FEATURES: &[&str] = &[${features.map(q).join(', ')}];\n\n${surface.map(([name,key]) => table(name,tables[key])).join('\n')}`
}
function knownGlobals() {
  const text = readFileSync(join(ROOT, 'crates/brain-check/src/scope.rs'), 'utf8'), start = text.indexOf('pub const KNOWN_GLOBALS')
  if (start < 0) throw Error('KNOWN_GLOBALS was not found; update its explicit extractor')
  const end = text.indexOf('];', start)
  if (end < 0) throw Error('KNOWN_GLOBALS has no closing delimiter')
  return new Set([...text.slice(start, end).matchAll(/"([A-Za-z_$][\w$]*)"/g)].map(m => m[1]))
}
async function download(url, maxBytes, signal) {
  const response = await fetch(url, { signal })
  if (!response.ok || Number(response.headers.get('content-length')) > maxBytes) throw Error('dataset download failed or exceeds its byte limit')
  const reader = response.body.getReader(), chunks = []
  let total = 0
  try {
    for (;;) {
      const { done, value } = await reader.read()
      if (done) break
      total += value.byteLength
      if (total > maxBytes) { await reader.cancel(); throw Error('dataset download exceeds its byte limit') }
      chunks.push(value)
    }
    return Buffer.concat(chunks)
  } finally { reader.releaseLock() }
}
export async function fetchPinned() {
  const deadline = Date.now() + 30000
  const controller = new AbortController(), timer = setTimeout(() => controller.abort(), 30000)
  const dir = mkdtempSync(join(tmpdir(), 'brain-web-features-'))
  try {
    const bytes = await download(PIN.tarball, 8 * 1024 * 1024, controller.signal)
    if (`sha512-${createHash('sha512').update(bytes).digest('base64')}` !== PIN.integrity) throw Error('web-features archive integrity mismatch')
    const archive = join(dir, 'source.tgz'); writeFileSync(archive, bytes, { mode: 0o600 })
    const extract = name => {
      const remaining = deadline - Date.now()
      if (remaining <= 0) throw Error('dataset download/extraction exceeded its total deadline')
      return execFileSync('tar', ['-xzOf', archive, `package/${name}`], { timeout: remaining, maxBuffer: MAX_BYTES, stdio: ['ignore', 'pipe', 'pipe'] })
    }
    const pkg = JSON.parse(extract('package.json'))
    if (pkg.version !== PIN.version || pkg.license !== PIN.license) throw Error('package metadata differs from pinned provenance')
    return extract('data.json')
  } finally { clearTimeout(timer); rmSync(dir, { recursive: true, force: true }) }
}
export function verifyPinned(bytes) {
  if (bytes.length > MAX_BYTES || sha256(bytes) !== PIN.dataSha256) throw Error('dataset SHA-256 differs from the pinned official package')
  return JSON.parse(bytes.toString('utf8'))
}
export async function main(args = process.argv.slice(2)) {
  const allowed = new Set(['--fetch', '--data', '--check'])
  let dataPath
  for (let i = 0; i < args.length; i++) {
    if (!allowed.has(args[i])) throw Error(`unknown option ${args[i]}`)
    if (args[i] === '--data') { dataPath = args[++i]; if (!dataPath || dataPath.startsWith('--')) throw Error('--data needs an extracted official data.json path') }
  }
  if (Boolean(dataPath) === args.includes('--fetch')) throw Error('choose exactly one input: --data <official data.json> or --fetch')
  if (dataPath && statSync(dataPath).size > MAX_BYTES) throw Error('dataset input exceeds 16 MiB')
  const bytes = dataPath ? readFileSync(dataPath) : await fetchPinned()
  const output = render(verifyPinned(bytes), knownGlobals(), PIN)
  if (args.includes('--check')) {
    if (readFileSync(OUTPUT, 'utf8') !== output) throw Error('baseline table is stale; regenerate from the pinned official dataset')
  } else {
    mkdirSync(dirname(OUTPUT), { recursive: true })
    const temp = `${OUTPUT}.${process.pid}.${randomUUID()}.tmp`
    try { writeFileSync(temp, output, { flag: 'wx' }); renameSync(temp, OUTPUT) } finally { rmSync(temp, { force: true }) }
  }
  console.log(args.includes('--check') ? 'baseline table matches pinned provenance' : `generated pinned baseline table (${Buffer.byteLength(output)} bytes)`)
}
if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) main().catch(error => { console.error(error.message); process.exitCode = 1 })
