// Application-owned captcha adapter for widgets with the common render/reset/remove API. The token stays in memory, is
// never logged or stored, and is good for one submit: reset after every submit, whatever it answered.
const PROVIDERS = {
  turnstile: { global: 'turnstile', script: 'https://challenges.cloudflare.com/turnstile/v0/api.js?render=explicit' },
  hcaptcha: { global: 'hcaptcha', script: 'https://js.hcaptcha.com/1/api.js?render=explicit' },
  recaptcha: { global: 'grecaptcha', script: 'https://www.google.com/recaptcha/api.js?render=explicit' }
}
const LOAD_MS = 8000
const loading = new Map()
const sdk = provider => {
  const ready = () => typeof globalThis[provider.global]?.render === 'function'
  if (ready()) return Promise.resolve(globalThis[provider.global])
  if (loading.has(provider.script)) return loading.get(provider.script)
  const started = new Promise((resolve, reject) => {
    const doc = globalThis.document, existing = doc.querySelector(`script[src="${provider.script}"]`)
    const script = existing || doc.createElement('script')
    const done = failed => {
      clearTimeout(timer); clearInterval(poll); script.removeEventListener('error', fail)
      loading.delete(provider.script)
      if (failed) { if (!existing) script.remove(); reject(Error('The captcha could not load')) }
      else resolve(globalThis[provider.global])
    }
    const fail = () => done(true)
    // The global appears after `load`, and grecaptcha later still, so readiness is polled rather than assumed.
    const poll = setInterval(() => { if (ready()) done(false) }, 50)
    const timer = setTimeout(fail, LOAD_MS)
    script.addEventListener('error', fail)
    if (!existing) { script.src = provider.script; script.async = true; script.defer = true; doc.head.appendChild(script) }
  })
  loading.set(provider.script, started)
  return started
}
/** A same-origin JSON POST with the captcha headers. Answers `{ status, body }`; the caller decides what a failure means. */
export async function post(path, body, headers = {}) {
  const base = globalThis.document?.querySelector('meta[name=brain-base]')?.getAttribute('content') || ''
  if (base && (!base.startsWith('/') || base.startsWith('//') || /[\\?#\x00-\x20]/.test(base))) throw Error('Invalid application base')
  const response = await fetch(base.replace(/\/$/, '') + path, { method: 'POST', credentials: 'same-origin', headers: { 'content-type': 'application/json', ...headers }, body: JSON.stringify(body) })
  return { status: response.status, body: await response.json().catch(() => null) }
}
export default {
  post,
  mount(element, initial = {}) {
    let props = initial, alive = true, api = null, id = null, nonce = 0
    const report = (status, token = '') => {
      if (!alive) return
      element.setAttribute('aria-busy', String(status === 'loading'))
      if (typeof props.onState === 'function') props.onState({ status, token })
    }
    const start = async () => {
      const mine = ++nonce
      const provider = PROVIDERS[props.provider || 'turnstile']
      if (!provider || !props.sitekey) return report('error')
      report('loading')
      try {
        const loaded = await sdk(provider)
        if (!alive || mine !== nonce) return
        api = loaded
        const render = () => {
          if (!alive || mine !== nonce) return
          id = api.render(element, {
            sitekey: props.sitekey,
            callback: token => report('verified', typeof token === 'string' ? token : ''),
            'expired-callback': () => report('expired'),
            'timeout-callback': () => report('expired'),
            'error-callback': () => { report('error'); return true }
          })
          report('ready')
        }
        // reCAPTCHA defines render only once it is ready; the others have no `ready`, or call back at once.
        if (typeof api.ready === 'function') api.ready(render)
        else render()
      } catch { if (alive && mine === nonce) report('error') }
    }
    const reset = () => {
      if (id === null) return start()
      report('ready')
      try { api.reset(id) } catch { report('error') }
    }
    start()
    return {
      update(next = {}) {
        props = { ...props, ...next }
        if (next.action === 'reset') reset()
      },
      destroy() {
        alive = false; nonce++
        try { if (id !== null && typeof api?.remove === 'function') api.remove(id) } catch {}
        element.replaceChildren()
      }
    }
  }
}
