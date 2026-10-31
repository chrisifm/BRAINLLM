# Native browser data and threads

Use small public URL state for navigation, IndexedDB for structured cache records, and optional official SQLite Wasm inside an owned worker when a relational local store is useful. Browser data is untrusted, evictable and never the authority for authentication. Persistent backend sessions remain HttpOnly cookies verified by the native backend.

## Shareable navigation

`urlState('overview', { key: 'tab' })` creates a signal restored from the URL. Compiler-generated bindings connect public top-level selected signals used by native select, Select and Tabs. Repeated components should supply distinct stable keys. `data-url-state="off"` opts a native selection out of automatic public navigation. A signal with an explicit urlState is not bound twice.

Restrict a value to an allowed set with `values`: a stale or hand-edited link falls back to the first allowed value
(no history entry is written), while a missing query parameter keeps the declared initial value. `<Tabs>` selects its
first tab when its bound value matches no tab.

```html
<script>
  let tab = urlState('overview', { key: 'tab', values: ['overview', 'details', 'reviews'] })
  let opened = sig([])
  watch(() => tab, (now) => (opened = [...opened, now]), { immediate: true }) // runs once now, then on every change
</script>
<p>Opened: {opened.join(', ')}</p>
<Tabs label="Product" tabs={[{ id: 'overview', label: 'Overview' }, { id: 'details', label: 'Details' }, { id: 'reviews', label: 'Reviews' }]} bind:value={tab}>
  {#if tab === 'details'}<p>Details</p>{:else if tab === 'reviews'}<p>Reviews</p>{:else}<p>Overview</p>{/if}
</Tabs>
```

`/shop?tab=bogus` renders Overview; `/shop?tab=reviews` renders Reviews, on the server and in the browser.

Native Dialog receives an opaque path-hash namespace and a ref/id identity when available, otherwise a dialog ordinal. Formatting or CSS hash shortening does not change this default key. Supply an explicit urlKey to preserve links across dialog insertion/removal or component renames. Its DOM adapter tracks open/close, restores direct links and browser back/forward, and disposes its observer/listeners with the component. Secret-like keys, email values, unsupported objects and oversized values are excluded. Unknown query parameters and existing history state survive. Default changes push history; explicit `history: 'replace'` supports high-frequency public state. Private forms, passwords, JWTs and transient loading state must never be URL state.

## Structured cache

```
const cache = database('navigation', { namespace: 'my-app', build: 'release-42' })
await cache.put('route:/products', { selected: 'cards' }, { ttl: 60000 })
const cached = await cache.get('route:/products')
await cache.batch([{ op: 'put', key: 'a', value: 1 }, { op: 'delete', key: 'b' }])
const page = await cache.scan({ prefix: 'route:', limit: 50 })
const next = await cache.scan({ prefix: 'route:', after: page.cursor })
cache.close()
```

These are BrainScript primitives, automatically imported by the compiler. IndexedDB uses a fixed native records store with atomic batches, separate namespaces, build invalidation and record expiry. Values must be JSON-serializable, each record at most 1 MiB; batches have at most 128 operations and 4 MiB. A handle admits at most 32 pending transactions. Scan/prune have explicit visit/output budgets; inspect the returned cursor and continuation flags. `prune()` removes expired or obsolete owned records. `close()` aborts owned transactions and closes the connection; other namespaces are untouched.

`storageInfo()` reports actual IndexedDB/OPFS support and browser quota estimates. `persistStorage()` requests persistence from a user action; the browser may refuse. Quota errors report B0914 with eviction guidance. Missing storage never silently downgrades durable data to memory.

`storage('settings', { namespace: 'my-app', scope: 'local' })` offers get/set/delete/clear for small public preferences; scope may be session. Keys, records and total namespace bytes are bounded. `cookie('theme', 'light')` writes a namespaced public preference with Secure/SameSite=Lax; pass null to delete. These helpers reject authentication-like keys and are not backend session APIs.

## Optional relational local data

Install the reviewed exact version of official `@sqlite.org/sqlite-wasm` in the app. Inside `lib/data.worker.jsbrain`, import its initializer and pass it to `sqlite(initialize, { namespace: 'my_app', name: 'navigation' })`. Export narrow async worker functions using the returned query/execute/batch methods. `query(sql, bindings)` permits one read-only statement; `execute(sql, bindings)` permits one mutation; `batch([{sql,bind}])` is atomic. Bind user values rather than concatenating SQL. Check the runtime method contract before generating a batch.

The adapter requires SQLite 3.54+, OPFS and Web Locks in an owned worker. It acquires an exclusive same-origin database lease before non-destructive SAHPool initialization; failures preserve existing files. No in-memory downgrade or implicit delete runs. Capacity is 64 MiB, pending work 32, rows 200 and output 2 MiB, with native SQLite expression/column/parameter/statement limits. `info()` reports limits and handle state; `close()` releases the owned database and lease. Unsupported browsers should use `database()` or a server-backed data source through an explicit app decision. Official Worker1/Promiser APIs deprecated upstream are not used.

## Threads and capabilities

Import a project worker through the native worker compiler path. Its RPC bridge owns cancellation, bounded pending work, deadlines, payload/in-flight bytes and disposal. Abort terminates an owned dedicated worker and rejects pending calls. `sharedWorker(url, {name})` owns its MessagePort; closing one client does not terminate other tabs. Remote modules and prototype/accessor RPC paths are rejected.

`transfer(value, transferables)` returns worker results without copying supported buffers. `messageChannel()` provides two native ports and close(); `offscreen(canvas)` transfers an owned canvas when supported. `sharedMemory(bytes)` requires cross-origin isolation plus SharedArrayBuffer/Atomics and rejects unsupported capabilities. `threadCapabilities()` reports actual support, including experimental features. `loadWorklet(kind, url, { context, fallback })` loads same-origin modules with explicit feature checks; audio requires the caller's existing context. Layout/animation worklets remain browser-dependent and require a real fallback. Standard WebAssembly APIs are available for app-owned modules; the compiler does not invent a universal Wasm ABI.

The generated service worker retains its versioned same-origin cache lifecycle. Push permissions, subscriptions and background execution need explicit application consent/configuration; merely having a service worker does not grant them.

Verification includes actual disposable Chrome IndexedDB transactions and OPFS persistence/rollback/reopen, plus direct native modal and selected-state history restoration without another document request. Compiler asset integration, raw SDK behavior and browser capabilities have separate tests; do not infer support for an untested browser from one Chrome run.
