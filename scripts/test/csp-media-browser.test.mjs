// Exercise the compiler's emitted CSP in a real browser; no external media or provider credentials.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { createServer } from 'node:http'
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { launch, openPage } from '../../bench/lib/cdp.mjs'
const brain = process.env.BRAIN_BIN ?? new URL('../../target/debug/brain', import.meta.url).pathname

test('emitted media and worker CSP permits opted-in blob URLs and baseline blocks them', { timeout: 120000 }, async () => {
  const root = mkdtempSync(join(tmpdir(), 'brain-media-csp-'))
  let chrome, page, server
  try {
    const run = (args, cwd) => {
      const result = spawnSync(brain, args, { cwd, encoding: 'utf8', timeout: 40000, maxBuffer: 1024 * 1024 })
      assert.equal(result.status, 0, result.stderr || result.stdout || result.error?.message)
    }
    run(['new', 'app'], root)
    const app = join(root, 'app')
    const config = JSON.parse(readFileSync(join(app, 'brain.json'), 'utf8'))
    config.csp = { ...config.csp, mediaSrc: ['blob:'], workerSrc: ['blob:'] }
    writeFileSync(join(app, 'brain.json'), JSON.stringify(config))
    run(['build'], app)
    const csp = readFileSync(join(app, 'dist/_headers'), 'utf8').match(/^\s*Content-Security-Policy:\s*(.+)$/m)?.[1]
    assert.ok(csp?.includes("media-src 'self' blob:") && csp.includes("worker-src 'self' blob:"), csp)
    const baseline = csp.replace(/; (?:media|worker)-src[^;]*/g, '')
    server = createServer((req, res) => {
      res.writeHead(200, { 'content-type': 'text/html', 'content-security-policy': req.url === '/allowed' ? csp : baseline })
      res.end('<!doctype html><html><head><title>CSP proof</title></head><body></body></html>')
    })
    await new Promise(resolve => server.listen(0, '127.0.0.1', resolve))
    chrome = await launch()
    for (const allowed of [false, true]) {
      const url = `http://127.0.0.1:${server.address().port}/${allowed ? 'allowed' : 'baseline'}`
      page = await openPage(chrome.port, url)
      for (let attempt = 0; attempt < 100; attempt++) {
        if (await page.evaluate(`location.href === ${JSON.stringify(url)} && document.readyState === 'complete'`)) break
        await new Promise(resolve => setTimeout(resolve, 30))
      }
      const result = await page.evaluate(`(async () => {
        const violations = [];
        addEventListener('securitypolicyviolation', e => violations.push(e.effectiveDirective));
        const worker = await new Promise(resolve => {
          const url = URL.createObjectURL(new Blob(['postMessage("ready")'], {type:'text/javascript'}));
          let instance;
          const finish = value => { clearTimeout(timer); instance?.terminate(); URL.revokeObjectURL(url); resolve(value) };
          const timer = setTimeout(() => finish('timeout'), 3000);
          try { instance = new Worker(url); instance.onmessage = () => finish('ready'); instance.onerror = () => finish('blocked') }
          catch { finish('blocked') }
        });
        const media = await new Promise(resolve => {
          // A mono 8-bit PCM WAV with 80 silent samples.
          const bytes = new Uint8Array(124), view = new DataView(bytes.buffer);
          const text = (at, value) => { for(let i=0;i<value.length;i++) bytes[at+i]=value.charCodeAt(i) };
          text(0,'RIFF'); view.setUint32(4,116,true); text(8,'WAVE'); text(12,'fmt ');
          view.setUint32(16,16,true); view.setUint16(20,1,true); view.setUint16(22,1,true);
          view.setUint32(24,8000,true); view.setUint32(28,8000,true); view.setUint16(32,1,true);
          view.setUint16(34,8,true); text(36,'data'); view.setUint32(40,80,true); bytes.fill(128,44);
          const audio = document.createElement('audio');
          const url = URL.createObjectURL(new Blob([bytes],{type:'audio/wav'}));
          const finish = value => { clearTimeout(timer); audio.removeAttribute('src'); audio.load(); URL.revokeObjectURL(url); resolve(value) };
          const timer = setTimeout(() => finish('timeout'),3000);
          audio.onloadedmetadata = () => finish('loaded'); audio.onerror = () => finish('blocked');
          audio.src = url; audio.load();
        });
        await new Promise(resolve => setTimeout(resolve,50));
        return {worker,media,violations};
      })()`)
      assert.equal(result.worker, allowed ? 'ready' : 'blocked', JSON.stringify(result))
      assert.equal(result.media, allowed ? 'loaded' : 'blocked', JSON.stringify(result))
      if (allowed) assert.deepEqual(result.violations, [])
      else {
        assert.ok(result.violations.includes('worker-src'), JSON.stringify(result))
        assert.ok(result.violations.includes('media-src'), JSON.stringify(result))
      }
      page.close(); page = undefined
    }
  } finally {
    page?.close(); chrome?.close()
    if (server) await new Promise(resolve => { server.close(resolve); server.closeAllConnections() })
    rmSync(root, { recursive: true, force: true })
  }
})
