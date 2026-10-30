// Small public preferences only. Authentication belongs to HttpOnly server cookies, never Web Storage.
const G = globalThis
const privateName = /password|secret|token|authorization|sessionid|csrf|jwt|credit/i
const valid = name => typeof name === 'string' && /^[a-zA-Z0-9][a-zA-Z0-9._-]{0,79}$/.test(name) && !privateName.test(name)
const problem = fix => Object.assign(new Error(fix), { code: 'B0913', fix })

export function storage(name, options = {}) {
  if (!valid(name) || !valid(options.namespace || 'app')) throw problem('storage: use public preference names; keep authentication in server sessions')
  if (options.scope && !['local', 'session'].includes(options.scope)) throw problem('storage: scope must be local or session')
  const prefix = `brain:pref:${options.namespace || 'app'}:${name}:`, version = options.version || 1
  const backend = () => {
    let s
    try { s = options.scope === 'session' ? G.sessionStorage : G.localStorage } catch {}
    if (!s) throw problem('Web Storage is unavailable; use an in-memory preference or a server data source')
    return s
  }
  const key = k => { if (!valid(k)) throw problem('storage: keys must be bounded public preference names'); return prefix + k }
  const keys = s => {
    if (s.length > 4096) throw problem('storage: origin contains too many keys; reduce stored preferences')
    const result = []
    for (let i = 0; i < s.length; i++) { const k = s.key(i); if (k?.startsWith(prefix)) result.push(k) }
    return result
  }
  return {
    get(k, fallback = null) {
      const s = backend(), raw = s.getItem(key(k))
      if (raw == null) return fallback
      if (raw.length > 8192) throw problem('storage: preference exceeds 8 KiB; use database() for structured data')
      let r; try { r = JSON.parse(raw) } catch { throw problem('storage: malformed preference; explicitly delete or replace this key') }
      return r?.v === version ? r.data : fallback
    },
    set(k, value) {
      const s = backend(), target = key(k)
      let raw
      try { raw = JSON.stringify({v:version,data:value}); if (value === undefined) throw Error() } catch { throw problem('storage: values must be JSON-serializable') }
      if (new TextEncoder().encode(raw).length > 8192) throw problem('storage: preference exceeds 8 KiB; use database() for structured data')
      const existing = keys(s)
      if (existing.length >= 128 && !existing.includes(target)) throw problem('storage: namespace exceeds 128 preference keys')
      let total = new TextEncoder().encode(raw).length
      for (const name of existing) if (name !== target) total += new TextEncoder().encode(s.getItem(name) || '').length
      if (total > 65536) throw problem('storage: namespace exceeds 64 KiB; move structured data to database()')
      try { s.setItem(target, raw) } catch { throw problem('storage: write failed or quota exceeded; reduce stored preferences') }
    },
    delete(k) { backend().removeItem(key(k)) },
    clear() { const s = backend(); for (const k of keys(s)) s.removeItem(k) },
  }
}

export function cookie(name, value, options = {}) {
  if (!valid(name)) throw problem('cookie: use a public preference name; authentication cookies belong to the backend')
  if (!G.document) throw problem('cookie: browser document unavailable; use ctx.cookies on the server')
  const key = `brain_pref_${name}`
  if (value === undefined) {
    const matches = G.document.cookie.split(';').map(x => x.trim()).filter(x => x.startsWith(key + '='))
    if (matches.length !== 1) return null
    try { return decodeURIComponent(matches[0].slice(key.length + 1)) } catch { return null }
  }
  const encoded = value === null ? '' : encodeURIComponent(String(value))
  if (encoded.length > 2048) throw problem('cookie: preferences must be at most 2 KiB; use database() for larger data')
  const age = value === null ? 0 : options.maxAge ?? 2592000
  if (!Number.isSafeInteger(age) || age < 0 || age > 31536000) throw problem('cookie: maxAge must be between zero and one year in seconds')
  G.document.cookie = `${key}=${encoded}; Path=/; Secure; SameSite=Lax; Max-Age=${age}`
  return value
}
