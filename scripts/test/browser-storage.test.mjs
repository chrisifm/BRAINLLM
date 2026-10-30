// Actual IndexedDB transactions in an owned disposable Chrome profile. No fake storage dependency.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { createServer } from 'node:http'
import { readFile } from 'node:fs/promises'
import { launch, openPage } from '../../bench/lib/cdp.mjs'

test('native IndexedDB survives connections, enforces transactions and isolates builds/namespaces', { timeout: 60000 }, async () => {
  let chrome, page
  const source = await readFile(new URL('../../runtime/brain-storage.js', import.meta.url))
  const preferences = await readFile(new URL('../../runtime/brain-preferences.js', import.meta.url))
  const server = createServer((req, res) => {
    res.setHeader('Cache-Control', 'no-store')
    res.setHeader('Content-Type', /\.js$/.test(req.url) ? 'text/javascript' : 'text/html')
    res.end(req.url === '/storage.js' ? source : req.url === '/preferences.js' ? preferences : '<!doctype html><title>Storage proof</title>')
  })
  const deadline = async (promise, ms = 10000) => {
    let timer
    try { return await Promise.race([promise, new Promise((_, reject) => { timer = setTimeout(() => reject(Error('Browser storage proof deadline')), ms) })]) }
    finally { clearTimeout(timer) }
  }
  try {
    await new Promise(resolve => server.listen(0, '127.0.0.1', resolve))
    chrome = await deadline(launch())
    page = await deadline(openPage(chrome.port, `http://127.0.0.1:${server.address().port}/`))
    const result = await deadline(page.evaluate(`(async () => {
      const { database, storageInfo } = await import('/storage.js');
      const { storage, cookie } = await import('/preferences.js');
      const prefs = storage('settings', {namespace:'proof'}), otherPrefs = storage('settings', {namespace:'other'});
      prefs.set('theme', 'light'); otherPrefs.set('theme', 'dark');
      const prefValue = prefs.get('theme'); prefs.clear();
      const prefIsolation = prefs.get('theme') === null && otherPrefs.get('theme') === 'dark';
      cookie('theme', 'light'); const cookieValue = cookie('theme'); cookie('theme', null);
      const cookieDeleted = cookie('theme') === null;
      let privateBlocked = false; try { prefs.set('accessToken', 'never-store') } catch { privateBlocked = true }
      const db = database('test', { namespace: 'proof', build: 'v1' });
      await db.put('p:a', { selected: 'card', nested: [1, 2] });
      await db.batch([{op:'put',key:'p:b',value:2},{op:'put',key:'p:c',value:3}]);
      const first = await db.scan({prefix:'p:',limit:2});
      const next = await db.scan({prefix:'p:',after:first.cursor});
      let atomicFailure = false;
      try { await db.batch([{op:'put',key:'never',value:1},{op:'unknown',key:'x'}]) } catch { atomicFailure = true }
      const absent = await db.get('never');
      let invalid = 0;
      for (const value of [undefined, () => 1, 'x'.repeat(1048577)]) { try { await db.put('bad', value) } catch { invalid++ } }
      await db.put('expired', 1, {ttl:1}); await new Promise(r => setTimeout(r, 10));
      const expired = await db.get('expired');
      db.close();
      const reopened = database('test', {namespace:'proof',build:'v1'});
      const restored = await reopened.get('p:a');
      const isolated = database('test', {namespace:'other',build:'v1'});
      const other = await isolated.get('p:a');
      const upgraded = database('test', {namespace:'proof',build:'v2'});
      const stale = await upgraded.get('p:a');
      const pruned = await upgraded.prune();
      const info = await storageInfo();
      reopened.close(); isolated.close(); upgraded.close();
      let closed = false; try { await reopened.get('p:a') } catch { closed = true }
      return { first, next, atomicFailure, absent, invalid, expired, restored, other, stale, pruned, indexedDB:info.indexedDB, closed, prefValue, prefIsolation, cookieValue, cookieDeleted, privateBlocked };
    })()`), 15000)
    assert.equal(result.first.items.length, 2)
    assert.equal(result.next.items[0].key, 'p:c')
    assert.equal(result.atomicFailure, true)
    assert.equal(result.absent, null)
    assert.equal(result.invalid, 3)
    assert.equal(result.expired, null)
    assert.deepEqual(result.restored, { selected: 'card', nested: [1, 2] })
    assert.equal(result.other, null)
    assert.equal(result.stale, null)
    assert.equal(result.pruned.deleted, 4)
    assert.equal(result.indexedDB, true)
    assert.equal(result.closed, true)
    assert.equal(result.prefValue, 'light')
    assert.equal(result.prefIsolation, true)
    assert.equal(result.cookieValue, 'light')
    assert.equal(result.cookieDeleted, true)
    assert.equal(result.privateBlocked, true)
  } finally {
    page?.close(); chrome?.close()
    await new Promise(resolve => server.close(resolve))
  }
})
