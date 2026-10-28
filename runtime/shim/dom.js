// Minimal DOM to test the runtime in Node with no dependencies. Covers only what brain.js uses.
// Installs `document`, `Event` and `MouseEvent` on globalThis. `stats.moves` counts insertions of already-connected nodes.

export const stats = { moves: 0 }
const VOID = new Set(['area', 'br', 'col', 'embed', 'hr', 'img', 'input', 'link', 'meta', 'source', 'track', 'wbr'])
const esc = s => String(s).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;')
const unesc = s => s.replace(/&(amp|lt|gt|quot|#39);/g, (_, e) => ({ amp: '&', lt: '<', gt: '>', quot: '"', '#39': "'" })[e])

export class Event {
  constructor(type, init = {}) {
    this.type = type
    this.bubbles = !!init.bubbles
    this.cancelable = !!init.cancelable
    this.target = null
    this.currentTarget = null
    this.cancelBubble = false
    this.defaultPrevented = false
    this._p = []
  }
  composedPath() {
    return this._p
  }
  stopImmediatePropagation() {
    this.cancelBubble = this._stop = true
  }
  stopPropagation() {
    this.cancelBubble = true
  }
  preventDefault() {
    this.defaultPrevented = true
  }
}
export class MouseEvent extends Event {}

// Minimal MutationObserver: only records for nodes connected to the document, plus takeRecords() (used by brain-inspect).
const MOS = new Set()
const rec = (type, target) => {
  if (MOS.size && target.isConnected) for (const o of MOS) o._q.push({ type, target })
}
export class MutationObserver {
  constructor(f) {
    this._f = f
    this._q = []
  }
  observe() {
    MOS.add(this)
  }
  disconnect() {
    MOS.delete(this)
    this._q = []
  }
  takeRecords() {
    const q = this._q
    this._q = []
    return q
  }
}

class Node {
  constructor(type, doc) {
    this.nodeType = type
    this.ownerDocument = doc
    this.parentNode = null
    this.childNodes = []
    this._l = { cap: {}, bub: {} }
  }
  get firstChild() {
    return this.childNodes[0] || null
  }
  get lastChild() {
    return this.childNodes[this.childNodes.length - 1] || null
  }
  get nextSibling() {
    const p = this.parentNode
    return p ? p.childNodes[p.childNodes.indexOf(this) + 1] || null : null
  }
  get previousSibling() {
    const p = this.parentNode
    return p ? p.childNodes[p.childNodes.indexOf(this) - 1] || null : null
  }
  get isConnected() {
    let n = this
    while (n.parentNode) n = n.parentNode
    return n.nodeType === 9
  }
  insertBefore(n, ref) {
    if (n.nodeType === 11) {
      for (const c of [...n.childNodes]) this.insertBefore(c, ref)
      return n
    }
    if (n.parentNode) {
      if (n.isConnected) stats.moves++
      n.parentNode.removeChild(n)
    }
    const i = ref ? this.childNodes.indexOf(ref) : -1
    if (ref && i < 0) throw Error('ref is not a child')
    i < 0 ? this.childNodes.push(n) : this.childNodes.splice(i, 0, n)
    n.parentNode = this
    rec('childList', this)
    return n
  }
  appendChild(n) {
    return this.insertBefore(n, null)
  }
  removeChild(n) {
    const i = this.childNodes.indexOf(n)
    if (i < 0) throw Error('not a child')
    this.childNodes.splice(i, 1)
    n.parentNode = null
    rec('childList', this)
    return n
  }
  remove() {
    if (this.parentNode) this.parentNode.removeChild(this)
  }
  replaceWith(...ns) {
    const p = this.parentNode
    for (const n of ns) p.insertBefore(n, this)
    this.remove()
  }
  get textContent() {
    return this.nodeType === 3 ? this.data : this.childNodes.filter(c => c.nodeType !== 8).map(c => c.textContent).join('')
  }
  set textContent(v) {
    for (const c of this.childNodes) c.parentNode = null
    this.childNodes = []
    if (v !== '') this.appendChild(this.ownerDocument.createTextNode(v))
  }
  addEventListener(t, f, cap) {
    const a = (this._l[cap ? 'cap' : 'bub'][t] ||= [])
    if (!a.includes(f)) a.push(f) // the same listener twice is one listener (DOM spec)
  }
  removeEventListener(t, f, cap) {
    const a = this._l[cap ? 'cap' : 'bub'][t] || []
    const i = a.indexOf(f)
    if (i >= 0) a.splice(i, 1)
  }
  dispatchEvent(e) {
    e.target = this
    const path = []
    for (let n = this; n; n = n.parentNode || n.host) path.push(n)
    if (path[path.length - 1].nodeType === 9 && path[path.length - 1].defaultView) path.push(path[path.length - 1].defaultView)
    e._p = path
    const fire = (n, k) => {
      for (const f of [...(n._l[k][e.type] || [])]) {
        if (e._stop) return
        Object.defineProperty(e, 'currentTarget', { configurable: true, writable: true, value: n })
        f.call(n, e)
      }
    }
    for (let i = path.length - 1; i >= 0 && !e.cancelBubble; i--) fire(path[i], 'cap')
    for (let i = 0; i < path.length && !e.cancelBubble; i++) {
      fire(path[i], 'bub')
      if (!e.bubbles) break
    }
    if (!e.defaultPrevented && e.type === 'click' && this.tagName === 'INPUT' && this.type === 'checkbox') this.checked = !this.checked
    return !e.defaultPrevented
  }
  contains(n) {
    for (; n; n = n.parentNode) if (n === this) return true
    return false
  }
  // Supported selectors: lists (`a, b`), descendant (`a b`), compounds of `tag`, `.class`, `#id`, `[attr]`,
  // `[attr=v]` (quoted or not) and `:not(<compound>)`. Enough for the runtime and the ui kit.
  querySelectorAll(sel) {
    const alts = sel.split(',').map(a => a.trim().split(/\s+/).map(compound))
    const out = []
    const walk = n => {
      for (const c of n.childNodes) {
        if (c.nodeType === 1 && alts.some(chain => matchChain(c, chain, this))) out.push(c)
        walk(c)
      }
    }
    walk(this)
    return out
  }
  querySelector(sel) {
    return this.querySelectorAll(sel)[0] || null
  }
  get innerHTML() {
    return this.childNodes.map(ser).join('')
  }
  set innerHTML(h) {
    this.textContent = ''
    parse(h, this.nodeName === 'TEMPLATE' ? this.content : this, this.ownerDocument)
  }
}

class Text extends Node {
  constructor(d, doc) {
    super(3, doc)
    this._d = String(d)
  }
  get data() {
    return this._d
  }
  set data(v) {
    this._d = String(v)
    rec('characterData', this)
  }
  get nodeValue() {
    return this._d
  }
  set nodeValue(v) {
    this.data = v
  }
  cloneNode() {
    return new Text(this.data, this.ownerDocument)
  }
}
class Comment extends Node {
  constructor(d, doc) {
    super(8, doc)
    this._d = String(d)
  }
  get data() {
    return this._d
  }
  set data(v) {
    this._d = String(v)
    rec('characterData', this)
  }
  get nodeValue() {
    return this._d
  }
  set nodeValue(v) {
    this.data = v
  }
  cloneNode() {
    return new Comment(this.data, this.ownerDocument)
  }
}
class Fragment extends Node {
  constructor(doc) {
    super(11, doc)
  }
  cloneNode(deep) {
    const f = new Fragment(this.ownerDocument)
    if (deep) for (const c of this.childNodes) f.appendChild(c.cloneNode(true))
    return f
  }
}

class Element extends Node {
  constructor(tag, doc) {
    super(1, doc)
    this.tagName = this.nodeName = tag.toUpperCase()
    this.attrs = new Map()
    this.value = ''
    this.checked = false
    this.selected = false
    const el = this
    this.style = {
      _p: new Map(),
      setProperty(k, v) {
        this._p.set(k, String(v))
      },
      removeProperty(k) {
        this._p.delete(k)
      },
      getPropertyValue(k) {
        return this._p.get(k) || ''
      },
    }
    this.classList = {
      _l: () => (el.getAttribute('class') || '').split(/\s+/).filter(Boolean),
      contains: c => el.classList._l().includes(c),
      toggle(c, on) {
        const l = this._l().filter(x => x !== c)
        if (on) l.push(c)
        l.length ? el.setAttribute('class', l.join(' ')) : el.removeAttribute('class')
        return on
      },
    }
    this.dataset = new Proxy({}, { get: (_, k) => el.getAttribute('data-' + String(k).replace(/[A-Z]/g, c => '-' + c.toLowerCase())) ?? undefined })
    if (this.tagName === 'TEMPLATE') this.content = new Fragment(doc)
  }
  get type() {
    return this.getAttribute('type') || ''
  }
  setAttribute(k, v) {
    this.attrs.set(k, String(v))
    rec('attributes', this)
    if (k === 'value') this.value = String(v)
    if (k === 'checked') this.checked = true
  }
  get attributes() {
    return [...this.attrs].map(([name, value]) => ({ name, value }))
  }
  replaceChildren(...ns) {
    this.textContent = ''
    for (const n of ns) this.appendChild(n)
  }
  getAttribute(k) {
    return this.attrs.has(k) ? this.attrs.get(k) : null
  }
  hasAttribute(k) {
    return this.attrs.has(k)
  }
  removeAttribute(k) {
    if (this.attrs.delete(k)) rec('attributes', this)
  }
  // reflected attributes the runtime and the ui kit read
  get id() {
    return this.getAttribute('id') || ''
  }
  set id(v) {
    this.setAttribute('id', v)
  }
  // HTMLFormElement.elements (namedItem only)
  get elements() {
    if (this.tagName !== 'FORM') return undefined
    const all = this.querySelectorAll('[name]')
    return { namedItem: n => all.find(e => e.getAttribute('name') === n) || null }
  }
  focus() {
    this.ownerDocument.activeElement = this
  }
  cloneNode(deep) {
    const e = new Element(this.tagName.toLowerCase(), this.ownerDocument)
    for (const [k, v] of this.attrs) e.setAttribute(k, v)
    if (deep) {
      for (const c of this.childNodes) e.appendChild(c.cloneNode(true))
      if (this.content) for (const c of this.content.childNodes) e.content.appendChild(c.cloneNode(true))
    }
    return e
  }
  get outerHTML() {
    return ser(this)
  }
}

function ser(n) {
  if (n.nodeType === 3) return esc(n.data)
  if (n.nodeType === 8) return `<!--${n.data}-->`
  if (n.nodeType === 11) return n.childNodes.map(ser).join('')
  const t = n.tagName.toLowerCase()
  const a = [...n.attrs].map(([k, v]) => (v === '' ? ` ${k}` : ` ${k}="${esc(v).replace(/"/g, '&quot;')}"`)).join('')
  return VOID.has(t) ? `<${t}${a}>` : `<${t}${a}>${n.childNodes.map(ser).join('')}</${t}>`
}

// Minimal HTML parser: tags, attributes, text, comments (including `<!>`), void elements and script raw text.
function parse(h, root, doc) {
  const stack = [root]
  const top = () => stack[stack.length - 1]
  const re = /<!--([\s\S]*?)-->|<!>|<\/([\w-]+)\s*>|<([\w-]+)((?:\s+[^\s=\/>]+(?:\s*=\s*(?:"[^"]*"|'[^']*'|[^\s>]+))?)*)\s*(\/?)>|([^<]+|<)/g
  let m
  while ((m = re.exec(h))) {
    if (m[0] === '<!>') top().appendChild(new Comment('', doc))
    else if (m[1] != null) top().appendChild(new Comment(m[1], doc))
    else if (m[2]) {
      const t = m[2].toUpperCase()
      for (let i = stack.length - 1; i > 0; i--)
        if (stack[i].tagName === t) {
          stack.length = i
          break
        }
    } else if (m[3]) {
      const el = new Element(m[3], doc)
      for (const a of m[4].matchAll(/([^\s=\/>]+)(?:\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s>]+)))?/g)) el.setAttribute(a[1], unesc(a[2] ?? a[3] ?? a[4] ?? ''))
      top().appendChild(el)
      const t = m[3].toLowerCase()
      if (t === 'script') {
        const end = h.indexOf('</script>', re.lastIndex)
        el.appendChild(new Text(h.slice(re.lastIndex, end), doc))
        re.lastIndex = end + 9
      } else if (!VOID.has(t) && !m[5]) stack.push(t === 'template' ? Object.assign(el.content, { tagName: 'TEMPLATE' }) : el)
    } else top().appendChild(new Text(unesc(m[6]), doc))
  }
}

class Document extends Node {
  constructor() {
    super(9, null)
    this.ownerDocument = this
    this.baseURI = 'http://localhost/'
    this.activeElement = null
    // <html><head></head><body></body></html>, as in a browser
    this.documentElement = this.appendChild(new Element('html', this))
    this.head = this.documentElement.appendChild(new Element('head', this))
    this.body = this.documentElement.appendChild(new Element('body', this))
  }
  createElement(t) {
    return new Element(t, this)
  }
  createTextNode(d) {
    return new Text(d, this)
  }
  createComment(d) {
    return new Comment(d, this)
  }
  createDocumentFragment() {
    return new Fragment(this)
  }
  getElementById(id) {
    const walk = n => {
      for (const c of n.childNodes) {
        if (c.nodeType !== 1) continue
        if (c.getAttribute('id') === id) return c
        const r = walk(c)
        if (r) return r
      }
      return null
    }
    return walk(this)
  }
}

// Selector helpers for querySelectorAll: a compound is a list of tests on one element.
function compound(str) {
  const tests = []
  const rx = /^(?:([\w-]+)|\.([\w-]+)|#([\w-]+)|\[([\w-]+)(?:=["']?([^"'\]]*)["']?)?\]|:not\(([^)]*)\))/
  let rest = str
  while (rest) {
    const m = rx.exec(rest)
    if (!m) throw Error('unsupported selector: ' + str)
    rest = rest.slice(m[0].length)
    if (m[1]) tests.push(e => e.tagName === m[1].toUpperCase())
    else if (m[2]) tests.push(e => (e.getAttribute('class') || '').split(/\s+/).includes(m[2]))
    else if (m[3]) tests.push(e => e.getAttribute('id') === m[3])
    else if (m[4]) tests.push(e => e.hasAttribute(m[4]) && (m[5] == null || e.getAttribute(m[4]) === m[5]))
    else {
      const inner = compound(m[6])
      tests.push(e => !inner(e))
    }
  }
  return e => tests.every(t => t(e))
}
// chain = [c0, c1, …, cn]: the element matches cn and has ancestors (below root) matching the rest, in order.
function matchChain(el, chain, root) {
  if (!chain[chain.length - 1](el)) return false
  let i = chain.length - 2
  for (let a = el.parentNode; i >= 0 && a && a !== root; a = a.parentNode) if (a.nodeType === 1 && chain[i](a)) i--
  return i < 0
}

export function install() {
  const doc = new Document()
  // real window at the end of the event path (after document), as in the browser
  const win = new Node(0, doc)
  doc.defaultView = win
  globalThis.window = win
  globalThis.document = doc
  globalThis.Event = Event
  globalThis.MouseEvent = MouseEvent
  globalThis.MutationObserver = MutationObserver
  return doc
}
