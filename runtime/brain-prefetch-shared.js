// Pure bounded transport shared by the page fallback and its dedicated module worker.
// No owners, DOM hooks, listeners or app modules execute when this module is imported.
export async function prefetchPage(url, signal) {
  try {
    const r = await fetch(url, { headers: { 'x-brain-nav': '1', 'x-brain-prefetch': '1' }, credentials: 'same-origin', priority: 'low', redirect: 'error', signal })
    const limit = 512 * 1024
    if (!r.ok || r.redirected || !/text\/html/i.test(r.headers?.get('content-type') || '') || +(r.headers.get('content-length') || 0) > limit) {
      await r.body?.cancel?.(); return null
    }
    let text, bytes = 0
    if (r.body?.getReader) {
      const reader = r.body.getReader(), decoder = new TextDecoder(), parts = []
      try {
        for (;;) {
          const { done, value } = await reader.read()
          if (done) break
          bytes += value.byteLength
          if (bytes > limit) { await reader.cancel(); return null }
          parts.push(decoder.decode(value, { stream: true }))
        }
        parts.push(decoder.decode()); text = parts.join('')
      } finally { reader.releaseLock?.() }
    } else {
      text = await r.text(); bytes = text.length * 2
      if (bytes > limit) return null
    }
    const assets = [], base = new URL(url)
    const pattern = /<script[^>]+src=["']([^"']+)["']|\bfrom\s*["']([^"']+\.(?:js|jsbrain))["']|\bimport\(\s*["']([^"']+\.(?:js|jsbrain))["']/g
    let hit
    while ((hit = pattern.exec(text)) && assets.length < 16) {
      const asset = new URL(hit[1] || hit[2] || hit[3], base)
      if (asset.origin === base.origin && asset.pathname.includes('/_b/')) assets.push(asset.href)
    }
    return { ok: true, status: r.status, url: r.url, headers: r.headers, text, bytes, assets }
  } catch { return null }
}
