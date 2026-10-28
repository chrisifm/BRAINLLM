// BRAINLLM RPC client (INTERNALS §11.1), loaded by the core's `rpc()` on the first call (the core stays small: #226).
// Same-origin POST with an anti-CSRF header; returns the response JSON. A failure (network or status ≥400) is reported
// to the catcher and rethrown to the caller; 422 (validation, `{ errors: { field: msg } }` that form() marks) is
// expected, not reported. It never imports the core (that would be a hash cycle): the core hands it `oops` and the base.
export async function rpc(id, a, oops, base) {
  let r, j
  try {
    r = await fetch(base + '_r/' + id, { method: 'POST', headers: { 'content-type': 'application/json', 'x-brain-rpc': '1' }, body: JSON.stringify(a) })
    const t = await r.text()
    j = t ? JSON.parse(t) : undefined
  } catch (e) {
    throw (oops(e, null, 'rpc'), e)
  }
  if (r.ok) return j
  const e = Object.assign(Error((j && j.error) || 'rpc ' + id + ': ' + r.status), { code: 'rpc', status: r.status, id, errors: j && j.errors })
  throw (r.status == 422 || oops(e, null, 'rpc'), e)
}
