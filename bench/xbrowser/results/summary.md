# Cross-browser matrix — 2026-10-13

playwright 1.64.0 · webkit 27.2 · firefox 157.0 · chromium 156.0.8078.4 · brain sha256 e9453256ff44 · node v26.8.1

| app | mode | browser | routes OK | interactions OK | console/page/net errors | nav without reload (router) | Navigation API | View Transitions | moveBefore | scheduler.yield | HMR style / script / both-in-one-save |
|---|---|---|---|---|---|---|---|---|---|---|---|
| tasks | dev | webkit 27.2 | 2/2 | 9/9 | 0 | yes (History API (pushState)) | yes | yes | no | no | ok / ok / ok |
| tasks | dev | firefox 157.0 | 2/2 | 9/9 | 0 | yes (History API (pushState)) | yes | yes | yes | yes | ok / ok / ok |
| tasks | dev | chromium 156.0.8078.4 | 2/2 | 9/9 | 0 | yes (History API (pushState)) | yes | yes | yes | yes | ok / ok / ok |
| tasks | build | webkit 27.2 | 2/2 | 9/9 | 0 | yes (Navigation API) | yes | yes (4 started, 4 finished, 0 skipped) | no | no | n/a |
| tasks | build | firefox 157.0 | 2/2 | 9/9 | 0 | yes (Navigation API) | yes | yes (4 started, 4 finished, 0 skipped) | yes | yes | n/a |
| tasks | build | chromium 156.0.8078.4 | 2/2 | 9/9 | 0 | yes (Navigation API) | yes | yes (4 started, 4 finished, 0 skipped) | yes | yes | n/a |
| landing | dev | webkit 27.2 | 1/1 | 6/6 | 0 | yes (History API (pushState)) | yes | yes | no | no | ok / ok / ok |
| landing | dev | firefox 157.0 | 1/1 | 6/6 | 0 | yes (History API (pushState)) | yes | yes | yes | yes | ok / ok / ok |
| landing | dev | chromium 156.0.8078.4 | 1/1 | 6/6 | 0 | yes (History API (pushState)) | yes | yes | yes | yes | ok / ok / ok |
| landing | build | webkit 27.2 | 1/1 | 6/6 | 0 | n/a (none shipped (single route)) | yes | yes (n/a) | no | no | n/a |
| landing | build | firefox 157.0 | 1/1 | 6/6 | 0 | n/a (none shipped (single route)) | yes | yes (n/a) | yes | yes | n/a |
| landing | build | chromium 156.0.8078.4 | 1/1 | 6/6 | 0 | n/a (none shipped (single route)) | yes | yes (n/a) | yes | yes | n/a |
| my-project | dev | webkit 27.2 | 1/1 | 4/4 | 0 | yes (History API (pushState)) | yes | yes | no | no | not run |
| my-project | dev | firefox 157.0 | 1/1 | 4/4 | 0 | yes (History API (pushState)) | yes | yes | yes | yes | not run |
| my-project | dev | chromium 156.0.8078.4 | 1/1 | 4/4 | 0 | yes (History API (pushState)) | yes | yes | yes | yes | not run |
| my-project | build | webkit 27.2 | 1/1 | 4/4 | 0 | n/a (none shipped (single route)) | yes | yes (n/a) | no | no | n/a |
| my-project | build | firefox 157.0 | 1/1 | 4/4 | 0 | n/a (none shipped (single route)) | yes | yes (n/a) | yes | yes | n/a |
| my-project | build | chromium 156.0.8078.4 | 1/1 | 4/4 | 0 | n/a (none shipped (single route)) | yes | yes (n/a) | yes | yes | n/a |
| biglist | dev | webkit 27.2 | 1/1 | 5/5 | 0 | yes (History API (pushState)) | yes | yes | no | no | not run |
| biglist | dev | firefox 157.0 | 1/1 | 5/5 | 0 | yes (History API (pushState)) | yes | yes | yes | yes | not run |
| biglist | dev | chromium 156.0.8078.4 | 1/1 | 5/5 | 0 | yes (History API (pushState)) | yes | yes | yes | yes | not run |
| biglist | build | webkit 27.2 | 1/1 | 5/5 | 0 | yes (Navigation API) | yes | yes (3 started, 3 finished, 0 skipped) | no | no | n/a |
| biglist | build | firefox 157.0 | 1/1 | 5/5 | 0 | yes (Navigation API) | yes | yes (3 started, 3 finished, 0 skipped) | yes | yes | n/a |
| biglist | build | chromium 156.0.8078.4 | 1/1 | 5/5 | 0 | yes (Navigation API) | yes | yes (3 started, 3 finished, 0 skipped) | yes | yes | n/a |

## Big list (1000 keyed rows)

| mode | browser | fill: rows right after click → final | reverse keeps node identity | reverse keeps focus |
|---|---|---|---|---|
| dev | webkit | 1000 → 1000 rows in 0 frames, 6 ms | yes | no |
| dev | firefox | 1000 → 1000 rows in 0 frames, 32 ms | yes | yes |
| dev | chromium | 1000 → 1000 rows in 0 frames, 9 ms | yes | yes |
| build | webkit | 1000 → 1000 rows in 0 frames, 5 ms | yes | no |
| build | firefox | 1000 → 1000 rows in 0 frames, 7 ms | yes | yes |
| build | chromium | 1000 → 1000 rows in 0 frames, 4 ms | yes | yes |

## Failures after retry (0)

none

## Retried cells (0)

none

## Routes not loaded

- biglist: /403 (scaffold route, not part of the probe), * (scaffold route, not part of the probe), /about (scaffold route, not part of the probe), /account (scaffold route, not part of the probe), /login (scaffold route, not part of the probe), /posts/:id (scaffold route, not part of the probe), /posts (scaffold route, not part of the probe)
