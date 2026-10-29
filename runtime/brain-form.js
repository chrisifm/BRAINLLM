// BRAINLLM forms (INTERNALS §14). `let f = form(schema, { submit: async (v) => … })` →
//   bind:value={f.v.email} · {f.err.email} · f.busy · f.error (form-level) · f.ok · on:submit={f.submit}
// Schema mini-language per field: `str num bool email url date` + `!` required + `<=N >=N` (length for str and lists,
// value for num) + `[T]` list of T, or a nested schema object (a type from types.jsbrain).
// Validation: a field is validated when it loses focus and every field on submit; errors show only after that.
// The element of each field is found without ids: the control that has focus while its value is written (the
// `bind:` input handler), or on submit `form.elements[name]`. On it the runtime keeps `aria-invalid` and, when the
// error element exists (`<Field>` or any element with id `f.id(name)`), `aria-describedby`.
// Submit: validates, focuses the first invalid control, ignores re-entry while `busy`, converts values (num → Number),
// awaits `submit(values)`. A returned or thrown `{ errors: { field: msg } }` marks fields; any other failure → f.error
// (a cancelled request, kind 'aborted', is not an error to show).
// Async checks (§41.6): `check: { email: async (v) => (await users.exists({ email: v })) ? 'Taken' : '' }` run after
// the field's own rule passes, when it loses focus and before submit; `f.checking` while any runs; a stale answer
// (the value changed meanwhile) is ignored.
// Unsaved changes (§41.5): `f.dirty` is true while values differ from the last saved ones (init, reset() or a successful
// submit); leaving the page (client navigation or closing the tab) then asks first. No API.
import { sig, fx, batch, untrack, _rt } from './brain.js'
import { rule, check, msg } from './brain-schema.js'

export { rule, check } // server.mjs validates posted forms with the same rules (§38)

const I = _rt()

const D = globalThis.document
let seq = 0
const GENERIC = 'Something went wrong. Try again.' // the text callForm (server.mjs) shows without JS (key form.error)
// navigate through the client router when it is loaded (no reload, base path applied), else a normal load
const go = u => {
  const G = globalThis
  if (G.__brainNavigate) return G.__brainNavigate(u)
  const m = D && D.querySelector('meta[name=brain-base]'), b = (m && m.getAttribute('content')) || '/'
  G.location && G.location.assign(b !== '/' && !(u + '/').startsWith(b) ? b + u.slice(1) : u)
}
// forms with unsaved changes: brain-router asks before a client navigation, the browser before unload. A form being
// submitted does not ask (#299): the user chose to save, and a submit() that navigates once the server accepted the
// data (`navigate('/products/' + p.id)`) leaves before submit() returns and marks the values saved.
const DIRTY = (globalThis.__brainDirty ||= new Set())
if (globalThis.addEventListener && !globalThis.__brainDirtyHook) {
  globalThis.__brainDirtyHook = 1
  globalThis.addEventListener('beforeunload', e => {
    for (const f of DIRTY) if (f.dirty && !f.busy) return void (e.preventDefault(), (e.returnValue = ''))
  })
}
const empty = v => v == null || v === '' || (Array.isArray(v) && !v.length)
const init0 = r => (r.list ? [] : r.t === 'bool' ? false : '')
const out = (r, v) => (r.list ? v || [] : r.t === 'num' ? (empty(v) ? null : Number(v)) : r.t === 'bool' ? !!v : v)

// state left by the server: globalThis.__brainForms while it renders, or the page's #brain-forms JSON
let SEEN
function posted(id) {
  if (!id) return null
  const g = globalThis.__brainForms
  if (g) return g[id] || null
  if (SEEN === undefined)
    try {
      const el = D && D.getElementById('brain-forms')
      SEEN = el ? JSON.parse(el.textContent) : null
    } catch {
      SEEN = null
    }
  return (SEEN && SEEN[id]) || null
}

