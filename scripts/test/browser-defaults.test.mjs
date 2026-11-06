// Real media-query behavior of the starter foundation, independent of any user's browser or application.
import test from 'node:test'
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { launch, openPage } from '../../bench/lib/cdp.mjs'
const css = readFileSync(new URL('../../crates/brain-cli/src/templates/tokens.cssbrain', import.meta.url), 'utf8')
const within = (promise, milliseconds, label) => {
  let timer
  return Promise.race([promise, new Promise((_, reject) => { timer = setTimeout(() => reject(Error(label + ' deadline')), milliseconds) })]).finally(() => clearTimeout(timer))
}
test('starter foundation keeps forced-color controls and readable print while content visibility stays explicit', { timeout: 45000 }, async () => {
  let chrome, page
  try {
    chrome = await within(launch(), 10000, 'owned Chrome startup')
    page = await within(openPage(chrome.port, 'about:blank'), 10000, 'owned page')
    await within(page.evaluate(`(() => {
      const style = document.createElement('style'); style.textContent = ${JSON.stringify(css)}; document.head.append(style);
      document.body.innerHTML = '<main><h1>Example document</h1><p>Readable text and native controls.</p><a id="link" href="#content">Read more</a><button id="control">Continue</button><section id="content">Complete content</section></main>';
    })()`), 5000, 'foundation')
    const snapshot = () => within(page.evaluate(`(() => {
      const body = getComputedStyle(document.body), link = getComputedStyle(document.querySelector('#link')), control = getComputedStyle(document.querySelector('#control'));
      return { print: matchMedia('print').matches, forced: matchMedia('(forced-colors:active)').matches,
        minHeight: body.minHeight, background: body.backgroundImage, underline: link.textDecorationLine,
        borderStyle: control.borderTopStyle, borderWidth: control.borderTopWidth,
        contentVisibility: getComputedStyle(document.querySelector('#content')).contentVisibility };
    })()`), 5000, 'media styles')
    const screen = await snapshot()
    assert.equal(screen.print, false)
    assert.equal(screen.forced, false)
    assert.equal(screen.contentVisibility, 'visible', 'ordinary content must not receive implicit layout containment')
    await within(page.send('Emulation.setEmulatedMedia', { media: 'print' }), 5000, 'print media')
    const print = await snapshot()
    assert.equal(print.print, true)
    assert.equal(print.minHeight, '0px', 'a screen-height minimum must not force extra paper space')
    assert.equal(print.background, 'none')
    assert.ok(print.underline.includes('underline'), 'printed links remain distinguishable')
    await within(page.send('Emulation.setEmulatedMedia', { media: '', features: [{ name: 'forced-colors', value: 'active' }] }), 5000, 'forced colors')
    const forced = await snapshot()
    assert.equal(forced.forced, true)
    assert.notEqual(forced.borderStyle, 'none')
    assert.notEqual(forced.borderWidth, '0px')
    await within(page.evaluate("document.head.querySelector('style').textContent += '@supports(content-visibility:auto){#content{content-visibility:auto;contain-intrinsic-size:auto 40rem}}'"), 5000, 'explicit containment')
    assert.equal((await snapshot()).contentVisibility, 'auto', 'authors can explicitly opt a measured section into native containment')
  } finally {
    page?.close()
    chrome?.close()
  }
})
