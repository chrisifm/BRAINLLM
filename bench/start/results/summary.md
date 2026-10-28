# brain start QA (#120) — 2026-10-13

binary sha256 `e9453256ff44` · node v26.8.1 · darwin arm64

| Case | Result | Numbers |
|---|---|---|
| 1a kill -9 of the child → back in < 1 s (fresh supervisor per rep) | PASS | n=10 median 81.1 ms · max 239.4 ms · min 53.1 ms |
| 1b 12 kill -9 back to back on one supervisor → no crash loop, each < 1 s | PASS | recovery per kill (ms): 51.8, 73.9, 61.7, 137.9, 65.8, 68.1, 49.4, 54.4, 89.6, 91, 92, 65.9 · median 65.9 · max 137.9 · supervisor still serving · lastExit "killed from outside (killed by signal 9)" · restarts 12 |
| 2 RSS over --max-memory 1M (--rss-ms 200) → clean restart | PASS | log {"t":1791927598,"src":"brain-start","event":"memory","rssKB":53344,"limitKB":1024} · pid 24764 → 25241 · first 200 296.2 ms after watch start · lastExit "memory" · memoryRestarts 1 (68 after 15 s over the limit, supervisor still up) |
| 3 brain reload ×3 under load (50 keep-alive clients, --instances 2) | PASS | 20011 requests, 0 failed · during reloads: 3141 req, p99 179 ms (p50 4.8, max 198.2) · overall p99 156.4 ms |
| 3b brain reload ×3 under load (50 Connection: close clients, --instances 2) | PASS | 38572 requests, 0 failed · during reloads: 3361 req, p99 115 ms (p50 5.1, max 141.8) · overall p99 38.1 ms |
| 3r brain reload to a build that crashes at startup, under load (50 keep-alive clients) | PASS | reload exit 1 after 88.2 ms, {"ok":false,"error":"reload failed: a new instance exited before it was ready (exit code 1); the old instances keep serving; see logs/app.log"} · instances 53156,53159 kept · 36871 requests, 0 failed (948 during the reload, p99 25.7 ms) |
| 4a child dies before ready, always → crash loop, exit 1 within the 60 s window, stderr explains | PASS | exit 1 after 11.9 s · 9 restarts · events ["exit","restart","exit","restart","exit","restart","exit","restart","exit","restart","exit","restart","exit","restart","exit","restart","exit","restart","exit","crash-loop","stopped"] |
| 4b ready once, then every restart crashes → crash loop, exit 1 | PASS | exit 1 after 12.3 s (12.2 s after ready) · 9 restarts / 10 exits |
| 5 --install reviewed (macOS plist run for real; systemd unit from source) | PASS | 16/16 checks |
| 6 static app served without Node | PASS | ETag "qatbfgbu" = build qatbfgbu · 304 304 · /_health {"ok":true,"build":"qatbfgbu"} · 0 child processes · 11 _headers checked |

Leftover processes: none