export function form(schema, o = {}) {
  const id = 'f' + ++seq, owner = I.owner() // aria effects are created later (in handlers) but live with the component
  const busy = sig(false), error = sig(''), shown = sig(0) // shown: bumped when touched set changes
  const checking = sig(0), saved = sig('') // async checks running · JSON of the last saved values
  const F = [] // flat fields: { path, r, s (value sig), e (error sig), el, touched }
  const byPath = {}

  // Reactive getters/setters, nested for sub-schemas: f.v.address.city
  function build(sch, base, init) {
    const v = {}, err = {}
    for (const k of Object.keys(sch)) {
      const r = rule(sch[k]), path = base ? base + '.' + k : k
      if (r.t === 'obj') {
        const sub = build(r.s, path, (init && init[k]) || {})
        Object.defineProperty(v, k, { enumerable: true, value: sub.v })
        Object.defineProperty(err, k, { enumerable: true, value: sub.err })
        continue
      }
      const x = { path, r, s: sig(init && k in init ? init[k] : init0(r)), e: sig(''), el: null, touched: false }
      F.push(x)
      byPath[path] = x
      Object.defineProperty(v, k, {
        enumerable: true,
        get: () => x.s.v,
        set: val => {
          x.s.v = val
          track(x, D && D.activeElement)
          if (x.touched) validate(x) // already shown: keep the message live while fixing it
        },
      })
      Object.defineProperty(err, k, { enumerable: true, get: () => (shown.v, x.touched ? x.e.v : '') })
    }
    return { v, err }
  }

  function track(x, el) {
    // the first field written while a control has focus claims it: logic in that control's handler that writes
    // other fields does not steal it
    if (!el || x.el || el.$ff || !el.isConnected || !/^(INPUT|SELECT|TEXTAREA)$/.test(el.tagName || '')) return
    x.el = el
    el.$ff = x
    el.addEventListener('focusout', () => touch(x))
    I.root(() => fx(() => {
      const e = shown.v >= 0 && x.touched && x.e.v // reads shown first: re-runs when the field gets touched
      if (e) el.setAttribute('aria-invalid', 'true')
      else el.removeAttribute('aria-invalid')
      const d = D && D.getElementById(fid(x.path))
      if (e && d) el.setAttribute('aria-describedby', fid(x.path))
      else if (el.getAttribute('aria-describedby') === fid(x.path)) el.removeAttribute('aria-describedby')
    }), owner)
  }
  const touch = x => {
    validate(x)
    if (!x.touched) (x.touched = true), shown.v++
    remote(x)
  }
  // async check of one field (only when its own rule passes); resolves to its message
  const remote = x => {
    const c = o.check && o.check[x.path], v = untrack(() => x.s.v)
    if (!c || untrack(() => x.e.v)) return Promise.resolve(untrack(() => x.e.v))
    checking.v++
    return Promise.resolve()
      .then(() => c(v))
      .then(m => m || '', e => (e && e.kind === 'aborted' ? '' : (e && e.message) || msg('form.check_failed', 'Could not check this value.')))
      .then(m => {
        if (untrack(() => x.s.v) === v) x.e.v = m // a stale answer (the value changed meanwhile) is ignored
        return untrack(() => x.e.v)
      })
      .finally(() => checking.v--)
  }
  const validate = x => (x.e.v = untrack(() => check(x.r, x.s.v)))
  const fid = p => `${id}-${p.replace(/\./g, '-')}-err`

  // A form posted without JS (§38) comes back from the server with its values and messages: start from them, already
  // shown, both while the server renders the page and when the page hydrates
  const st = posted(o.submit && o.submit.id)
  const { v, err } = build(schema, '', st ? st.values : o.init)
  if (st) {
    for (const p in st.errors || {}) if (byPath[p]) (byPath[p].e.v = String(st.errors[p])), (byPath[p].touched = true)
    error.v = st.error || ''
  }
  const now = () => JSON.stringify(F.map(x => x.s.v))
  saved.v = untrack(now)

  function values() {
    const r = {}
    for (const x of F) {
      const ks = x.path.split('.')
      let t = r
      for (const k of ks.slice(0, -1)) t = t[k] || (t[k] = {})
      t[ks.at(-1)] = out(x.r, untrack(() => x.s.v))
    }
    return r
  }
  function mark(errors) {
    batch(() => {
      for (const p in errors) {
        const x = byPath[p]
        if (x) (x.e.v = String(errors[p])), (x.touched = true)
      }
      shown.v++
    })
  }

  // `f.v` and `f.err` are the form's live fields: replacing the object (`f.v = { … }`) would leave the inputs bound to
  // signals the form no longer reads (submit then sees every field empty), so it fails loudly instead (§14, B0133)
  const fixed = k => {
    throw Error(`form: \`${k}\` cannot be replaced; start from values with form(schema, { init }) or load them later with f.reset(values)`)
  }
  const f = {
    get v() {
      return v
    },
    set v(_) {
      fixed('f.v')
    },
    get err() {
      return err
    },
    set err(_) {
      fixed('f.err')
    },
    get busy() {
      return busy.v
    },
    get error() {
      return error.v
    },
    get dirty() {
      return now() !== saved.v
    },
    get checking() {
      return checking.v > 0
    },
    get ok() {
      for (const x of F) if (check(x.r, x.s.v)) return false
      return true
    },
    id: path => fid(path),
    values,
    touch: path => byPath[path] && touch(byPath[path]),
    errors: mark,
    reset(init) {
      batch(() => {
        for (const x of F) {
          let val = init
          for (const k of x.path.split('.')) val = val == null ? undefined : val[k]
          x.s.v = val === undefined ? init0(x.r) : val
          x.e.v = ''
          x.touched = false
        }
        error.v = ''
        shown.v++
        saved.v = untrack(now)
      })
    },
    async submit(e) {
      if (e && e.preventDefault) e.preventDefault()
      if (untrack(() => busy.v)) return // no re-entry: a double click sends once
      const fm = e && e.target && e.target.elements ? e.target : null
      let first = null
      batch(() => {
        for (const x of F) {
          if (!x.el && fm) track(x, fm.elements.namedItem && fm.elements.namedItem(x.path))
          validate(x)
          x.touched = true
          if (!first && x.e.v) first = x
        }
        error.v = ''
        shown.v++
      })
      if (o.validate && !first) {
        const cross = o.validate(values()) // cross-field rules: { field: msg }
        if (cross && Object.keys(cross).length) {
          mark(cross)
          first = byPath[Object.keys(cross)[0]] || null
          if (!first) return
        }
      }
      if (first) return void (first.el && first.el.focus && first.el.focus())
      busy.v = true
      if (fm) fm.setAttribute('aria-busy', 'true') // assistive tech hears the form is working
      try {
        if (o.check) {
          await Promise.all(F.map(remote))
          first = F.find(x => untrack(() => x.e.v))
          if (first) return void (shown.v++, first.el && first.el.focus && first.el.focus())
        }
        if (!o.submit) return
        const r = await o.submit(values())
        if (r && r.errors) mark(r.errors)
        else {
          saved.v = untrack(now) // saved: no longer dirty
          // `return { redirect: '/x' }` from the server function: the same page the 303 reaches without JS (§38)
          if (r && typeof r.redirect === 'string' && r.redirect[0] === '/' && r.redirect[1] !== '/') go(r.redirect)
        }
        return r
      } catch (x) {
        if (x && x.errors) mark(x.errors)
        // a server failure reads the same as without JS: 4xx and the author's own errors keep their message; 5xx and a
        // TypeError (the network dropped, or a bug) get the generic one
        else if (!(x && x.kind === 'aborted')) error.v = (x && x.message && !(x.status >= 500) && x.name !== 'TypeError' && x.message) || msg('form.error', GENERIC)
      } finally {
        busy.v = false
        if (fm) fm.removeAttribute('aria-busy')
      }
    },
  }
  DIRTY.add(f)
  if (owner) I.root(() => fx(() => () => DIRTY.delete(f)), owner)
  return f
}
