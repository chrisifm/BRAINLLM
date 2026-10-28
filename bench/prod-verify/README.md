# Production verification: collections, `<meta cache>`, `_loading.brain` (QA #233, #253)

End-to-end checks of three production features against apps made with `brain new`, built with `brain build` and served
by `brain start` (and by `node dist/server.mjs` where that matters).

## Method

```sh
node bench/prod-verify/run.mjs                    # builds brain-cli into $HOME/tmp/brainllm-target-qa, then every case (about 1 min)
BRAIN=/path/to/brain node bench/prod-verify/run.mjs
```

- The binary is copied (removed first, never copied over a previous copy) into the work dir
  (`BRAIN_PV_WORK`, default `~/tmp/brainllm-prodverify`), fingerprinted, and warmed once with `brain help` with no
  timeout (the first exec of a fresh copy waits for the macOS assessment).
- **#233a static blog**: `brain new pv-static`, `brain add collection blog`, two more entries in `content/blog/`. The build
  must prerender `dist/blog/index.html` (3 titles), list the entries in `sitemap.xml` and `llms.txt`, and write the `.md`
  twins. Served by `brain start` and by `node dist/server.mjs`: `/blog`, `/blog/second-post`, `/blog/tag/news`,
  `/blog/rss.xml`.
- **#233b dynamic blog**: same app, but `views/blog/index.brain` reads `ctx.query.tag` and `brain:content` in `load()`.
  The build must not prerender it (no `dist/blog/index.html`) and must list it in `dist/_server/manifest.json` with a
  server module. `?tag=` must filter under `brain start`, under `node dist/server.mjs`, and with the app's `.brain/`
  moved away (the dist must not import from the build cache). No "cannot find module" in any output or `logs/app.log`.
- **#253a `<meta cache="2">`**: `/clock`'s `load()` returns a unique stamp per render. Timeline: 0, 0.5, 1.5 s (same
  stamp), 2.3 s (stale copy, refresh starts), 2.8 s (refreshed stamp), 7.5 s (new render), then two requests with a
  `__Host-session` cookie (never cached).
- **#253b `_loading.brain`**: `/slow`'s `load()` waits 1500 ms. SSR: time to the first streamed chunk and whether that
  chunk carries the loading markup. Client: headless Chrome (`bench/lib/cdp.mjs`) opens `/`, calls the router
  (`globalThis.__brainNavigate`) and records when `[aria-label="Loading"]` and the content appear; `/about` is the
  fast-route control.
- Every process started is stopped by its own pid (`brain stop`, then the pids from `brain status --json`).
- Output: `results/summary.md` and `results/summary.json`. Exit 1 on any failed case or leftover process.

## Results

See `results/summary.md`. Run of 2026-10-13 (binary `ab097bf48b37`): 16/17 PASS. The one failure: during SSR the
streamed shell does not contain the `_loading.brain` markup; the loading view only appears on client navigation.
