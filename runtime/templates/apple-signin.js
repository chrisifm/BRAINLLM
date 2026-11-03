// Application-owned Sign in with Apple JS adapter (popup flow). No tokens, private errors or session data are logged or stored.
const SDK = 'https://appleid.cdn-apple.com/appleauth/static/jsapi/appleid/1/en_US/appleid.auth.js'
let loading = null, active = null
const sdk = () => {
  if (globalThis.AppleID?.auth) return Promise.resolve(globalThis.AppleID.auth)
  if (loading) return loading
  loading = new Promise((resolve, reject) => {
    const doc = globalThis.document, existing = doc.querySelector(`script[src="${SDK}"]`)
    const script = existing || doc.createElement('script')
    const done = error => {
      clearTimeout(timer); script.removeEventListener('load', loaded); script.removeEventListener('error', failed)
      if (error) { if (!existing) script.remove(); reject(Error('Apple sign-in is unavailable')) }
      else resolve(globalThis.AppleID.auth)
    }
    const loaded = () => globalThis.AppleID?.auth ? done(false) : done(true), failed = () => done(true)
    const timer = setTimeout(failed, 6000)
    script.addEventListener('load', loaded); script.addEventListener('error', failed)
    if (!existing) { script.src = SDK; script.async = true; script.defer = true; doc.head.appendChild(script) }
  }).catch(error => { loading = null; throw error })
  return loading
}
// Apple sends the name only on the first authorization and unsigned: it is a display hint, never an identity.
const name = user => {
  const first = typeof user?.name?.firstName === 'string' ? user.name.firstName.trim() : ''
  const last = typeof user?.name?.lastName === 'string' ? user.name.lastName.trim() : ''
  const full = `${first} ${last}`.trim()
  return full && full.length <= 200 ? full : undefined
}
export default {
  mount(element, initial = {}) {
    if (active) throw Error('Only one Apple sign-in widget may be active')
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
        const challenge = await request('/_brain/auth/apple/challenge')
        if (!alive) return
        if (!challenge || !/^[A-Za-z0-9-]+(\.[A-Za-z0-9-]+)+$/.test(challenge.client_id) || !/^[a-f0-9]{64}$/.test(challenge.nonce) || typeof challenge.redirect_uri !== 'string') return failed()
        const auth = await sdk()
        if (!alive) return
        auth.init({ clientId: challenge.client_id, scope: 'name email', redirectURI: challenge.redirect_uri, nonce: challenge.nonce, usePopup: true })
        const button = globalThis.document.createElement('button')
        button.type = 'button'; button.className = 'brain-apple-signin'; button.textContent = 'Sign in with Apple'
        button.addEventListener('click', async () => {
          if (!alive || busy) return
          notify('Completing secure sign-in.', true)
          try {
            const result = await auth.signIn()
            const token = result?.authorization?.id_token
            if (!alive) return
            if (typeof token !== 'string' || token.length > 16384) return failed()
            const session = await request('/_brain/auth/apple', { id_token: token, ...(name(result.user) ? { name: name(result.user) } : {}) })
            if (!alive) return
            if (!session?.user) return failed()
            notify('You are signed in.', false, true)
            // A full auth-boundary reload forgets all prior user-specific in-memory application state.
            globalThis.location?.reload()
          } catch {
            // A closed popup (`popup_closed_by_user`) is a choice, not an error.
            if (alive) notify('Choose Apple sign-in or use email sign-in.', false)
          }
        })
        element.replaceChildren(button)
        notify('Choose Apple sign-in or use email sign-in.', false)
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
