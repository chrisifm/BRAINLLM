# app

BRAINLLM app. Rules: `brain spec` · check: `brain check --json` · development: `brain run` · production: `brain build`.

| Path | What goes there |
|---|---|
| `app.brain` | root layout: `<head>`, site `<meta seo>`, nav (every view is linked from it) |
| `views/` | routes (`about.brain` → `/about`, `404.brain`, `_offline.brain`); may export `load` |
| `comp/` · `ui/` | stateful components (with a colocated `.test.brain`) · stateless micro components |
| `channels/` `services/` `api/` `lib/` `server/` `content/` | see each README and its `brain add` command |
| `public/` | files served as is (icons, og.png); referenced ones get a content hash at build |
| `tokens.cssbrain` | design tokens: a `var(--x)` that does not exist is a compile error |
| `brain.json` | name, site URL, cache, SEO minimum score, budgets |
| `.brain_env` | variables per environment (never committed; `.brain_env.example` lists the keys) |

## Phases
- [ ] Describe the app in one paragraph (replace this list with your own phases)
- [ ] Views and navigation
- [ ] Data and server functions
- [ ] Tests and `brain doctor` exit 0
- [ ] `brain build` and deploy
