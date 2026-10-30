// Application-owned GIS adapter. No credentials, tokens, private errors or session data are logged or stored.
const SDK = 'https://accounts.google.com/gsi/client'
let loading = null, active = null
const sdk = () => {
  if (globalThis.google?.accounts?.id) return Promise.resolve(globalThis.google.accounts.id)
  if (loading) return loading
  loading = new Promise((resolve, reject) => {
    const doc = globalThis.document, existing = doc.querySelector(`script[src="${SDK}"]`)
    const script = existing || doc.createElement('script')
    const done = error => {
      clearTimeout(timer); script.removeEventListener('load', loaded); script.removeEventListener('error', failed)
      if (error) { if (!existing) script.remove(); reject(Error('Google sign-in is unavailable')) }
      else resolve(globalThis.google.accounts.id)
    }
    const loaded = () => globalThis.google?.accounts?.id ? done(false) : done(true), failed = () => done(true)
    const timer = setTimeout(failed, 6000)
    script.addEventListener('load', loaded); script.addEventListener('error', failed)
    if (!existing) { script.src = SDK; script.async = true; script.defer = true; doc.head.appendChild(script) }
  }).catch(error => { loading = null; throw error })
  return loading
}
export default {
  mount(element, initial = {}) {
    if (active) throw Error('Only one Google sign-in widget may be active')
    let props = initial, alive = true, busy = false, pending = null
    const owner = {}; active = owner
    const notify = (message, waiting, signedIn = false) => {
      if (!alive) return
      busy = waiting; element.setAttribute('aria-busy', String(waiting))
      if ('inert' in element) element.inert = waiting
      if (typeof props.onState === 'function') props.onState({ message, busy: waiting, signedIn })
    }
    const request = async (path, body, method = 'POST') => {
      const controller = new AbortController(); pending = controller
      const timer = setTimeout(() => controller.abort(), 8000)
      try {
        const base = globalThis.document?.querySelector('meta[name=brain-base]')?.getAttribute('content') || ''
        if (base && (!base.startsWith('/') || base.startsWith('//') || /[\\?#\x00-\x20]/.test(base))) throw Error('Invalid application base')
        const response = await fetch(base.replace(/\/$/, '') + path, { method, credentials: 'same-origin', headers: body == null ? {} : { 'content-type': 'application/json' }, ...(body == null ? {} : { body: JSON.stringify(body) }), signal: controller.signal })
        if (!response.ok) return { $status: response.status }
        return await response.json()
      } finally { clearTimeout(timer); if (pending === controller) pending = null }
    }
    const failed = () => notify('Sign-in could not be completed. Retry or use email sign-in.', false)
    const start = async () => {
      if (!alive || busy) return
      notify('Preparing secure sign-in.', true)
      try {
        const current = await request('/_brain/auth/me', null, 'GET')
        if (!alive) return
        if (current?.user) { element.replaceChildren(); return notify('You are signed in.', false, true) }
        if (current?.$status !== 401) return failed()
        const renewed = await request('/_brain/auth/refresh')
        if (!alive) return
        if (renewed?.user) { notify('You are signed in.', false, true); globalThis.location?.reload(); return }
        if (renewed?.$status !== 401) return failed()
        const challenge = await request('/_brain/auth/google/challenge')
        if (!alive) return
        if (!challenge || !/^[a-zA-Z0-9._-]{1,255}\.apps\.googleusercontent\.com$/.test(challenge.client_id) || !/^[a-f0-9]{64}$/.test(challenge.nonce)) return failed()
        const api = await sdk()
        if (!alive) return
        api.initialize({ client_id: challenge.client_id, nonce: challenge.nonce, callback: async result => {
          if (!alive || busy) return
          notify('Completing secure sign-in.', true)
          try {
            if (typeof result?.credential !== 'string' || result.credential.length > 16384) return failed()
            const session = await request('/_brain/auth/google', { credential: result.credential })
            if (!alive) return
            if (!session?.user) return failed()
            notify('You are signed in.', false, true)
            // A full auth-boundary reload forgets all prior user-specific in-memory application state.
            globalThis.location?.reload()
          } catch { if (alive) failed() }
        } })
        element.replaceChildren()
        api.renderButton(element, { type: 'standard', theme: 'outline', size: 'large', text: 'signin_with', width: 280 })
        notify('Choose Google sign-in or use email sign-in.', false)
      } catch { if (alive) failed() }
    }
    void start()
    return {
      update(next) {
        props = next || {}
        if (!alive || busy) return
        if (props.action === 'logout') {
          notify('Signing out.', true)
          void request('/_brain/auth/logout').then(result => {
            if (!alive) return
            if (!result?.ok) return failed()
            notify('You are signed out.', false); globalThis.location?.reload()
          }, () => { if (alive) failed() })
        } else void start()
      },
      destroy() {
        if (!alive) return
        alive = false; pending?.abort(); element.replaceChildren(); element.removeAttribute('aria-busy')
        if ('inert' in element) element.inert = false
        if (active === owner) active = null
      },
    }
  },
}
