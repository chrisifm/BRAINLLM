// Rechecks the Phase 0 generations with the real checker (`brain check --json <file>`).
// Usage: node bench/phase0/recheck.mjs [--brain target/debug/brain]  → results/recheck.json + table on stdout
import { execFileSync } from 'node:child_process';
import { readdirSync, existsSync, writeFileSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, '../..');
const i = process.argv.indexOf('--brain');
const brain = i > 0 ? process.argv[i + 1] : join(root, 'target/debug/brain');
const res = join(here, 'results');
const sets = ['gen', 'gen-t2', 'qwen-gen'].filter((d) => existsSync(join(res, d)));

const check = (file) => {
  try {
    return JSON.parse(execFileSync(brain, ['check', '--json', file], { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] }) || '[]');
  } catch (e) {
    const out = (e.stdout || '').trim();
    try { return JSON.parse(out); } catch { return [{ code: 'CRASH', msg: (e.stderr || out).slice(0, 200) }]; }
  }
};

const rows = [];
for (const set of sets) {
  for (const variant of readdirSync(join(res, set))) {
    const dir = join(res, set, variant);
    const files = readdirSync(dir).filter((f) => f.endsWith('.brain')).sort();
    const per = files.map((f) => ({ file: f, diags: check(join(dir, f)) }));
    const isA11y = (d) => /^B04|^B05/.test(d.code);
    const codes = {};
    for (const p of per) for (const d of p.diags) codes[d.code] = (codes[d.code] || 0) + 1;
    rows.push({
      set, variant, n: files.length,
      clean: per.filter((p) => p.diags.length === 0).length,
      noSyntaxErr: per.filter((p) => p.diags.every(isA11y)).length,
      codes, per,
    });
  }
}
writeFileSync(join(res, 'recheck.json'), JSON.stringify({ brain, date: new Date().toISOString(), rows }, null, 1));
console.log('| set | variant | n | no diagnostics | no language errors (a11y/SEO only) | codes |');
console.log('|---|---|---:|---:|---:|---|');
for (const r of rows) console.log(`| ${r.set} | ${r.variant} | ${r.n} | ${r.clean}/${r.n} | ${r.noSyntaxErr}/${r.n} | ${Object.entries(r.codes).map(([k, v]) => `${k}×${v}`).join(' ')} |`);
