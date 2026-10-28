# Seven-stack interactive dashboard benchmark

This is a separate experiment from the original three-stack report. It measures a small working dashboard, not the speed of every application or a replacement for framework feature comparisons.

The frozen BRAINLLM source is commit `7889787`; its release binary SHA-256 is `29d7693f5a6fa93713d6e4691bdfde7e8a59a70ffcd0f701a7acccf3b25498c2`. See [the measured report](results/expanded-20261028/summary.md), [machine-readable summary](results/expanded-20261028/summary.json), raw samples, dependency versions and browser observations in that directory. Later compiler changes are not represented.

## Workload and parity

All seven applications implement the existing user dashboard: initial users, search and empty state, create/edit form with required fields, update, cancel and delete. They use the same data and simulated API delay. The original `app/`, React and Svelte fixtures are preserved. Vue and Angular are native ports; Astro hydrates the same React dashboard with `client:load`; webpack bundles the same React source.

The Chrome harness verifies nine interaction assertions per application, and rejects uncaught runtime exceptions. This is behavioral smoke coverage, not screenshot equivalence, exhaustive accessibility coverage or proof of full framework equivalence.

BRAINLLM serves its actual production SSR output using its frozen binary. Other references use temporary static HTTP servers. Astro includes a hydrated React island; this is not a comparison against a zero-JavaScript Astro page. Angular uses its production application builder. webpack uses production JavaScript optimization and extracted CSS, without an additional CSS minifier. These differences are part of the recorded configurations.

## Measurements and limits

- Each candidate runs sequentially: one sanity build, two warmups and 20 timed production builds. Each timed build removes `dist`; other caches remain warm.
- Commands execute the native binary or the installed Node CLI directly. Dependency installation, Rust compilation and npm/npx wrapper startup are excluded.
- Timings include process startup and output generation. They are **warm build wall-clock times**, not incremental edit latency, cold builds or browser rendering speed.
- Initial-route payload is the sum of successful same-origin Document, Stylesheet and Script response bodies observed during initial loading in Chrome, each gzipped separately at level 9. It excludes HTTP headers, images, failed requests, later navigation assets and service-worker precache traffic. It is a reproducible transfer-size estimate, not a measured network speed.
- The machine was shared, not idle or thermally controlled. Candidates ran in fixed order, not randomized. Load averages, every sample, standard deviation and extrema are published. Do not turn a single median ratio into a universal speed claim.
- This experiment uses Node 26.12.0 and direct CLI execution. Do not combine these timings with the historical report, which used a different environment and launcher policy.
- Dependencies belong only to benchmark fixtures, not the BRAINLLM compiler. Lockfiles pin public npm packages.

## Reproduce

Use Node 26, npm, Rust and Chrome on macOS. Inspect both scripts and fixture configuration first. `BENCH_WORK` must be a disposable directory you own; `BENCH_OUT` must be a new result directory. These scripts do not upload anything.

```sh
# From a clean checkout of the source revision to measure:
CARGO_BUILD_JOBS=2 cargo build --release -p brain-cli
export BRAIN_BIN="$PWD/target/release/brain"
export BRAIN_SOURCE_COMMIT="$(git rev-parse HEAD)"
export BENCH_WORK="$(mktemp -d)"
export BENCH_OUT="$PWD/bench/build/results/my-expanded-run"
BENCH_SETUP_ONLY=1 node bench/build/expanded.mjs
node bench/build/expanded-browser.mjs
node bench/build/expanded.mjs
node bench/build/expanded-browser.mjs
```

Keep the same frozen binary and fixture copies throughout. To change either, start with a new scratch directory. `RUNS` defaults to 20. `BENCH_ONLY` supports a comma-separated subset for diagnosis; subset runs overwrite corresponding output files and are not a complete comparison. Preserve the seven-candidate final browser report.

Builds and installs have ten-minute deadlines and capped private scratch logs. Browser sessions use a private temporary profile, close owned tabs, stop owned servers and remove the profile. Builds run one at a time. Remove only your own scratch directory after collecting the public results. The timing runner bounds timeout and signal cleanup to its owned process groups.

See the official configuration references for [Angular builds](https://angular.dev/reference/configs/workspace-config), [Astro React islands](https://docs.astro.build/en/guides/integrations-guide/react/), [webpack](https://webpack.js.org/guides/getting-started/) and [Vite](https://vite.dev/guide/).
