// Web Crypto owns cryptography in browsers and Node. No hashing implementation is bundled.
export async function hashText(value, algorithm = 'SHA-256') {
  if (!['SHA-256', 'SHA-384', 'SHA-512'].includes(algorithm)) throw RangeError('hashText: SHA-256, SHA-384 or SHA-512 required')
  if (typeof value !== 'string') throw TypeError('hashText: text input required')
  if (value.length > 8388608) throw RangeError('hashText: input exceeds 8 MiB')
  const bytes = new TextEncoder().encode(value)
  if (bytes.byteLength > 8388608) throw RangeError('hashText: UTF-8 input exceeds 8 MiB')
  if (!globalThis.crypto?.subtle) throw Error('hashText: Web Crypto requires a secure browser context or Node 22+')
  const digest = new Uint8Array(await globalThis.crypto.subtle.digest(algorithm, bytes))
  let hex = ''
  for (let i = 0; i < digest.length; i++) hex += digest[i].toString(16).padStart(2, '0')
  return hex
}
export async function shortHash(value, length = 32) {
  if (!Number.isInteger(length) || length < 8 || length > 64) throw RangeError('shortHash: length must be 8..64 hex characters')
  return (await hashText(value)).slice(0, length)
}
