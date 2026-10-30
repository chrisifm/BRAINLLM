// Private server worker for @google-cloud/storage 8.4.0. ADC only; stdout is a bounded machine result.
import { constants } from 'node:fs'
import { open, realpath, lstat, unlink } from 'node:fs/promises'
import { join, sep } from 'node:path'
import { createHash } from 'node:crypto'
import { pipeline } from 'node:stream/promises'
import { Transform } from 'node:stream'
import { pathToFileURL } from 'node:url'
const MAX = 20 * 1024 * 1024
const fail = () => { throw Error('GCS_OPERATION_FAILED') }
const safe = key => typeof key === 'string' && key.length <= 512 && key.split('/').every(part => /^[a-zA-Z0-9_.-]+$/.test(part) && part !== '.' && part !== '..')
export async function validate(input) {
  if (input?.schemaVersion !== 1 || !/^[a-z0-9][a-z0-9._-]{1,61}[a-z0-9]$/.test(input.bucket || '') || !/^[a-zA-Z0-9_-]{1,80}$/.test(input.prefix || '') || !['write', 'read', 'delete'].includes(input.request?.op) || !Array.isArray(input.request.objects) || !input.request.objects.length || input.request.objects.length > 9 || typeof input.root !== 'string') fail()
  const root = await realpath(input.root)
  if ((await lstat(input.root)).isSymbolicLink()) fail()
  const paths = new Set()
  for (const object of input.request.objects) {
    if (!safe(object.key) || !object.key.startsWith(input.bucket + '/' + input.prefix + '/') || paths.has(object.key)) fail()
    paths.add(object.key)
    if (!object.key.split('/').some(part => /^[a-f0-9]{8}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{12}$/.test(part))) fail()
    if (input.request.op !== 'delete') {
      if (!Number.isSafeInteger(object.bytes) || object.bytes < 1 || object.bytes > MAX || !/^[a-f0-9]{64}$/.test(object.sha256 || '')) fail()
      if (input.request.op === 'write' && (!safe(object.source) || object.source !== object.key || !['image/png', 'image/jpeg', 'image/webp'].includes(object.contentType) || !object.key.endsWith('-' + object.sha256 + '.' + object.contentType.slice(6)))) fail()
      if (input.request.op === 'read' && (!object.generation || !/^gcs-read\/[a-f0-9]{32}\.bin$/.test(object.target || ''))) fail()
    }
    if (object.generation != null && (typeof object.generation !== 'string' || !/^[0-9]{0,32}$/.test(object.generation))) fail()
  }
  return root
}
async function localPath(root, key, existing) {
  if (!safe(key)) fail()
  let current = root
  for (const part of key.split('/')) {
    current = join(current, part)
    let entry
    try { entry = await lstat(current) } catch (error) { if (!existing && error.code === 'ENOENT') continue; fail() }
    if (entry.isSymbolicLink()) fail()
  }
  if (!current.startsWith(root + sep)) fail()
  return current
}
function checkedStream(expected) {
  let bytes = 0; const digest = createHash('sha256')
  return new Transform({ transform(chunk, encoding, callback) {
    bytes += chunk.length
    if (bytes > MAX || bytes > expected.bytes) return callback(Error('GCS_OPERATION_FAILED'))
    digest.update(chunk); callback(null, chunk)
  }, flush(callback) { callback(bytes === expected.bytes && digest.digest('hex') === expected.sha256 ? null : Error('GCS_OPERATION_FAILED')) } })
}
export async function execute(input, createStorage) {
  const root = await validate(input)
  const storage = await createStorage(), bucket = storage.bucket(input.bucket)
  const [policy] = await bucket.getMetadata()
  if (policy.iamConfiguration?.publicAccessPrevention !== 'enforced' || policy.iamConfiguration?.uniformBucketLevelAccess?.enabled !== true) fail()
  const output = []
  for (const object of input.request.objects) {
    const remoteKey = object.key.slice(input.bucket.length + 1), file = bucket.file(remoteKey)
    if (input.request.op === 'write') {
      const source = await localPath(root, object.source, true), sourceHandle = await open(source, constants.O_RDONLY | (constants.O_NOFOLLOW || 0))
      try {
        const stat = await sourceHandle.stat(); if (!stat.isFile() || stat.size !== object.bytes) fail()
        try {
          await pipeline(sourceHandle.createReadStream({ autoClose: true }), checkedStream(object), file.createWriteStream({ resumable: false, validation: 'crc32c', preconditionOpts: { ifGenerationMatch: 0 }, metadata: { contentType: object.contentType, cacheControl: 'private, no-store', metadata: { sha256: object.sha256 } } }))
        } catch (error) { if (![412, '412'].includes(error.code)) fail() }
      } finally { await sourceHandle.close().catch(() => {}) }
      const [metadata] = await file.getMetadata()
      if (String(metadata.size) !== String(object.bytes) || metadata.metadata?.sha256 !== object.sha256 || metadata.contentType !== object.contentType || !/^[0-9]{1,32}$/.test(metadata.generation || '')) fail()
      output.push({ key: object.key, generation: metadata.generation })
    } else if (input.request.op === 'read') {
      if (!object.generation) fail()
      const target = await localPath(root, object.target, false), handle = await open(target, constants.O_WRONLY | (constants.O_NOFOLLOW || 0))
      const stat = await handle.stat(); if (!stat.isFile() || stat.size !== 0) { await handle.close(); fail() }
      let completed = false
      try { await pipeline(bucket.file(remoteKey, { generation: object.generation }).createReadStream({ validation: 'crc32c' }), checkedStream(object), handle.createWriteStream({ autoClose: true, flush: true })); completed = true } finally { await handle.close().catch(() => {}); if (!completed) await unlink(target).catch(() => {}) }
      output.push({ key: object.key, generation: object.generation })
    } else {
      let generation = object.generation
      if (!generation) {
        let metadata
        try { [metadata] = await file.getMetadata() } catch (error) { if ([404, '404'].includes(error.code)) { output.push({ key: object.key, deleted: true }); continue } fail() }
        const expected = /-([a-f0-9]{64})\.(?:png|jpeg|webp)$/.exec(object.key)?.[1]
        if (!expected || metadata.metadata?.sha256 !== expected || !/^[0-9]{1,32}$/.test(metadata.generation || '')) fail()
        generation = metadata.generation
      }
      try { await file.delete({ ifGenerationMatch: generation, ignoreNotFound: true }) } catch (error) { if (![404, '404'].includes(error.code)) fail() }
      output.push({ key: object.key, deleted: true })
    }
  }
  return { ok: true, objects: output }
}
async function main() {
  const chunks = []; let bytes = 0
  for await (const chunk of process.stdin) { bytes += chunk.length; if (bytes > 32768) fail();chunks.push(chunk) }
  const input = JSON.parse(Buffer.concat(chunks).toString('utf8'))
  if (process.argv.includes('--validate')) { await validate(input); return { ok: true } }
  if (process.env.STORAGE_EMULATOR_HOST || process.env.GOOGLE_EXTERNAL_ACCOUNT_ALLOW_EXECUTABLES) fail()
  return execute(input, async () => { const { Storage } = await import('@google-cloud/storage'); return new Storage({ retryOptions: { autoRetry: false, maxRetries: 0 } }) })
}
if (import.meta.url === pathToFileURL(process.argv[1] || '').href) main().then(result => process.stdout.write(JSON.stringify(result)), () => { process.stdout.write('{"ok":false,"error":"GCS_OPERATION_FAILED"}'); process.exitCode = 1 })
