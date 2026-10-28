// Mounts QA's channels-app shop view and exercises props, ~ model, events, slots, exposed API, context, store, bus.
import assert from 'node:assert/strict'
import { pathToFileURL } from 'node:url'
import { join } from 'node:path'

const [dir, shim] = process.argv.slice(2)
const { install, MouseEvent, Event } = await import(pathToFileURL(shim).href)
const doc = install()
const { mount } = await import(process.env.BRAIN_RUNTIME_URL)
const Shop = (await import(pathToFileURL(join(dir, 'views/shop.brain.js')).href)).default
const cart = (await import(pathToFileURL(join(dir, 'channels/cart.jsbrain.js')).href)).default

const click = el => el.dispatchEvent(new MouseEvent('click', { bubbles: true }))
const text = el => el.textContent.replace(/\s+/g, ' ').trim()
const all = (root, tag) => {
  const out = []
  const walk = n => {
    for (const c of n.childNodes) {
      if (c.nodeType === 1 && (c.tagName || c.nodeName).toLowerCase() === tag) out.push(c)
      walk(c)
    }
  }
  walk(root)
  return out
}
const flush = () => new Promise(r => setTimeout(r, 0))

const host = doc.createElement('div')
doc.body.appendChild(host)
mount(Shop, host, {})
await flush()

// 1 props + 4 slots: two product cards with prices, footer slot rendered.
const cards = all(host, 'article')
assert.equal(cards.length, 2)
assert.ok(text(cards[0]).includes('120.00 MXN'), text(cards[0]))
assert.ok(text(all(host, 'footer')[0]).length > 0)

// 2 `~` model: + in QtyInput updates the card's qty (the output shows 2).
const [dec, inc] = all(cards[0], 'button')
click(inc)
await flush()
assert.equal(text(all(cards[0], 'output')[0]), '2')

// 3 events + 7 store: "add" emits to the shop, which writes the cart store; CartSummary reads it.
const addBtn = all(cards[0], 'button')[2]
click(addBtn)
await flush()
assert.equal(cart.items.length, 1)
assert.equal(cart.items[0].qty, 2)
const aside = all(host, 'aside')[0]
assert.ok(text(aside).startsWith('1 '), text(aside))

// 6 context: the theme select feeds CartSummary through provide/use (class:dark follows it).
const select = all(host, 'select')[0]
select.value = 'dark'
select.dispatchEvent(new Event('change', { bubbles: true }))
await flush()
assert.ok((aside.getAttribute('class') || '').includes('dark'), aside.getAttribute('class'))

// 5 exposed API + 8 bus: "Vaciar" calls confirm.open() (exposed); confirming clears the store and the toast shows.
const dialog = all(host, 'dialog')[0]
dialog.showModal = () => { dialog.open = true }
dialog.close = () => { dialog.open = false }
click(all(aside, 'button')[0])
assert.equal(dialog.open, true)
click(all(dialog, 'button')[1])
await flush()
assert.equal(cart.items.length, 0)
assert.ok(all(host, 'p').some(p => p.getAttribute('role') === 'status'), 'toast visible')
console.log('ok channels')
process.exit(0)
