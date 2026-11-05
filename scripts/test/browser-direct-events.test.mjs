// Real native close/cancel/toggle/invalid events prove direct handlers beyond the SSR DOM oracle.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { createServer } from 'node:http'
import { readFile } from 'node:fs/promises'
import { launch, openPage } from '../../bench/lib/cdp.mjs'
test('native dialog closure restores scroll and respects cancellation, disclosure and validation events', { timeout: 60000 }, async () => {
  const source = await readFile(new URL('../../runtime/brain.js', import.meta.url))
  const server = createServer((request, response) => {
    response.setHeader('Content-Type', request.url === '/brain.js' ? 'text/javascript' : 'text/html')
    response.end(request.url === '/brain.js' ? source : '<!doctype html><title>Native event proof</title><button id="opener">Open</button><main id="app"></main>')
  })
  let chrome, page
  const escape = async () => {
    await page.send('Input.dispatchKeyEvent', { type: 'keyDown', key: 'Escape', code: 'Escape', windowsVirtualKeyCode: 27 })
    await page.send('Input.dispatchKeyEvent', { type: 'keyUp', key: 'Escape', code: 'Escape', windowsVirtualKeyCode: 27 })
  }
  try {
    await new Promise(resolve => server.listen(0, '127.0.0.1', resolve))
    chrome = await launch()
    page = await openPage(chrome.port, `http://127.0.0.1:${server.address().port}/`)
    await page.evaluate(`(async()=>{
      const {mount,on}=await import('/brain.js');
      globalThis.proof={close:0,cancel:0,toggle:0,invalid:0,block:false};
      proof.stop=mount(()=>{
        const root=document.createElement('section');root.innerHTML='<dialog><button autofocus>Inside</button></dialog><details><summary>Details</summary><p>Content</p></details><input required aria-label="Required value">';
        proof.dialog=root.querySelector('dialog');proof.details=root.querySelector('details');proof.input=root.querySelector('input');
        on(proof.dialog,'close',()=>{proof.close++;document.documentElement.style.overflow=''});
        on(proof.dialog,'cancel',e=>{proof.cancel++;proof.cancelable=e.cancelable;if(proof.block)e.preventDefault();proof.prevented=e.defaultPrevented});
        on(proof.details,'toggle',()=>proof.toggle++);
        on(proof.input,'invalid',e=>{e.preventDefault();proof.invalid++});
        return root;
      },document.querySelector('#app'));
      proof.open=()=>{document.documentElement.style.overflow='hidden';proof.dialog.showModal()};
      proof.open();
      await new Promise(resolve=>{proof.dialog.addEventListener('close',resolve,{once:true});proof.dialog.close()});
    })()`)
    assert.deepEqual(await page.evaluate('[proof.close,document.documentElement.style.overflow]'), [1, ''])
    // CloseWatcher cancellation needs activation; open through a real pointer action.
    const opener = await page.evaluate('proof.block=true;document.querySelector("#opener").onclick=()=>proof.open();(()=>{const r=document.querySelector("#opener").getBoundingClientRect();return{x:r.x+r.width/2,y:r.y+r.height/2}})()')
    for (const type of ['mousePressed','mouseReleased']) await page.send('Input.dispatchMouseEvent',{type,x:opener.x,y:opener.y,button:'left',clickCount:1})
    await escape()
    assert.deepEqual(await page.evaluate('[proof.cancel,proof.dialog.open,proof.close,proof.cancelable,proof.prevented]'), [1, true, 1, true, true])
    await page.evaluate('proof.block=false')
    await escape()
    await page.evaluate('new Promise((resolve,reject)=>{const end=Date.now()+3000;const poll=()=>proof.close===2?resolve():Date.now()>end?reject(Error("native close event missing")):setTimeout(poll,10);poll()})')
    assert.deepEqual(await page.evaluate('[proof.cancel,proof.close,document.documentElement.style.overflow]'), [2, 2, ''])
    await page.evaluate('new Promise(resolve=>{proof.details.addEventListener("toggle",resolve,{once:true});proof.details.open=true})')
    assert.equal(await page.evaluate('proof.toggle'), 1)
    assert.deepEqual(await page.evaluate('[proof.input.checkValidity(),proof.invalid]'), [false, 1])
    await page.evaluate('proof.stop();proof.dialog.dispatchEvent(new Event("close"));proof.input.dispatchEvent(new Event("invalid"))')
    assert.deepEqual(await page.evaluate('[proof.close,proof.invalid]'), [2, 1])
  } finally { await page?.close(); await chrome?.close(); await new Promise(resolve => server.close(resolve)) }
})
