// Actual native dialog behavior: backdrop opt-in, drag containment, URL state, focus and disposal.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { createServer } from 'node:http'
import { readFile } from 'node:fs/promises'
import { launch, openPage } from '../../bench/lib/cdp.mjs'
test('dismissible native dialogs close only on deliberate backdrop clicks and dispose their listeners', { timeout: 60000 }, async () => {
  const files = Object.fromEntries(await Promise.all(['brain.js', 'brain-url.js'].map(async name => [name, await readFile(new URL('../../runtime/' + name, import.meta.url))])))
  const server = createServer((request, response) => {
    const file = files[request.url.slice(1)]
    response.setHeader('Content-Type', file ? 'text/javascript' : 'text/html')
    response.end(file || '<!doctype html><title>Dialog proof</title><button id="opener">Open</button><main id="app"></main>')
  })
  let chrome, page
  const mouse = (type, x, y) => page.send('Input.dispatchMouseEvent', { type, x, y, button: type === 'mouseMoved' ? 'none' : 'left', clickCount: 1 })
  const click = async (x, y) => { await mouse('mouseMoved', x, y); await mouse('mousePressed', x, y); await mouse('mouseReleased', x, y) }
  try {
    await new Promise(resolve => server.listen(0, '127.0.0.1', resolve))
    chrome = await launch()
    page = await openPage(chrome.port, `http://127.0.0.1:${server.address().port}/`)
    await page.evaluate("new Promise(r=>document.readyState==='complete'?r():addEventListener('load',r,{once:true}))")
    await page.evaluate(`(async()=>{
      const {mount}=await import('/brain.js'),{urlDialog}=await import('/brain-url.js');
      globalThis.proof={async mount(key,dismissible){
        document.querySelector('#opener').focus();
        proof.stop=mount(()=>{
          const dialog=document.createElement('dialog');dialog.style.cssText='padding:40px;width:240px';
          dialog.innerHTML='<button autofocus>Inside</button>';proof.dialog=dialog;
          urlDialog(dialog,key,{dismissible});return dialog;
        },document.querySelector('#app'));
        proof.dialog.showModal();
        const r=proof.dialog.getBoundingClientRect();return {x:r.left+10,y:r.top+10};
      }};
    })()`)
    for (const key of ['', 'modal']) {
      const inside = await page.evaluate(`proof.mount(${JSON.stringify(key)},true)`)
      await click(inside.x, inside.y)
      assert.equal(await page.evaluate('proof.dialog.open'), true, 'dialog padding is not the backdrop')
      await mouse('mousePressed', inside.x, inside.y)
      await mouse('mouseMoved', 2, 2)
      await mouse('mouseReleased', 2, 2)
      assert.equal(await page.evaluate('proof.dialog.open'), true, 'dragging from content does not dismiss')
      await mouse('mousePressed', 2, 2)
      await page.evaluate("proof.dialog.dispatchEvent(new PointerEvent('pointercancel',{bubbles:true,pointerId:1,isPrimary:true}))")
      await mouse('mouseReleased', 2, 2)
      assert.equal(await page.evaluate('proof.dialog.open'), true, 'a canceled pointer sequence does not dismiss')
      if (key) assert.equal(await page.evaluate("new URL(location.href).searchParams.get('modal')"), 'true', 'immediate open survives initial deferred URL sync')
      await click(2, 2)
      assert.equal(await page.evaluate('proof.dialog.open'), false)
      assert.equal(await page.evaluate("document.activeElement.id"), 'opener', 'native close restores focus')
      await page.evaluate('new Promise(r=>setTimeout(r,30))')
      assert.equal(await page.evaluate("new URL(location.href).searchParams.has('modal')"), false)
      await page.evaluate('proof.stop()')
    }
    await page.evaluate("proof.mount('',false)")
    await click(2, 2)
    assert.equal(await page.evaluate('proof.dialog.open'), true, 'default behavior is preserved')
    await page.send('Input.dispatchKeyEvent', { type: 'keyDown', key: 'Escape', code: 'Escape', windowsVirtualKeyCode: 27 })
    await page.send('Input.dispatchKeyEvent', { type: 'keyUp', key: 'Escape', code: 'Escape', windowsVirtualKeyCode: 27 })
    assert.equal(await page.evaluate('proof.dialog.open'), false, 'Escape remains native')
    await page.evaluate("(async()=>{proof.stop();await proof.mount('',true);proof.dialog.close();proof.stop();document.body.appendChild(proof.dialog);proof.dialog.showModal()})()")
    await click(2, 2)
    assert.equal(await page.evaluate('proof.dialog.open'), true, 'disposed listeners cannot close a reused element')
  } finally { page?.close(); chrome?.close(); await new Promise(resolve => server.close(resolve)) }
})
